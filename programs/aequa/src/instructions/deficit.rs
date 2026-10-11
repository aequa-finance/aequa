//! The third line of the backstop: once a shortfall has held through `deficit_hold`
//! distributions, `cover_deficit` first burns whatever the Reserve Fund holds against it (fees
//! since the last distribution, or anything sent there), then moves AEQUA from the stake pool
//! to the slash-sale vault for the rest, at most `slash_cap` of the pool in any rolling 30
//! days. The guardian can pause it but never trigger it.

use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Burn, Mint as UsdqMint, Token, TokenAccount},
    token_interface::{self, Mint, TokenAccount as AAccount, TokenInterface, TransferChecked},
};

use crate::constants::{seeds::*, BPS, SLASH_WINDOW_SECS};
use crate::errors::AequaError;
use crate::events::DeficitCovered;
use crate::instructions::pyth_pro::*;
use crate::math::U256;
use crate::state::{path, AequaPool, Config, Reserve, Twap};
use crate::signer;

/// The TWAP window the buyback and the slash sale price against (30 minutes).
pub const TWAP_WINDOW_SECS: i64 = 30 * 60;

/// AEQUA's dollar price at 1e12 (dollars per whole AEQUA): the pool's TWAP, converted with the
/// quote asset's Pyth price unless the quote is a dollar.
pub fn aequa_price_usd<'info>(config: &Config, twap: &Twap, pyth: &PythPro<'info>, payer: &AccountInfo<'info>, message: &[u8], ed25519_index: u16, now: i64) -> Result<u128> {
    let avg = twap.average(now, TWAP_WINDOW_SECS)?;
    let feed = config.aequa.quote_feed;
    if feed == 0 {
        return Ok(avg);
    }
    let update = pyth.verify(payer, message, ed25519_index)?;
    update.require_fresh(now, config.params.pyth_freshness_secs)?;
    let quote_e8 = update.any_price(feed)?;
    Ok(avg * quote_e8 as u128 / 100_000_000)
}

/// AEQUA base units worth `usd` (6 decimals) at `price_usd_e12`, rounded up.
pub fn aequa_for_usd(usd: u64, price_usd_e12: u128, decimals: u8) -> Result<u64> {
    require!(price_usd_e12 > 0, AequaError::TwapNotReady);
    let n = U256::from(usd) * U256::from(10u64).pow(U256::from(decimals as u32)) * U256::from(1_000_000_000_000u64);
    let d = U256::from(price_usd_e12) * U256::from(1_000_000u64);
    let v = (n + d - U256::from(1u8)) / d;
    require!(v <= U256::from(u64::MAX), AequaError::MathOverflow);
    Ok(v.as_u64())
}

#[derive(Accounts)]
pub struct CoverDeficit<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [RESERVE], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
    #[account(mut, seeds = [AEQUA_POOL], bump = aequa_pool.bump)]
    pub aequa_pool: Box<Account<'info, AequaPool>>,
    #[account(mut)]
    pub usdq_mint: Box<Account<'info, UsdqMint>>,
    #[account(seeds = [TWAP], bump = twap.bump)]
    pub twap: Box<Account<'info, Twap>>,
    #[account(address = config.aequa.mint @ AequaError::AequaNotSet)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, seeds = [AEQUA_POOL_VAULT], bump = config.bumps.aequa_pool_vault)]
    pub aequa_pool_vault: Box<InterfaceAccount<'info, AAccount>>,
    #[account(mut, seeds = [SLASH_SALE], bump = config.bumps.slash_sale)]
    pub slash_sale: Box<InterfaceAccount<'info, AAccount>>,
    #[account(mut, seeds = [RESERVE_FUND], bump = config.bumps.reserve_fund)]
    pub reserve_fund: Box<Account<'info, TokenAccount>>,
    #[account(address = config.aequa.token_program)]
    pub token_program: Interface<'info, TokenInterface>,
    pub usdq_token_program: Program<'info, Token>,
    pub pyth: PythPro<'info>,
}

