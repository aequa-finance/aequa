//! The program's arithmetic. Raw u64 amounts, 256-bit
//! intermediates where a product can pass u128, and every amount paid out rounded down.

use anchor_lang::prelude::*;

use crate::constants::{BPS, HOUR, MULT_ONE, PRICE_DECIMALS, USD_DECIMALS};
use crate::errors::AequaError;

mod wide {
    uint::construct_uint! {
        pub struct U256(4);
    }
}
pub use wide::U256;

fn pow10(n: u32) -> U256 {
    U256::from(10u64).pow(U256::from(n))
}

fn to_u64(v: U256) -> Result<u64> {
    require!(v <= U256::from(u64::MAX), AequaError::MathOverflow);
    Ok(v.as_u64())
}

/// `out = floor(amount × (10000 − fee) / 10000)`; the fee is the rest.
pub fn after_fee(amount: u64, fee_bps: u16) -> (u64, u64) {
    let out = ((amount as u128) * (BPS - fee_bps as u64) as u128 / BPS as u128) as u64;
    (out, amount - out)
}

/// xStock raw units to dollars at `P × (1 − band)`, before the fee (the mint path):
/// `raw × M × P × (10000 − band) × 10^6 / (10^d × 10^12 × 10^8 × 10000)`.
pub fn xstock_to_usd(raw: u64, mult_e12: u128, price_e8: u64, band_bps: u16, decimals: u8) -> Result<u64> {
    let n = U256::from(raw) * U256::from(mult_e12) * U256::from(price_e8) * U256::from(BPS - band_bps as u64) * pow10(USD_DECIMALS as u32);
    let d = pow10(decimals as u32) * U256::from(MULT_ONE) * pow10(PRICE_DECIMALS) * U256::from(BPS);
    to_u64(n / d)
}

/// Dollars (after the fee) to xStock raw units at `P × (1 + band)` (the redeem path).
pub fn usd_to_xstock(usd: u64, mult_e12: u128, price_e8: u64, band_bps: u16, decimals: u8) -> Result<u64> {
    let n = U256::from(usd) * pow10(decimals as u32) * U256::from(MULT_ONE) * pow10(PRICE_DECIMALS) * U256::from(BPS);
    let d = pow10(USD_DECIMALS as u32) * U256::from(mult_e12) * U256::from(price_e8) * U256::from(BPS + band_bps as u64);
    require!(!d.is_zero(), AequaError::MathOverflow);
    to_u64(n / d)
}

/// Mid value of xStock raw units in dollars: `raw × M × P` (the reserve's valuation).
pub fn xstock_value(raw: u64, mult_e12: u128, price_e8: u64, decimals: u8) -> Result<u64> {
    xstock_to_usd(raw, mult_e12, price_e8, 0, decimals)
}

/// Raw units worth `usd` at the mid price, rounded down.
pub fn usd_to_xstock_mid(usd: u64, mult_e12: u128, price_e8: u64, decimals: u8) -> Result<u64> {
    usd_to_xstock(usd, mult_e12, price_e8, 0, decimals)
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

/// `x × num / den`, rounded down, for u64 amounts and bps-like ratios.
pub fn mul_div(x: u64, num: u64, den: u64) -> Result<u64> {
    require!(den != 0, AequaError::MathOverflow);
    let v = (x as u128) * (num as u128) / (den as u128);
    require!(v <= u64::MAX as u128, AequaError::MathOverflow);
    Ok(v as u64)
}

/// A Pyth price `mantissa × 10^expo` at 8 decimals, rounded down.
pub fn price_e8(mantissa: i64, expo: i16) -> Result<u64> {
    require!(mantissa > 0, AequaError::InvalidPythPrice);
    let m = mantissa as u128;
    let shift = PRICE_DECIMALS as i32 + expo as i32;
    let v = if shift >= 0 {
        require!(shift <= 30, AequaError::InvalidPythPrice);
        m.checked_mul(10u128.pow(shift as u32)).ok_or(AequaError::MathOverflow)?
    } else {
        require!(shift >= -30, AequaError::InvalidPythPrice);
        m / 10u128.pow((-shift) as u32)
    };
    require!(v > 0 && v <= u64::MAX as u128, AequaError::InvalidPythPrice);
    Ok(v as u64)
}

/// The ScaledUiAmount multiplier as an integer at 1e12: `floor(m × 1e12 + 0.5)`, the same
/// rounding as `Math.round` in the app.
pub fn multiplier_e12(m: f64) -> Option<u128> {
    if !m.is_finite() || m <= 0.0 || m > 1.0e6 {
        return None;
    }
    let v = m * 1.0e12 + 0.5;
    let r = v as u128;
    if r == 0 {
        None
    } else {
        Some(r)
    }
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
    fn xstock_round_trip_never_gains() {
        let m = multiplier_e12(1.005714560286254).unwrap();
        let p = 66_512_340_000u64; // $665.1234 at 1e8
        let raw = 12_345_678_901u64; // 123.45678901 SPYx
        let usd = xstock_to_usd(raw, m, p, 5, 8).unwrap();
        let back = usd_to_xstock(usd, m, p, 5, 8).unwrap();
        assert!(back < raw);
    }

    #[test]
    fn prices() {
        assert_eq!(price_e8(66_512_340, -5).unwrap(), 66_512_340_000);
        assert_eq!(price_e8(6_651_234_000_000, -10).unwrap(), 66_512_340_000);
        assert!(price_e8(0, -8).is_err());
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
