//! The buyback's price reference: the designated pool's price, observed by the
//! keeper and averaged over time. Each observation reads the pool's two reserve accounts, so a
//! constant-product pool of any venue works; the average weighs each price by how long it
//! stood, so moving the price for one observation moves the average by only that
//! observation's share of the window. An average needs an observation from the last five
//! minutes and at least ten inside the window, so a stalled observer can't leave an old price
//! standing.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::TokenAccount;

use crate::constants::seeds::*;
use crate::errors::AequaError;
use crate::state::{Config, Twap};

/// Observations closer together than this are refused, so one block can't fill the ring.
pub const MIN_SPACING_SECS: i64 = 30;
/// The newest observation an average accepts is at most this old.
pub const MAX_AGE_SECS: i64 = 300;
/// Observations an average needs inside its window.
pub const MIN_SAMPLES: usize = 10;
/// Prices are quote units per AEQUA, each in whole tokens, at 1e12.
pub const PRICE_ONE: u128 = 1_000_000_000_000;

impl Twap {
    fn ring(&self) -> usize {
        self.at.len()
    }

    fn latest(&self) -> Option<usize> {
        if self.count == 0 {
            None
        } else {
            Some((self.head as usize + self.ring() - 1) % self.ring())
        }
    }

    /// Cumulative price × seconds at `now`, carrying the last price forward.
    fn cumulative_at(&self, now: i64) -> Option<u128> {
        let i = self.latest()?;
        Some(self.cumulative[i] + self.last_price * (now - self.at[i]).max(0) as u128)
    }

    pub fn record(&mut self, now: i64, price: u128) -> Result<()> {
        let cumulative = match self.latest() {
            Some(i) => {
                require!(now - self.at[i] >= MIN_SPACING_SECS, AequaError::NotDue);
                self.cumulative_at(now).unwrap()
            }
            None => 0,
        };
        let h = self.head as usize;
        self.at[h] = now;
        self.cumulative[h] = cumulative;
        self.last_price = price;
        self.head = ((h + 1) % self.ring()) as u8;
        self.count = (self.count as usize + 1).min(self.ring()) as u8;
        Ok(())
    }

    /// The time-weighted average over the last `window` seconds. Needs an observation at or
    /// before the window's start, one from the last `MAX_AGE_SECS`, and `MIN_SAMPLES` inside.
    pub fn average(&self, now: i64, window: i64) -> Result<u128> {
        let from = now - window;
        let n = self.count as usize;
        let latest = self.latest().ok_or(AequaError::TwapNotReady)?;
        require!(now - self.at[latest] <= MAX_AGE_SECS, AequaError::TwapNotReady);
        let inside = (0..n).filter(|&k| self.at[(self.head as usize + self.ring() - 1 - k) % self.ring()] > from).count();
        require!(inside >= MIN_SAMPLES, AequaError::TwapNotReady);
        // The newest observation at or before `from`.
        let mut start = None;
        for k in 0..n {
            let i = (self.head as usize + self.ring() - 1 - k) % self.ring();
            if self.at[i] <= from {
                start = Some(i);
                break;
            }
        }
        let i = start.ok_or(AequaError::TwapNotReady)?;
        let elapsed = (now - self.at[i]).max(1) as u128;
        let end = self.cumulative_at(now).ok_or(AequaError::TwapNotReady)?;
        Ok((end - self.cumulative[i]) / elapsed)
    }
}

/// `quote / aequa` from reserve amounts, as whole tokens, at 1e12.
pub fn pool_price(aequa_amount: u64, aequa_decimals: u8, quote_amount: u64, quote_decimals: u8) -> Result<u128> {
    require!(aequa_amount > 0 && quote_amount > 0, AequaError::TwapNotReady);
    let num = (quote_amount as u128)
        .checked_mul(10u128.pow(aequa_decimals as u32))
        .and_then(|v| v.checked_mul(PRICE_ONE))
        .ok_or(AequaError::MathOverflow)?;
    let den = (aequa_amount as u128).checked_mul(10u128.pow(quote_decimals as u32)).ok_or(AequaError::MathOverflow)?;
    Ok(num / den)
}

#[derive(Accounts)]
pub struct ObservePool<'info> {
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = keeper @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub keeper: Signer<'info>,
    #[account(mut, seeds = [TWAP], bump = twap.bump, constraint = twap.pool == config.aequa.pool @ AequaError::AequaNotSet)]
    pub twap: Box<Account<'info, Twap>>,
    #[account(address = config.aequa.pool_aequa_vault @ AequaError::AequaNotSet)]
    pub pool_aequa_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = config.aequa.pool_quote_vault @ AequaError::AequaNotSet)]
    pub pool_quote_vault: Box<InterfaceAccount<'info, TokenAccount>>,
}

impl ObservePool<'_> {
    pub fn run(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let a = &self.config.aequa;
        require!(a.is_set(), AequaError::AequaNotSet);
        let price = pool_price(self.pool_aequa_vault.amount, a.decimals, self.pool_quote_vault.amount, a.quote_decimals)?;
        self.twap.record(now, price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Twap {
        Twap { pool: Pubkey::default(), at: [0; 64], cumulative: [0; 64], last_price: 0, head: 0, count: 0, bump: 0 }
    }

    /// Observations every minute from `from` to `to` (inclusive) at `price`.
    fn fill(t: &mut Twap, from: i64, to: i64, price: u128) {
        let mut at = from;
        while at <= to {
            t.record(at, price).unwrap();
            at += 60;
        }
    }

    #[test]
    fn average_weighs_by_time() {
        let mut t = empty();
        fill(&mut t, 0, 1_440, 100);
        fill(&mut t, 1_500, 1_800, 200);
        // 1,500 s at 100 and 300 s at 200 over the last 1,800 s.
        assert_eq!(t.average(1_800, 1_800).unwrap(), (100 * 1_500 + 200 * 300) / 1_800);
        assert!(t.record(1_810, 300).is_err(), "observations need 30 s between them");
    }

    #[test]
    fn needs_history_before_the_window() {
        let mut t = empty();
        fill(&mut t, 1_000, 1_500, 100);
        assert!(t.average(1_500, 1_800).is_err());
        fill(&mut t, 1_560, 2_800, 100);
        assert_eq!(t.average(2_800, 1_800).unwrap(), 100);
    }

    #[test]
    fn refuses_a_stale_or_sparse_window() {
        let mut t = empty();
        fill(&mut t, 0, 1_800, 100);
        assert_eq!(t.average(1_800, 1_800).unwrap(), 100);
        // The observer stops: five minutes on, the old price no longer counts.
        assert!(t.average(1_800 + MAX_AGE_SECS + 1, 1_800).is_err());
        // Two observations spanning the window are too few.
        let mut sparse = empty();
        sparse.record(0, 100).unwrap();
        sparse.record(1_700, 100).unwrap();
        assert!(sparse.average(1_800, 1_800).is_err());
    }

    #[test]
    fn ring_wraps() {
        let mut t = empty();
        for k in 0..200 {
            t.record(k * 60, 100 + k as u128).unwrap();
        }
        assert_eq!(t.count, 64);
        let avg = t.average(199 * 60, 1_800).unwrap();
        assert!(avg > 280 && avg < 300, "{avg}");
    }

    #[test]
    fn pool_prices() {
        // 1,000,000 AEQUA (6 dp) against 50 USDC (6 dp): 0.00005 USDC per AEQUA.
        assert_eq!(pool_price(1_000_000_000_000, 6, 50_000_000, 6).unwrap(), 50_000_000);
    }
}
