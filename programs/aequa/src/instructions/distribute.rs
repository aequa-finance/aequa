//! `distribute`: the permissionless crank that values the reserve and either
//! mints its surplus (80% to sUSDq, vesting; 10% to the Reserve Fund while it is under target;
//! 10% to the protocol vault), or burns Reserve Fund USDq against a shortfall.
//!
//! NAV = Σ vault × M × P (mid) + buffer + in-flight USDC + hedge account value.
//! Remaining accounts: for each listed collateral in order, its Collateral, mint and vault.

use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Burn, Mint, MintTo, Token, TokenAccount},
    token_interface::TokenAccount as XAccount,
};

use crate::constants::{seeds::*, BPS, DAY};
use crate::errors::AequaError;
use crate::instructions::pyth_pro::*;
use crate::math;
use crate::state::{path, Collateral, Config, Reserve, UsdqVault};
use crate::token2022::read_mint_value;

/// The Pyth update and the hedge report's HyperCore time must be this close (ms), so the spot
/// leg and the short are marked at one moment.
pub const PAIRING_MS: i64 = 20_000;

#[derive(Accounts)]
pub struct Distribute<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint, has_one = susdq_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [RESERVE], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
    #[account(mut, seeds = [SUSDQ_VAULT], bump = usdq_vault.bump)]
    pub usdq_vault: Box<Account<'info, UsdqVault>>,
    #[account(mut)]
    pub usdq_mint: Box<Account<'info, Mint>>,
    pub susdq_mint: Box<Account<'info, Mint>>,
    #[account(mut, seeds = [SUSDQ_ASSETS], bump = config.bumps.susdq_assets)]
    pub susdq_assets: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [RESERVE_FUND], bump = config.bumps.reserve_fund)]
    pub reserve_fund: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [PROTOCOL], bump = config.bumps.protocol)]
    pub protocol: Box<Account<'info, TokenAccount>>,
    #[account(seeds = [BUFFER], bump = config.bumps.buffer)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub pyth: PythPro<'info>,
}

/// The surplus split of one distribution. The Reserve Fund and protocol parts round
/// down; sUSDq keeps the remainder. With no sUSDq outstanding, its part joins the Reserve
/// Fund's; whatever the fund can't take below its target goes to the protocol vault.
pub fn split(amount: u64, fund_share_bps: u16, protocol_share_bps: u16, fund_room: u64, stakers: bool) -> (u64, u64, u64) {
    let fund = ((amount as u128) * fund_share_bps as u128 / BPS as u128) as u64;
    let protocol = ((amount as u128) * protocol_share_bps as u128 / BPS as u128) as u64;
    let susdq = amount - fund - protocol;
    let (susdq, to_fund_wanted) = if stakers { (susdq, fund) } else { (0, fund + susdq) };
    let to_fund = to_fund_wanted.min(fund_room);
    (susdq, to_fund, protocol + (to_fund_wanted - to_fund))
}

