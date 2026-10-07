//! Pieces every path shares: the fee, where a fee goes, and signing for the program's own
//! token accounts.

use crate::state::Config;

/// The mint and redeem fee. The stAEQUA tiers come once the token exists.
pub fn fee_bps(config: &Config) -> u16 {
    config.params.base_fee_bps
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
