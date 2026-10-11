//! What each instruction did, for indexers and the reserve page.

use anchor_lang::prelude::*;

/// Where a fee went.
pub const TO_RESERVE_FUND: u8 = 0;
pub const TO_PROTOCOL: u8 = 1;

#[event]
pub struct Minted {
    pub user: Pubkey,
    /// USDC or the collateral mint.
    pub asset: Pubkey,
    pub amount_in: u64,
    pub usdq_out: u64,
    pub fee: u64,
    pub fee_to: u8,
    pub fee_bps: u16,
    /// The price used, at 1e8, and its session (0 for USDC).
    pub price_e8: u64,
    pub session: u8,
}

#[event]
pub struct Redeemed {
    pub user: Pubkey,
    pub asset: Pubkey,
    pub usdq_in: u64,
    pub amount_out: u64,
    pub fee: u64,
    pub fee_to: u8,
    pub fee_bps: u16,
    pub price_e8: u64,
    pub session: u8,
}

#[event]
pub struct UsdqStaked {
    pub user: Pubkey,
    pub usdq: u64,
    pub shares: u64,
}

#[event]
pub struct UsdqCooldownStarted {
    pub user: Pubkey,
    pub shares: u64,
    pub usdq: u64,
    pub cooling: u64,
    pub unlock_at: i64,
}

#[event]
pub struct UsdqClaimed {
    pub user: Pubkey,
    pub usdq: u64,
}

#[event]
pub struct Distributed {
    pub nav: i64,
    pub supply: u64,
    pub surplus: i64,
    pub amount: u64,
    pub to_susdq: u64,
    pub to_reserve_fund: u64,
    pub to_protocol: u64,
    /// USDq burned from the Reserve Fund against a deficit.
    pub burned: u64,
    pub deficit_streak: u8,
    /// USDq per sUSDq after the distribution, at 1e12.
    pub susdq_rate_e12: u128,
    pub at: i64,
}

#[event]
pub struct HedgeReported {
    pub account_value: i64,
    pub market_live: u8,
    pub credited_total: u64,
    pub withdrawn_total: u64,
    pub hl_time: i64,
}

#[event]
pub struct Swapped {
    pub collateral: Pubkey,
    /// True when USDC bought collateral.
    pub buy: bool,
    pub amount_in: u64,
    pub amount_out: u64,
    pub min_out: u64,
    pub price_e8: u64,
}

#[event]
pub struct BridgedOut {
    pub amount: u64,
    pub bridged_out_total: u64,
    pub message: Pubkey,
}

#[event]
pub struct Landed {
    pub amount: u64,
    pub landed_total: u64,
}

#[event]
pub struct AequaStaked {
    pub user: Pubkey,
    pub aequa: u64,
    pub shares: u64,
}

#[event]
pub struct AequaUnstakeStarted {
    pub user: Pubkey,
    pub shares: u64,
    pub window_opens: i64,
    pub window_closes: i64,
}

#[event]
pub struct AequaWithdrawn {
    pub user: Pubkey,
    pub aequa: u64,
    pub shares: u64,
}

#[event]
pub struct BoughtBack {
    pub usdq_in: u64,
    pub aequa_out: u64,
    pub burned: u64,
    pub streamed: u64,
    pub twap_e12: u128,
}

#[event]
pub struct DeficitCovered {
    /// Reserve Fund USDq burned against the shortfall first.
    pub fund_burned: u64,
    /// AEQUA moved to the slash-sale vault.
    pub aequa: u64,
    /// The shortfall left for the slash sale to raise.
    pub deficit: u64,
}

#[event]
pub struct SlashSold {
    pub aequa_in: u64,
    pub usdq_out: u64,
    pub burned: u64,
    pub to_reserve_fund: u64,
}

#[event]
pub struct PathsPaused {
    pub by: Pubkey,
    pub paused: u32,
    pub collateral: Option<Pubkey>,
}

#[event]
pub struct ParamsChanged {
    pub by: Pubkey,
}
