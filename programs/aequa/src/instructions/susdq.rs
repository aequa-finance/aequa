//! sUSDq: shares over staked USDq whose rate rises as each distribution vests;
//! unstaking locks USDq for the cooldown, then it is claimed.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, MintTo, Token, TokenAccount, TransferChecked};

use crate::constants::seeds::*;
use crate::errors::AequaError;
use crate::math;
use crate::signer;
use crate::state::{path, Config, UsdqCooldown, UsdqVault};

#[derive(Accounts)]
pub struct StakeUsdq<'info> {
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint, has_one = susdq_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [SUSDQ_VAULT], bump = usdq_vault.bump)]
    pub usdq_vault: Box<Account<'info, UsdqVault>>,
    pub usdq_mint: Box<Account<'info, Mint>>,
    #[account(mut)]
    pub susdq_mint: Box<Account<'info, Mint>>,
    #[account(mut, token::mint = usdq_mint, token::authority = user)]
    pub user_usdq: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = susdq_mint)]
    pub user_susdq: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [SUSDQ_ASSETS], bump = config.bumps.susdq_assets)]
    pub susdq_assets: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl StakeUsdq<'_> {
    pub fn run(&mut self, amount: u64, min_shares: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        self.config.require_open(path::STAKE_USDQ)?;
        require!(amount > 0, AequaError::ZeroAmount);
        require!(self.user_usdq.amount >= amount, AequaError::InsufficientFunds);
        let vault = &mut self.usdq_vault;
        let shares = math::shares_for(amount, vault.total_assets(now), self.susdq_mint.supply);
        require!(shares > 0, AequaError::ZeroAmount);
        require!(shares >= min_shares, AequaError::SlippageExceeded);
        vault.base = vault.base.checked_add(amount).ok_or(AequaError::MathOverflow)?;

        let program = self.token_program.key();
        token::transfer_checked(
            CpiContext::new(
                program,
                TransferChecked {
                    from: self.user_usdq.to_account_info(),
                    mint: self.usdq_mint.to_account_info(),
                    to: self.susdq_assets.to_account_info(),
                    authority: self.user.to_account_info(),
                },
            ),
            amount,
            6,
        )?;
        let bump = vault.bump;
        token::mint_to(
            CpiContext::new_with_signer(
                program,
                MintTo { mint: self.susdq_mint.to_account_info(), to: self.user_susdq.to_account_info(), authority: vault.to_account_info() },
                signer!(SUSDQ_VAULT, bump),
            ),
            shares,
        )?;
        Ok(())
    }
}

/// Burns shares and locks their USDq for the cooldown. A second unstake during a cooldown adds
/// to the locked amount and restarts the cooldown; the app claims an opened one first.
#[derive(Accounts)]
pub struct StartUsdqCooldown<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint, has_one = susdq_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [SUSDQ_VAULT], bump = usdq_vault.bump)]
    pub usdq_vault: Box<Account<'info, UsdqVault>>,
    pub usdq_mint: Box<Account<'info, Mint>>,
    #[account(mut)]
    pub susdq_mint: Box<Account<'info, Mint>>,
    #[account(mut, token::mint = susdq_mint, token::authority = user)]
    pub user_susdq: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [SUSDQ_ASSETS], bump = config.bumps.susdq_assets)]
    pub susdq_assets: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [SUSDQ_SILO], bump = config.bumps.susdq_silo)]
    pub susdq_silo: Box<Account<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + UsdqCooldown::INIT_SPACE,
        seeds = [SUSDQ_COOLDOWN, user.key().as_ref()],
        bump,
    )]
    pub cooldown: Box<Account<'info, UsdqCooldown>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl StartUsdqCooldown<'_> {
    pub fn run(&mut self, shares: u64, bump: u8) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        config.require_open(path::UNSTAKE_USDQ)?;
        require!(shares > 0, AequaError::ZeroAmount);
        require!(self.user_susdq.amount >= shares, AequaError::InsufficientFunds);

        let vault = &mut self.usdq_vault;
        vault.fold(now);
        let usdq = math::assets_for(shares, vault.base, self.susdq_mint.supply);
        require!(usdq > 0, AequaError::ZeroAmount);
        vault.base -= usdq;

        let program = self.token_program.key();
        token::burn(
            CpiContext::new(
                program,
                Burn { mint: self.susdq_mint.to_account_info(), from: self.user_susdq.to_account_info(), authority: self.user.to_account_info() },
            ),
            shares,
        )?;
        let assets_bump = config.bumps.susdq_assets;
        token::transfer_checked(
            CpiContext::new_with_signer(
                program,
                TransferChecked {
                    from: self.susdq_assets.to_account_info(),
                    mint: self.usdq_mint.to_account_info(),
                    to: self.susdq_silo.to_account_info(),
                    authority: self.susdq_assets.to_account_info(),
                },
                signer!(SUSDQ_ASSETS, assets_bump),
            ),
            usdq,
            6,
        )?;

        let cooldown = &mut self.cooldown;
        cooldown.owner = self.user.key();
        cooldown.bump = bump;
        cooldown.amount = cooldown.amount.checked_add(usdq).ok_or(AequaError::MathOverflow)?;
        cooldown.unlock_at = now + config.params.susdq_cooldown_secs as i64;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct ClaimUsdq<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint)]
    pub config: Box<Account<'info, Config>>,
    pub usdq_mint: Box<Account<'info, Mint>>,
    #[account(mut, token::mint = usdq_mint)]
    pub user_usdq: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [SUSDQ_SILO], bump = config.bumps.susdq_silo)]
    pub susdq_silo: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        close = user,
        seeds = [SUSDQ_COOLDOWN, user.key().as_ref()],
        bump = cooldown.bump,
    )]
    pub cooldown: Box<Account<'info, UsdqCooldown>>,
    pub token_program: Program<'info, Token>,
}

impl ClaimUsdq<'_> {
    pub fn run(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        config.require_open(path::UNSTAKE_USDQ)?;
        let amount = self.cooldown.amount;
        require!(amount > 0, AequaError::NothingToClaim);
        require!(now >= self.cooldown.unlock_at, AequaError::CooldownActive);
        let bump = config.bumps.susdq_silo;
        token::transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.key(),
                TransferChecked {
                    from: self.susdq_silo.to_account_info(),
                    mint: self.usdq_mint.to_account_info(),
                    to: self.user_usdq.to_account_info(),
                    authority: self.susdq_silo.to_account_info(),
                },
                signer!(SUSDQ_SILO, bump),
            ),
            amount,
            6,
        )?;
        Ok(())
    }
}
