//! The hard-coded range of every parameter governance can set. A value
//! outside its range fails; caps can at most double in one change.

use anchor_lang::prelude::*;

use crate::constants::{DAY, HOUR};
use crate::errors::AequaError;
use crate::state::Params;

fn within<T: PartialOrd>(v: T, lo: T, hi: T) -> Result<()> {
    require!(v >= lo && v <= hi, AequaError::ParamOutOfRange);
    Ok(())
}

/// A capacity or spot cap may at most double in one change; lowering is free. From zero it
/// may return to its launch value.
pub fn check_cap_change(old: u64, new: u64, launch: u64) -> Result<()> {
    let ceiling = if old == 0 { launch } else { old.saturating_mul(2) };
    require!(new <= ceiling, AequaError::ParamOutOfRange);
    Ok(())
}

/// Launch values.
pub const LAUNCH_USDC_HOURLY_CAP: u64 = 1_000_000_000_000; // $1,000,000
pub const LAUNCH_XSTOCK_HOURLY_CAP: u64 = 500_000_000_000; // $500,000
pub const LAUNCH_SPOT_CAP: u64 = 15_000_000_000_000; // $15,000,000

impl Params {
    /// The launch values.
    pub fn launch() -> Self {
        Self {
            leverage_bps: [20_000, 30_000, 40_000],
            buffer_bps: [500, 1_000, 1_500],
            delta_tolerance_bps: 50,
            usdc_hourly_cap: LAUNCH_USDC_HOURLY_CAP,
            base_fee_bps: 10,
            tier_stake: [100_000, 1_000_000],
            tier_fee_bps: [5, 2],
            keeper_slippage_bps: 50,
            pyth_freshness_secs: 10,
            report_freshness_secs: 120,
            distribution_interval_secs: 8 * HOUR as u32,
            susdq_vesting_secs: 8 * HOUR as u32,
            cushion_bps: 25,
            yield_cap_bps_per_day: 5,
            reserve_fund_share_bps: 1_000,
            protocol_share_bps: 1_000,
            reserve_fund_target_bps: 200,
            susdq_cooldown_secs: 7 * DAY as u32,
            aequa_cooldown_secs: 10 * DAY as u32,
            ca_window_before_secs: 24 * HOUR as u32,
            ca_window_after_secs: HOUR as u32,
        }
    }

    /// Checks every field against its range. `old` is the value in force, for the caps that
    /// may only double; `None` at initialization.
    pub fn validate(&self, old: Option<&Params>) -> Result<()> {
        let [lo, target, hi] = self.leverage_bps;
        for v in self.leverage_bps {
            within(v, 15_000, 50_000)?;
        }
        require!(lo <= target && target <= hi, AequaError::ParamOutOfRange);

        let [lo, target, hi] = self.buffer_bps;
        for v in self.buffer_bps {
            within(v, 300, 2_500)?;
        }
        require!(lo <= target && target <= hi, AequaError::ParamOutOfRange);

        within(self.delta_tolerance_bps, 10, 100)?;
        within(self.base_fee_bps, 0, 30)?;
        for i in 0..2 {
            require!(self.tier_fee_bps[i] <= self.base_fee_bps, AequaError::ParamOutOfRange);
        }
        require!(self.tier_stake[0] <= self.tier_stake[1], AequaError::ParamOutOfRange);
        within(self.keeper_slippage_bps, 10, 100)?;
        within(self.pyth_freshness_secs, 2, 30)?;
        within(self.report_freshness_secs, 30, 600)?;
        within(self.distribution_interval_secs as i64, HOUR, 24 * HOUR)?;
        within(self.susdq_vesting_secs as i64, HOUR, 24 * HOUR)?;
        within(self.cushion_bps, 0, 100)?;
        within(self.yield_cap_bps_per_day, 1, 10)?;
        // The Reserve Fund's share is fixed; the protocol's share moves within 5%–20%,
        // taken from or returned to sUSDq's share.
        require!(self.reserve_fund_share_bps == 1_000, AequaError::ParamOutOfRange);
        within(self.protocol_share_bps, 500, 2_000)?;
        within(self.reserve_fund_target_bps, 100, 500)?;
        within(self.susdq_cooldown_secs as i64, 3 * DAY, 14 * DAY)?;
        within(self.aequa_cooldown_secs as i64, 7 * DAY, 21 * DAY)?;
        within(self.ca_window_before_secs as i64, 0, 72 * HOUR)?;
        within(self.ca_window_after_secs as i64, 0, 24 * HOUR)?;

        match old {
            Some(old) => check_cap_change(old.usdc_hourly_cap, self.usdc_hourly_cap, LAUNCH_USDC_HOURLY_CAP)?,
            None => require!(self.usdc_hourly_cap <= LAUNCH_USDC_HOURLY_CAP, AequaError::ParamOutOfRange),
        }
        Ok(())
    }
}

/// A collateral's session bands: each 1–100 bps.
pub fn check_bands(bands: &[u16; 4]) -> Result<()> {
    for b in bands {
        within(*b, 1, 100)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_values_pass() {
        let p = Params::launch();
        p.validate(None).unwrap();
        assert_eq!(p.spot_share_bps(), 6_750);
    }

    #[test]
    fn caps_at_most_double() {
        assert!(check_cap_change(100, 200, 1).is_ok());
        assert!(check_cap_change(100, 201, 1).is_err());
        assert!(check_cap_change(100, 0, 1).is_ok());
        assert!(check_cap_change(0, 1, 1).is_ok());
        assert!(check_cap_change(0, 2, 1).is_err());
    }

    #[test]
    fn out_of_range_fails() {
        let mut p = Params::launch();
        p.base_fee_bps = 31;
        assert!(p.validate(None).is_err());
        let mut p = Params::launch();
        p.protocol_share_bps = 2_100;
        assert!(p.validate(None).is_err());
        let mut p = Params::launch();
        p.tier_fee_bps = [11, 2];
        assert!(p.validate(None).is_err());
    }
}
