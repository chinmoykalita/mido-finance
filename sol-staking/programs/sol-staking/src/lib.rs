use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, MintTo, Token, TokenAccount};

// At the top of the file, uncomment:
use mpl_token_metadata::instructions::create_metadata_accounts_v3;
use mpl_token_metadata::state::DataV2;
use anchor_spl::token::{Mint, TokenAccount};
use mpl_token_metadata::ID as TOKEN_METADATA_PROGRAM_ID;

// Add this dependency in Cargo.toml:
// mpl-token-metadata = "1.9.1"
declare_id!("2bkxhcxzEQcMzyL3V5BJV9iGVKMG9ozQCSqWsdAC3h6o");

#[program]
pub mod sol_staking {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        mint_bump: u8,
        withdrawal_limit: u64,
        time_lock: i64,
    ) -> Result<()> {
        let staking_pool = &mut ctx.accounts.staking_pool;
        staking_pool.mint_bump = mint_bump;
        staking_pool.treasury = ctx.accounts.treasury.key();
        staking_pool.admin = ctx.accounts.admin.key();
        staking_pool.upgrade_authority = ctx.accounts.admin.key();
        staking_pool.withdrawal_limit = withdrawal_limit;
        staking_pool.last_withdrawal = 0;
        staking_pool.time_lock = time_lock;

        emit!(InitializeEvent {
            admin: ctx.accounts.admin.key(),
            treasury: ctx.accounts.treasury.key(),
            mint: ctx.accounts.mint.key(),
        });

        Ok(())
    }

    pub fn stake(ctx: Context<Stake>, amount: u64) -> Result<()> {
        let staking_pool = &ctx.accounts.staking_pool;
        let staking_pool_key = staking_pool.key();

        // Define seeds for the treasury PDA
        let treasury_seeds = &[b"treasury", staking_pool_key.as_ref()];
        let (treasury_pda, treasury_bump) =
            Pubkey::find_program_address(treasury_seeds, ctx.program_id);

        // Create a binding for the treasury bump
        let treasury_bump_binding = [treasury_bump];
        let treasury_signer_seeds = &[&[
            b"treasury",
            staking_pool_key.as_ref(),
            &treasury_bump_binding,
        ][..]];

        // Transfer SOL from the user to the treasury using invoke_signed
        let transfer_instruction = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.user.key(),
            &ctx.accounts.treasury.key(),
            amount,
        );

        anchor_lang::solana_program::program::invoke(
            &transfer_instruction,
            &[
                ctx.accounts.user.to_account_info(),
                ctx.accounts.treasury.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Mint mSOL tokens to the user using CPI with signer seeds
        let cpi_accounts = MintTo {
            mint: ctx.accounts.mint.to_account_info(),
            to: ctx.accounts.user_msol_account.to_account_info(),
            authority: ctx.accounts.mint_authority.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();

        // Define signer seeds for mint authority PDA
        let mint_authority_seeds = &[&[
            b"mint_authority",
            staking_pool_key.as_ref(),
            &[staking_pool.mint_bump],
        ][..]];

        // Create a binding for the mint authority bump
        let mint_authority_seeds_binding = [
            b"mint_authority",
            staking_pool_key.as_ref(),
            &[staking_pool.mint_bump],
        ];
        let mint_authority_signer_seeds = &[&mint_authority_seeds_binding[..]];

        let cpi_ctx =
            CpiContext::new_with_signer(cpi_program, cpi_accounts, mint_authority_signer_seeds);
        token::mint_to(cpi_ctx, amount)?;

        emit!(StakeEvent {
            user: ctx.accounts.user.key(),
            amount,
        });

        Ok(())
    }

    pub fn unstake(ctx: Context<Unstake>, amount: u64) -> Result<()> {
        // Check if treasury has enough balance
        let treasury_balance = ctx.accounts.treasury.lamports();
        if treasury_balance < amount {
            return Err(ErrorCode::InsufficientTreasuryBalance.into());
        }

        // Check if user has enough mSOL tokens
        if ctx.accounts.user_msol_account.amount < amount {
            return Err(ErrorCode::InsufficientMsolBalance.into());
        }

        // Burn mSOL tokens from the user
        token::burn(ctx.accounts.into_burn_context(), amount)?;

        let staking_pool = &ctx.accounts.staking_pool;
        let staking_pool_key = staking_pool.key();

        let treasury_seeds = &[b"treasury", staking_pool_key.as_ref()];
        let (_, treasury_bump) = Pubkey::find_program_address(treasury_seeds, ctx.program_id);

        let transfer_instruction = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.treasury.key(),
            &ctx.accounts.user.key(),
            amount,
        );

        // Invoke the transfer instruction with signer seeds
        anchor_lang::solana_program::program::invoke_signed(
            &transfer_instruction,
            &[
                ctx.accounts.treasury.to_account_info(),
                ctx.accounts.user.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[&[
                b"treasury",
                staking_pool_key.as_ref(),
                &[treasury_bump],
            ]],
        )?;

        emit!(UnstakeEvent {
            user: ctx.accounts.user.key(),
            amount,
        });

        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        let staking_pool = &mut ctx.accounts.staking_pool;
        let current_time = Clock::get()?.unix_timestamp;

        // Ensure only the admin can withdraw funds
        if ctx.accounts.admin.key() != staking_pool.admin {
            return Err(ErrorCode::Unauthorized.into());
        }

        // Check time lock
        if current_time - staking_pool.last_withdrawal < staking_pool.time_lock {
            return Err(ErrorCode::WithdrawalTooSoon.into());
        }

        // Check withdrawal limit
        if amount > staking_pool.withdrawal_limit {
            return Err(ErrorCode::WithdrawalLimitExceeded.into());
        }

        // Check if treasury has enough balance
        let treasury_balance = ctx.accounts.treasury.lamports();
        if treasury_balance < amount {
            return Err(ErrorCode::InsufficientTreasuryBalance.into());
        }

        // Transfer SOL from treasury to admin's wallet
        **ctx
            .accounts
            .treasury
            .to_account_info()
            .try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.admin.try_borrow_mut_lamports()? += amount;

        // Update last withdrawal time
        staking_pool.last_withdrawal = current_time;

        emit!(WithdrawEvent {
            admin: ctx.accounts.admin.key(),
            amount,
        });

        Ok(())
    }

    // Uncomment if using Metaplex for metadata
    pub fn create_metadata(
        ctx: Context<CreateMetadata>,
        name: String,
        symbol: String,
        uri: String,
    ) -> Result<()> {
        let metadata_seeds = &[b"metadata", ctx.accounts.mint.key().as_ref()];
        let signer_seeds = &[&metadata_seeds[..]];
    
        let accounts = vec![
            ctx.accounts.metadata.to_account_info(),
            ctx.accounts.mint.to_account_info(),
            ctx.accounts.mint_authority.to_account_info(),
            ctx.accounts.payer.to_account_info(),
            ctx.accounts.update_authority.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ];
    
        let metadata_data = DataV2 {
            name,
            symbol,
            uri,
            seller_fee_basis_points: 0, // Adjust as needed
            creators: None,
            collection: None,
            uses: None,
        };
    
        let ix = create_metadata_accounts_v3(
            TOKEN_METADATA_PROGRAM_ID,  // Correct program ID
            ctx.accounts.metadata.key(), // Metadata account
            ctx.accounts.mint.key(),     // Mint account
            ctx.accounts.mint_authority.key(),
            ctx.accounts.payer.key(),
            ctx.accounts.update_authority.key(),
            metadata_data.name.clone(),
            metadata_data.symbol.clone(),
            metadata_data.uri.clone(),
            None, // Creators
            0,    // Seller fee
            true, // Is Mutable
            true, // Is Primary Sale Happened
            None, // Token Edition
        );
    
        invoke_signed(
            &ix,
            &accounts,
            &[signer_seeds], // Add signer seeds for PDA authority
        )?;
    
        Ok(())
    }


    pub fn change_admin(ctx: Context<ChangeAdmin>, new_admin: Pubkey) -> Result<()> {
        let staking_pool = &mut ctx.accounts.staking_pool;

        // Ensure the new admin is not the zero address
        if new_admin == Pubkey::default() {
            return Err(ErrorCode::InvalidAdminAddress.into());
        }

        // Update the admin
        staking_pool.admin = new_admin;

        emit!(ChangeAdminEvent {
            old_admin: ctx.accounts.admin.key(),
            new_admin,
        });

        Ok(())
    }

    pub fn set_upgrade_authority(
        ctx: Context<SetUpgradeAuthority>,
        new_upgrade_authority: Pubkey,
    ) -> Result<()> {
        let staking_pool = &mut ctx.accounts.staking_pool;
        
        if new_upgrade_authority == Pubkey::default() {
            return Err(ErrorCode::InvalidUpgradeAuthority.into());
        }
    
        // Update the upgrade authority
        staking_pool.upgrade_authority = new_upgrade_authority;
    
        emit!(SetUpgradeAuthorityEvent {
            old_authority: ctx.accounts.current_authority.key(),
            new_authority: new_upgrade_authority,
        });
    
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = admin, space = 8 + 32 + 32 + 8 + 8 + 8 + 8)]
    pub staking_pool: Account<'info, StakingPool>,

    #[account(
        seeds = [b"treasury", staking_pool.key().as_ref()],
        bump,
    )]
    /// CHECK: This is safe because it's a PDA owned by the program
    pub treasury: AccountInfo<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        init,
        payer = admin,
        mint::decimals = 9,
        mint::authority = mint_authority,
    )]
    pub mint: Account<'info, Mint>,

    /// CHECK: This is safe because it's a PDA that will be the mint authority
    #[account(
        seeds = [b"mint_authority", staking_pool.key().as_ref()],
        bump,
    )]
    pub mint_authority: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Stake<'info> {
    #[account(mut)]
    pub staking_pool: Account<'info, StakingPool>,

    #[account(mut)]
    pub user: Signer<'info>,

    #[account(mut)]
    pub user_msol_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"treasury", staking_pool.key().as_ref()],
        bump,
    )]
    /// CHECK: This is safe because it's a PDA owned by the program
    pub treasury: AccountInfo<'info>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    /// CHECK: This is safe because it's a PDA that is the mint authority
    #[account(
        seeds = [b"mint_authority", staking_pool.key().as_ref()],
        bump,
    )]
    pub mint_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Unstake<'info> {
    #[account(mut)]
    pub staking_pool: Account<'info, StakingPool>,

    #[account(mut)]
    pub user: Signer<'info>,

    #[account(mut)]
    pub user_msol_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"treasury", staking_pool.key().as_ref()],
        bump,
    )]
    /// CHECK: This is safe because it's a PDA owned by the program
    pub treasury: AccountInfo<'info>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub staking_pool: Account<'info, StakingPool>,

    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"treasury", staking_pool.key().as_ref()],
        bump,
    )]
    /// CHECK: This is safe because it's a PDA owned by the program
    pub treasury: AccountInfo<'info>,
}

