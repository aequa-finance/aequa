//! Pieces every path shares: the fee tier, where a fee goes, and signing for the program's
//! own token accounts.

use anchor_lang::prelude::*;

use crate::math;
use crate::state::{AequaStake, Config, PoolHead, StakeState};
use crate::constants::AEQUA_WINDOW_SECS;

/// Active stAEQUA for the fee tier, rounded up so a stake of exactly a threshold reaches its
/// tier. Cooling positions have no tier.
pub fn active_aequa(config: &Config, stake: Option<&AequaStake>, pool: Option<&PoolHead>, owner: &Pubkey, now: i64) -> u64 {
    let (Some(stake), Some(pool)) = (stake, pool) else { return 0 };
    if stake.owner != *owner {
        return 0;
    }
    if stake.state(now, config.params.aequa_cooldown_secs as i64, AEQUA_WINDOW_SECS) != StakeState::Active {
        return 0;
    }
    math::assets_for_up(stake.shares, pool.assets(now), pool.total_shares)
}

/// The mint and redeem fee for this holder; `pool` is the stAEQUA pool account, read only for
/// its head.
pub fn fee_bps(config: &Config, stake: Option<&AequaStake>, pool: Option<&AccountInfo>, owner: &Pubkey, now: i64) -> Result<u16> {
    let head = match (stake, pool) {
        (Some(_), Some(p)) => Some(PoolHead::read(p)?),
        _ => None,
    };
    Ok(config.fee_bps(active_aequa(config, stake, head.as_ref(), owner, now)))
}

/// Fees go to the Reserve Fund until it holds its target, then to the protocol vault.
/// `supply` is USDq supply once this instruction is done.
pub fn fee_to_reserve_fund(config: &Config, reserve_fund_balance: u64, supply: u64) -> bool {
    reserve_fund_balance < config.reserve_fund_target(supply)
}

/// Seeds for a PDA that is its own authority.
#[macro_export]
macro_rules! signer {
    ($seed:expr, $bump:expr) => {
        &[&[$seed, &[$bump]][..]]
    };
}
