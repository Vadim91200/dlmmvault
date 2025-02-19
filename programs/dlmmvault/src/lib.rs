use anchor_lang::prelude::*;
use anchor_lang::system_program;

pub const AGGREGATOR_PROGRAM_ID: Pubkey = pubkey!("AAAA...replace-with-real-aggregator-PK");

declare_id!("VaULT11111111111111111111111111111111111111111");

#[program]
pub mod meteora_sol_vault {
    use super::*;

    /// Creates and initializes the vault.  
    /// Called once by the vault admin.
    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault_account = &mut ctx.accounts.vault_account;
        vault_account.admin = *ctx.accounts.admin.key;
        vault_account.total_shares = 0;
        vault_account.total_sol = 0;
        vault_account.invested_amount = 0;
        vault_account.bump = *ctx.bumps.get("vault_account").unwrap();

        Ok(())
    }

    /// User deposits SOL. We credit them shares in the vault.
    /// The user’s wallet signs and sends lamports.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        // Transfer lamports from user to vault (system_program::transfer).
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.user.to_account_info(),
                to: ctx.accounts.vault_account.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, amount)?;

        // Accounting updates
        let vault_account = &mut ctx.accounts.vault_account;
        let user_account = &mut ctx.accounts.user_account;

        // If vault has no shares yet, 1 deposit lamport = 1 share.
        // Otherwise, pro-rate.
        let shares_to_mint = if vault_account.total_sol == 0 || vault_account.total_shares == 0 {
            amount
        } else {
            let share_price = vault_account.total_sol as f64 / vault_account.total_shares as f64;
            (amount as f64 / share_price) as u64
        };

        vault_account.total_sol += amount;
        vault_account.total_shares += shares_to_mint;
        user_account.shares += shares_to_mint;

        Ok(())
    }

    /// Called by a strategist with data from an external sources that says:
    ///  - which pool address to deposit into
    ///  - how much SOL to swap
    ///  
    /// Call a meteora lib to deposit the amount into the pool 
    pub fn invest(
        ctx: Context<Invest>,
        pool_address: Pubkey,
        sol_to_invest: u64
    ) -> Result<()> {
        let vault_account = &mut ctx.accounts.vault_account;
        require_keys_eq!(vault_account.admin, ctx.accounts.admin.key(), VaultError::Unauthorized);

        // Check the vault has enough SOL
        require!(
            vault_account.total_sol >= sol_to_invest,
            VaultError::InsufficientVaultBalance
        );

        // Decrease vault’s liquid SOL to reflect the portion now going into LP
        vault_account.total_sol -= sol_to_invest;
        vault_account.invested_amount += sol_to_invest;

        // TODO: Construct a CPI call to Meteora program to deposit the `sol_to_invest` into the
        // specified pool_address. 

        Ok(())
    }

    /// Called by the strategist to end the strategy.
    /// The aggregator is told to redeem the LP tokens for SOL.
    /// That SOL is transferred into the vault's account so users can withdraw.
    pub fn finalize_strategy(ctx: Context<FinalizeStrategy>) -> Result<()> {
        let vault_account = &mut ctx.accounts.vault_account;
        require_keys_eq!(vault_account.admin, ctx.accounts.admin.key(), VaultError::Unauthorized);

        // TODO: Construct a CPI call to Meteora program to withdraw the sol invested 
        let sol_received = vault_account.invested_amount;
        vault_account.invested_amount = 0;

        // The vault’s total_sol increases by however much we redeemed from LP
        vault_account.total_sol += sol_received;

        Ok(())
    }

    /// User withdraws the portion of SOL corresponding to some fraction of their shares.
    pub fn withdraw(ctx: Context<Withdraw>, shares_to_withdraw: u64) -> Result<()> {
        let vault_account = &mut ctx.accounts.vault_account;
        let user_account = &mut ctx.accounts.user_account;

        require!(
            user_account.shares >= shares_to_withdraw,
            VaultError::InsufficientUserShares
        );

        // The fraction of total shares they hold:
        // share_price = total_sol / total_shares
        require!(vault_account.total_shares > 0, VaultError::NoVaultShares);

        let share_price = vault_account.total_sol as f64 / vault_account.total_shares as f64;
        let sol_amount = (shares_to_withdraw as f64 * share_price) as u64;

        // Check vault can pay that out.
        require!(
            vault_account.total_sol >= sol_amount,
            VaultError::InsufficientVaultBalance
        );

        // Decrement from vault
        vault_account.total_sol -= sol_amount;
        vault_account.total_shares -= shares_to_withdraw;

        // Remove from user
        user_account.shares -= shares_to_withdraw;

        // Send the SOL back.
        let vault_info = ctx.accounts.vault_account.to_account_info();
        let user_info = ctx.accounts.user.to_account_info();

        **vault_info.try_borrow_mut_lamports()? -= sol_amount;
        **user_info.try_borrow_mut_lamports()? += sol_amount;

        Ok(())
    }
}

// ------------------ Context Structs ------------------ //

#[derive(Accounts)]
#[instruction()]
pub struct InitializeVault<'info> {
    #[account(
        init,
        payer = admin,
        seeds = [b"vault", admin.key().as_ref()],
        bump,
        space = 8 + 200
    )]
    pub vault_account: Account<'info, VaultAccount>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub vault_account: Account<'info, VaultAccount>,
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + 64,
        seeds = [b"user", user.key().as_ref(), vault_account.key().as_ref()],
        bump
    )]
    pub user_account: Account<'info, VaultUser>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Invest<'info> {
    #[account(mut)]
    pub vault_account: Account<'info, VaultAccount>,
    /// The authorized admin/strategist.
    pub admin: Signer<'info>,
    /// The aggregator program we’ll call for deposit (if we had real CPI).  
    /// (Optional: you might store this in your VaultAccount.)
    #[account(address = AGGREGATOR_PROGRAM_ID)]
    pub aggregator_program: Program<'info, ExternalAggregatorProgram>,
}

#[derive(Accounts)]
pub struct FinalizeStrategy<'info> {
    #[account(mut)]
    pub vault_account: Account<'info, VaultAccount>,
    pub admin: Signer<'info>,
    /// The aggregator program used for withdrawing from the pool.
    #[account(address = AGGREGATOR_PROGRAM_ID)]
    pub aggregator_program: Program<'info, ExternalAggregatorProgram>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub vault_account: Account<'info, VaultAccount>,
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        mut,
        seeds = [b"user", user.key().as_ref(), vault_account.key().as_ref()],
        bump
    )]
    pub user_account: Account<'info, VaultUser>,
}

// ------------------ Account Structs ------------------ //

#[account]
pub struct VaultAccount {
    /// The authority that can call invest/finalize_strategy
    pub admin: Pubkey,
    /// Total number of "shares" minted
    pub total_shares: u64,
    /// Total amount of SOL (lamports) currently “liquid” in the vault
    pub total_sol: u64,
    /// The amount of SOL that has been invested (if any).
    pub invested_amount: u64,
    /// Bump for the vault pda
    pub bump: u8,
}

#[account]
pub struct VaultUser {
    /// How many shares this user has
    pub shares: u64,
}

// ------------------ Errors ------------------ //

#[error_code]
pub enum VaultError {
    #[msg("Unauthorized operation.")]
    Unauthorized,
    #[msg("Vault has insufficient SOL for this operation.")]
    InsufficientVaultBalance,
    #[msg("User does not have enough shares.")]
    InsufficientUserShares,
    #[msg("Vault has no shares.")]
    NoVaultShares,
}
