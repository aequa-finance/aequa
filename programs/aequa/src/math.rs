//! The program's arithmetic. Raw u64 amounts, u128 intermediates, and every
//! amount paid out rounded down.

use crate::constants::{BPS, HOUR};

/// `out = floor(amount × (10000 − fee) / 10000)`; the fee is the rest.
pub fn after_fee(amount: u64, fee_bps: u16) -> (u64, u64) {
    let out = ((amount as u128) * (BPS - fee_bps as u64) as u128 / BPS as u128) as u64;
    (out, amount - out)
}

/// The bucket's level at `now`: `min(cap, available + cap × elapsed / 3600)`.
pub fn bucket_level(available: u64, updated_at: i64, cap: u64, now: i64) -> u64 {
    let elapsed = now.saturating_sub(updated_at).max(0) as u128;
    let level = available as u128 + (cap as u128) * elapsed / HOUR as u128;
    level.min(cap as u128) as u64
}

/// Shares for assets: `floor(assets × shares / total)`, 1:1 for the first.
pub fn shares_for(assets: u64, total: u64, shares: u64) -> u64 {
    if shares == 0 || total == 0 {
        assets
    } else {
        ((assets as u128) * (shares as u128) / (total as u128)) as u64
    }
}

/// Assets for shares: `floor(shares × total / shares_total)`.
pub fn assets_for(shares: u64, total: u64, total_shares: u64) -> u64 {
    if total_shares == 0 {
        0
    } else {
        ((shares as u128) * (total as u128) / (total_shares as u128)) as u64
    }
}

/// Linear vesting of `amount` between `start` and `end`.
pub fn vested(amount: u64, start: i64, end: i64, now: i64) -> u64 {
    if now >= end || end <= start {
        return amount;
    }
    if now <= start {
        return 0;
    }
    ((amount as u128) * ((now - start) as u128) / ((end - start) as u128)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_rounds_down() {
        assert_eq!(after_fee(1_000_000, 10), (999_000, 1_000));
        assert_eq!(after_fee(1, 10), (0, 1));
        assert_eq!(after_fee(9_999, 10), (9_989, 10));
    }

    #[test]
    fn bucket_refills_linearly() {
        assert_eq!(bucket_level(0, 0, 1_000_000, 1_800), 500_000);
        assert_eq!(bucket_level(900_000, 0, 1_000_000, 3_600), 1_000_000);
    }

    #[test]
    fn vesting_is_linear() {
        assert_eq!(vested(800, 100, 900, 500), 400);
        assert_eq!(vested(800, 100, 900, 1_000), 800);
        assert_eq!(vested(800, 100, 900, 50), 0);
    }
}