// Uncomment if using Metaplex for metadata
#[derive(Accounts)]
pub struct CreateMetadata<'info> {
    #[account(mut)]
    pub metadata: Signer<'info>,

    #[account(mut)]
    pub mint: Account<'info, Mint>,

    #[account(signer)]
    pub mint_authority: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(signer)]
    pub update_authority: Signer<'info>,

    pub system_program: Program<'info, System>,

    #[account(address = TOKEN_METADATA_PROGRAM_ID)]
    pub token_metadata_program: Program<'info, mpl_token_metadata::ID>,
}

#[derive(Accounts)]
pub struct ChangeAdmin<'info> {
    #[account(mut, has_one = admin @ ErrorCode::Unauthorized)]
    pub staking_pool: Account<'info, StakingPool>,

    pub admin: Signer<'info>,
}

#[account]
pub struct StakingPool {
    pub mint_bump: u8,
    pub treasury: Pubkey,
    pub admin: Pubkey,
    pub upgrade_authority: Pubkey,
    pub withdrawal_limit: u64,
    pub last_withdrawal: i64,
    pub time_lock: i64,
}

#[derive(Accounts)]
pub struct SetUpgradeAuthority<'info> {
    #[account(
        mut,
        constraint = staking_pool.upgrade_authority == current_authority.key() @ ErrorCode::Unauthorized
    )]
    pub staking_pool: Account<'info, StakingPool>,
    
    pub current_authority: Signer<'info>,
}