impl CoverDeficit<'_> {
    /// `message` carries the quote asset's price when the pool's quote isn't a dollar; it may
    /// be empty otherwise.
    pub fn run(&mut self, message: &[u8], ed25519_index: u16) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        let p = config.params;
        config.require_open(path::COVER_DEFICIT)?;
        require!(config.aequa.is_set(), AequaError::AequaNotSet);
        let reserve = &self.reserve;
        require!(reserve.deficit > 0 && reserve.deficit_streak >= p.deficit_hold, AequaError::DeficitNotHeld);
        require!(reserve.last_cover < reserve.last_distribution, AequaError::NotDue);

        // The Reserve Fund's USDq burns against the shortfall before any stake is touched.
        let fund_burned = self.reserve_fund.amount.min(reserve.deficit);
        if fund_burned > 0 {
            let bump = config.bumps.reserve_fund;
            token::burn(
                CpiContext::new_with_signer(
                    self.usdq_token_program.key(),
                    Burn { mint: self.usdq_mint.to_account_info(), from: self.reserve_fund.to_account_info(), authority: self.reserve_fund.to_account_info() },
                    signer!(RESERVE_FUND, bump),
                ),
                fund_burned,
            )?;
            let reserve = &mut self.reserve;
            reserve.deficit -= fund_burned;
            reserve.deficit_burned_total = reserve.deficit_burned_total.saturating_add(fund_burned);
        }
        let deficit = self.reserve.deficit;
        if deficit == 0 {
            self.reserve.last_cover = now;
            emit!(DeficitCovered { fund_burned, aequa: 0, deficit: 0 });
            return Ok(());
        }

        let config = &self.config;
        let price = aequa_price_usd(config, &self.twap, &self.pyth, &self.caller.to_account_info(), message, ed25519_index, now)?;
        // Enough AEQUA to raise the deficit even at the sale's lowest allowed fill.
        let needed = aequa_for_usd(deficit, price, self.mint.decimals)?;
        let needed = ((needed as u128) * BPS as u128 / (BPS - p.slash_twap_bound_bps as u64) as u128) as u64;
        let pending = self.slash_sale.amount;

        let pool = &mut self.aequa_pool;
        pool.fold(now);
        let recent = pool.slashed_since(now, SLASH_WINDOW_SECS);
        let allowance = ((pool.base as u128 + recent as u128) * p.slash_cap_bps as u128 / BPS as u128) as u64;
        let allowance = allowance.saturating_sub(recent);
        require!(allowance > 0, AequaError::SlashCapReached);
        let amount = needed.saturating_sub(pending).min(allowance).min(pool.base);
        require!(amount > 0, AequaError::NothingToMove);

        pool.base -= amount;
        pool.slashed_total = pool.slashed_total.saturating_add(amount);
        pool.record_slash(now, amount);

        let bump = config.bumps.aequa_pool_vault;
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.key(),
                TransferChecked {
                    from: self.aequa_pool_vault.to_account_info(),
                    mint: self.mint.to_account_info(),
                    to: self.slash_sale.to_account_info(),
                    authority: self.aequa_pool_vault.to_account_info(),
                },
                signer!(AEQUA_POOL_VAULT, bump),
            ),
            amount,
            self.mint.decimals,
        )?;
        self.reserve.last_cover = now;
        emit!(DeficitCovered { fund_burned, aequa: amount, deficit });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::aequa_for_usd;

    #[test]
    fn aequa_for_dollars_rounds_up() {
        // $1,000 at $0.00005 per AEQUA (6 dp): 20,000,000 AEQUA.
        assert_eq!(aequa_for_usd(1_000_000_000, 50_000_000, 6).unwrap(), 20_000_000_000_000);
        assert_eq!(aequa_for_usd(1, 3_000_000_000_000, 6).unwrap(), 1);
    }
}