impl<'info> Distribute<'info> {
    pub fn run(&mut self, message: &[u8], ed25519_index: u16, remaining: &'info [AccountInfo<'info>]) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        let p = config.params;
        config.require_open(path::DISTRIBUTE)?;
        let reserve = &self.reserve;
        require!(now >= reserve.last_distribution + p.distribution_interval_secs as i64, AequaError::NotDue);
        require!(reserve.report_fresh(now, p.report_freshness_secs), AequaError::HedgeNotLive);

        // Every collateral live, none inside a corporate-action window.
        let update = self.pyth.verify(&self.caller.to_account_info(), message, ed25519_index)?;
        update.require_fresh(now, p.pyth_freshness_secs)?;
        let pyth_ms = (update.timestamp_us / 1_000) as i64;
        require!((pyth_ms - reserve.report.hl_time).abs() <= PAIRING_MS, AequaError::HedgeNotLive);
        let count = config.collateral_count as usize;
        require!(remaining.len() == count * 3, AequaError::CollateralMismatch);
        let mut spot: u128 = 0;
        for i in 0..count {
            let (c_info, mint_info, vault_info) = (&remaining[3 * i], &remaining[3 * i + 1], &remaining[3 * i + 2]);
            let c: Account<Collateral> = Account::try_from(c_info)?;
            require!(c.mint == config.collaterals[i] && c.index as usize == i, AequaError::CollateralMismatch);
            require_keys_eq!(mint_info.key(), c.mint, AequaError::CollateralMismatch);
            require_keys_eq!(vault_info.key(), c.vault, AequaError::CollateralMismatch);
            let vault: InterfaceAccount<XAccount> = InterfaceAccount::try_from(vault_info)?;
            let view = read_mint_value(mint_info, now, p.ca_window_before_secs, p.ca_window_after_secs)?;
            require!(!view.in_ca_window, AequaError::CorporateActionWindow);
            let price = update.price(c.feed, p.pyth_freshness_secs)?;
            spot += math::xstock_value(vault.amount, view.multiplier_e12, price.price_e8, view.decimals)? as u128;
        }

        let nav: i128 = spot as i128 + self.buffer.amount as i128 + reserve.in_flight() + reserve.report.account_value as i128;
        let supply = self.usdq_mint.supply;
        let floor = supply as i128 + (supply as i128 * p.cushion_bps as i128 / BPS as i128);
        let elapsed = now - reserve.last_distribution;

        let (mut amount, mut burned) = (0u64, 0u64);
        let mut deficit = 0u64;
        let mut streak = reserve.deficit_streak;
        let config_bump = [config.bumps.config];
        let config_seeds: &[&[&[u8]]] = &[&[CONFIG, &config_bump]];
        let program = self.token_program.key();

        if nav > floor {
            let surplus = (nav - floor) as u128;
            let cap = (supply as u128) * p.yield_cap_bps_per_day as u128 * elapsed.max(0) as u128 / (BPS as u128 * DAY as u128);
            amount = surplus.min(cap).min(u64::MAX as u128) as u64;
            let target = config.reserve_fund_target(supply.saturating_add(amount));
            let room = target.saturating_sub(self.reserve_fund.amount);
            let (to_susdq, to_fund, to_protocol) = split(amount, p.reserve_fund_share_bps, p.protocol_share_bps, room, self.susdq_mint.supply > 0);
            for (to, part) in [
                (self.susdq_assets.to_account_info(), to_susdq),
                (self.reserve_fund.to_account_info(), to_fund),
                (self.protocol.to_account_info(), to_protocol),
            ] {
                if part > 0 {
                    token::mint_to(
                        CpiContext::new_with_signer(program, MintTo { mint: self.usdq_mint.to_account_info(), to, authority: config.to_account_info() }, config_seeds),
                        part,
                    )?;
                }
            }
            if to_susdq > 0 {
                let vault = &mut self.usdq_vault;
                let done = vault.vesting.restart(now, to_susdq, p.susdq_vesting_secs as i64)?;
                vault.base += done;
            }
            streak = 0;
        } else if nav < supply as i128 {
            // A shortfall: the period's surplus is nothing; the Reserve Fund burns up to it.
            let short = (supply as i128 - nav).min(u64::MAX as i128) as u64;
            burned = short.min(self.reserve_fund.amount);
            if burned > 0 {
                let bump = [config.bumps.reserve_fund];
                token::burn(
                    CpiContext::new_with_signer(
                        program,
                        Burn { mint: self.usdq_mint.to_account_info(), from: self.reserve_fund.to_account_info(), authority: self.reserve_fund.to_account_info() },
                        &[&[RESERVE_FUND, &bump]],
                    ),
                    burned,
                )?;
            }
            deficit = short - burned;
            streak = if deficit > 0 { streak.saturating_add(1) } else { 0 };
        } else {
            streak = 0;
        }

        let reserve = &mut self.reserve;
        reserve.last_distribution = now;
        reserve.last_nav = nav.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        reserve.last_supply = supply;
        reserve.deficit = deficit;
        reserve.deficit_streak = streak;
        reserve.distributions += 1;
        reserve.distributed_total = reserve.distributed_total.saturating_add(amount);
        reserve.deficit_burned_total = reserve.deficit_burned_total.saturating_add(burned);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn split_rounds_parts_down_and_overflows_to_protocol() {
        // 80 / 10 / 10 with a remainder that stays with sUSDq.
        assert_eq!(split(1_000_003, 1_000, 1_000, u64::MAX, true), (800_003, 100_000, 100_000));
        // The fund takes what fits under its target; the rest goes to the protocol vault.
        assert_eq!(split(1_000_000, 1_000, 1_000, 40_000, true), (800_000, 40_000, 160_000));
        // No stakers yet: sUSDq's part joins the fund's.
        assert_eq!(split(1_000_000, 1_000, 1_000, u64::MAX, false), (0, 900_000, 100_000));
        assert_eq!(split(1_000_000, 1_000, 2_000, 0, true), (700_000, 0, 300_000));
    }
}
