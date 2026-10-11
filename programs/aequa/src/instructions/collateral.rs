//! The xStock paths: mint USDq against SPYx or QQQx at the Pyth price less
//! the session band, and redeem USDq for them at the price plus the band. Both need the
//! collateral live (a fresh price in an open session), the hedge live (a fresh report with the
//! perp in session), no corporate-action window and the issuer's mint unpaused.

use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Burn, Mint, MintTo, Token, TokenAccount, Transfer},
    token_interface::{self, TokenAccount as XAccount, TokenInterface, TransferChecked},
};

use crate::constants::seeds::*;
use crate::errors::AequaError;
use crate::instructions::common::{fee_bps, fee_to_reserve_fund};
use crate::math;
use crate::instructions::pyth_pro::*;
use crate::pyth::{self, Price};
use crate::state::{path, AequaStake, Collateral, Config, Reserve};
use crate::token2022::{read_mint, MintView};

/// Everything an xStock path checks before it prices anything.
fn open_path(config: &Config, reserve: &Reserve, collateral: &Collateral, mint: &AccountInfo, update: &pyth::Update, now: i64) -> Result<(MintView, Price)> {
    let p = &config.params;
    let view = read_mint(mint, now, p.ca_window_before_secs, p.ca_window_after_secs)?;
    view.require_open()?;
    require!(reserve.hedge_live(collateral.index, now, p.report_freshness_secs), AequaError::HedgeNotLive);
    update.require_fresh(now, p.pyth_freshness_secs)?;
    let price = update.price(collateral.feed, p.pyth_freshness_secs)?;
    Ok((view, price))
}

#[derive(Accounts)]
pub struct MintWithCollateral<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(seeds = [RESERVE], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
    #[account(mut, seeds = [COLLATERAL, mint.key().as_ref()], bump = collateral.bump, has_one = mint, has_one = vault)]
    pub collateral: Box<Account<'info, Collateral>>,
    /// CHECK: the listed mint (the collateral's `has_one`); `read_mint` reads it raw.
    pub mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, XAccount>>,
    /// The user's xStock account; the token program checks the mint and the authority.
    #[account(mut)]
    pub user_xstock: Box<InterfaceAccount<'info, XAccount>>,
    #[account(mut)]
    pub usdq_mint: Box<Account<'info, Mint>>,
    #[account(mut, token::mint = usdq_mint)]
    pub user_usdq: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [RESERVE_FUND], bump = config.bumps.reserve_fund)]
    pub reserve_fund: Box<Account<'info, TokenAccount>>,
    /// CHECK: the protocol vault, by its seeds; only ever a destination, which the token
    /// program checks.
    #[account(mut, seeds = [PROTOCOL], bump = config.bumps.protocol)]
    pub protocol: UncheckedAccount<'info>,
    #[account(seeds = [AEQUA_STAKE, user.key().as_ref()], bump = aequa_stake.bump)]
    pub aequa_stake: Option<Box<Account<'info, AequaStake>>>,
    /// CHECK: the stAEQUA pool, by its seeds; the fee tier reads its head (`PoolHead`).
    #[account(seeds = [AEQUA_POOL], bump = config.bumps.aequa_pool)]
    pub aequa_pool: Option<UncheckedAccount<'info>>,
    pub token_program: Program<'info, Token>,
    #[account(address = collateral.token_program)]
    pub xstock_token_program: Interface<'info, TokenInterface>,
    pub pyth: PythPro<'info>,
}

impl MintWithCollateral<'_> {
    pub fn run(&mut self, message: &[u8], ed25519_index: u16, amount: u64, min_out: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        config.require_open(path::MINT_COLLATERAL)?;
        require!(!self.collateral.paused, AequaError::Paused);
        require!(amount > 0, AequaError::ZeroAmount);
        require!(self.user_xstock.amount >= amount, AequaError::InsufficientFunds);

        let update = self.pyth.verify(&self.user.to_account_info(), message, ed25519_index)?;
        let (view, price) = open_path(config, &self.reserve, &self.collateral, &self.mint.to_account_info(), &update, now)?;
        let c = &mut self.collateral;
        let band = c.bands_bps[price.session as usize];
        let value = math::xstock_to_usd(amount, view.multiplier_e12, price.price_e8, band, view.decimals)?;
        require!(value > 0, AequaError::ZeroAmount);

        let supply = self.usdq_mint.supply.checked_add(value).ok_or(AequaError::MathOverflow)?;
        require!(supply <= config.supply_cap(), AequaError::SupplyCapExceeded);
        let spot = math::xstock_value(self.vault.amount, view.multiplier_e12, price.price_e8, view.decimals)?;
        require!(spot.saturating_add(value) <= c.spot_cap, AequaError::SpotCapExceeded);
        let cap = c.hourly_cap;
        c.mint_bucket.draw(value, cap, now)?;

        let fee_bps = fee_bps(config, self.aequa_stake.as_ref().map(|a| &***a), self.aequa_pool.as_ref().map(|a| a.as_ref()), &self.user.key(), now)?;
        let (out, fee) = math::after_fee(value, fee_bps);
        require!(out > 0, AequaError::ZeroAmount);
        require!(out >= min_out, AequaError::SlippageExceeded);
        let to_fund = fee_to_reserve_fund(config, self.reserve_fund.amount, supply);

        token_interface::transfer_checked(
            CpiContext::new(
                self.xstock_token_program.key(),
                TransferChecked {
                    from: self.user_xstock.to_account_info(),
                    mint: self.mint.to_account_info(),
                    to: self.vault.to_account_info(),
                    authority: self.user.to_account_info(),
                },
            ),
            amount,
            view.decimals,
        )?;
        let bump = [config.bumps.config];
        let seeds: &[&[&[u8]]] = &[&[CONFIG, &bump]];
        let program = self.token_program.key();
        token::mint_to(
            CpiContext::new_with_signer(
                program,
                MintTo { mint: self.usdq_mint.to_account_info(), to: self.user_usdq.to_account_info(), authority: config.to_account_info() },
                seeds,
            ),
            out,
        )?;
        if fee > 0 {
            let to = if to_fund { self.reserve_fund.to_account_info() } else { self.protocol.to_account_info() };
            token::mint_to(
                CpiContext::new_with_signer(program, MintTo { mint: self.usdq_mint.to_account_info(), to, authority: config.to_account_info() }, seeds),
                fee,
            )?;
        }
        Ok(())
    }
}