impl<'info> Stake<'info> {
    fn into_mint_context(&self) -> CpiContext<'_, '_, '_, 'info, MintTo<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            MintTo {
                mint: self.mint.to_account_info(),
                to: self.user_msol_account.to_account_info(),
                authority: self.mint_authority.to_account_info(),
            },
        )
    }
}

impl<'info> Unstake<'info> {
    fn into_burn_context(&self) -> CpiContext<'_, '_, '_, 'info, Burn<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Burn {
                mint: self.mint.to_account_info(),
                from: self.user_msol_account.to_account_info(),
                authority: self.user.to_account_info(),
            },
        )
    }
}

#[error_code]
pub enum ErrorCode {
    #[msg("You are not authorized to perform this action.")]
    Unauthorized,

    #[msg("Insufficient balance in the treasury.")]
    InsufficientTreasuryBalance,

    #[msg("Insufficient mSOL balance for unstaking.")]
    InsufficientMsolBalance,

    #[msg("Withdrawal limit exceeded.")]
    WithdrawalLimitExceeded,

    #[msg("Withdrawal too soon after the last withdrawal.")]
    WithdrawalTooSoon,

    #[msg("Invalid admin address. The admin cannot be set to the zero address.")]
    InvalidAdminAddress,

    #[msg("Invalid upgrade authority address")]
    InvalidUpgradeAuthority,
}

#[event]
pub struct InitializeEvent {
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub mint: Pubkey,
}

#[event]
pub struct StakeEvent {
    pub user: Pubkey,
    pub amount: u64,
}

#[event]
pub struct UnstakeEvent {
    pub user: Pubkey,
    pub amount: u64,
}

#[event]
pub struct WithdrawEvent {
    pub admin: Pubkey,
    pub amount: u64,
}

#[event]
pub struct CreateMetadataEvent {
    pub mint: Pubkey,
    pub metadata: Pubkey,
}

#[event]
pub struct ChangeAdminEvent {
    pub old_admin: Pubkey,
    pub new_admin: Pubkey,
}

#[event]
pub struct SetUpgradeAuthorityEvent {
    pub old_authority: Pubkey,
    pub new_authority: Pubkey,
}