#[derive(Accounts)]
pub struct RedeemForCollateral<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(seeds = [RESERVE], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
    #[account(mut, seeds = [COLLATERAL, mint.key().as_ref()], bump = collateral.bump, has_one = mint, has_one = vault)]
    pub collateral: Box<Account<'info, Collateral>>,
    /// CHECK: the listed mint (the collateral's `has_one`); `read_mint` reads it raw.
    pub mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, XAccount>>,
    /// CHECK: where the xStock goes; the token program checks it is an account of the mint.
    #[account(mut)]
    pub user_xstock: UncheckedAccount<'info>,
    #[account(mut)]
    pub usdq_mint: Box<Account<'info, Mint>>,
    #[account(mut, token::mint = usdq_mint, token::authority = user)]
    pub user_usdq: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [RESERVE_FUND], bump = config.bumps.reserve_fund)]
    pub reserve_fund: Box<Account<'info, TokenAccount>>,
    /// CHECK: the protocol vault, by its seeds; only ever a destination, which the token
    /// program checks.
    #[account(mut, seeds = [PROTOCOL], bump = config.bumps.protocol)]
    pub protocol: UncheckedAccount<'info>,
    #[account(seeds = [AEQUA_STAKE, user.key().as_ref()], bump = aequa_stake.bump)]
    pub aequa_stake: Option<Box<Account<'info, AequaStake>>>,
    /// CHECK: the stAEQUA pool, by its seeds; the fee tier reads its head (`PoolHead`).
    #[account(seeds = [AEQUA_POOL], bump = config.bumps.aequa_pool)]
    pub aequa_pool: Option<UncheckedAccount<'info>>,
    pub token_program: Program<'info, Token>,
    #[account(address = collateral.token_program)]
    pub xstock_token_program: Interface<'info, TokenInterface>,
    pub pyth: PythPro<'info>,
}

impl RedeemForCollateral<'_> {
    pub fn run(&mut self, message: &[u8], ed25519_index: u16, amount: u64, min_out: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        config.require_open(path::REDEEM_COLLATERAL)?;
        require!(!self.collateral.paused, AequaError::Paused);
        require!(amount > 0, AequaError::ZeroAmount);
        require!(self.user_usdq.amount >= amount, AequaError::InsufficientFunds);

        let update = self.pyth.verify(&self.user.to_account_info(), message, ed25519_index)?;
        let (view, price) = open_path(config, &self.reserve, &self.collateral, &self.mint.to_account_info(), &update, now)?;
        let c = &mut self.collateral;
        let band = c.bands_bps[price.session as usize];

        let fee_bps = fee_bps(config, self.aequa_stake.as_ref().map(|a| &***a), self.aequa_pool.as_ref().map(|a| a.as_ref()), &self.user.key(), now)?;
        let (net, fee) = math::after_fee(amount, fee_bps);
        let raw = math::usd_to_xstock(net, view.multiplier_e12, price.price_e8, band, view.decimals)?;
        require!(raw > 0, AequaError::ZeroAmount);
        require!(raw <= self.vault.amount, AequaError::VaultInsufficient);
        require!(raw >= min_out, AequaError::SlippageExceeded);
        let cap = c.hourly_cap;
        c.redeem_bucket.draw(amount, cap, now)?;
        let supply = self.usdq_mint.supply - net;
        let to_fund = fee_to_reserve_fund(config, self.reserve_fund.amount, supply);

        let program = self.token_program.key();
        token::burn(
            CpiContext::new(
                program,
                Burn { mint: self.usdq_mint.to_account_info(), from: self.user_usdq.to_account_info(), authority: self.user.to_account_info() },
            ),
            net,
        )?;
        if fee > 0 {
            let to = if to_fund { self.reserve_fund.to_account_info() } else { self.protocol.to_account_info() };
            token::transfer(
                CpiContext::new(program, Transfer { from: self.user_usdq.to_account_info(), to, authority: self.user.to_account_info() }),
                fee,
            )?;
        }
        let mint_key = self.mint.key();
        let vault_bump = [c.vault_bump];
        let seeds: &[&[&[u8]]] = &[&[COLLATERAL_VAULT, mint_key.as_ref(), &vault_bump]];
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                self.xstock_token_program.key(),
                TransferChecked {
                    from: self.vault.to_account_info(),
                    mint: self.mint.to_account_info(),
                    to: self.user_xstock.to_account_info(),
                    authority: self.vault.to_account_info(),
                },
                seeds,
            ),
            raw,
            view.decimals,
        )?;
        Ok(())
    }
}
