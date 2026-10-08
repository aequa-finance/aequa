//! Program accounts. Every token account the program holds sits at its own PDA
//! and is its own authority, so a signature the program gives for one account can move
//! nothing else.

use anchor_lang::prelude::*;

use crate::constants::MAX_COLLATERALS;
use crate::errors::AequaError;
use crate::math;

/// The paths the guardian can stop, as bits of `Config::paused`.
pub mod path {
    pub const MINT_USDC: u32 = 1 << 0;
    pub const REDEEM_USDC: u32 = 1 << 1;
    pub const MINT_COLLATERAL: u32 = 1 << 2;
    pub const REDEEM_COLLATERAL: u32 = 1 << 3;
    pub const STAKE_USDQ: u32 = 1 << 4;
    /// `start_usdq_cooldown` and `claim_usdq`.
    pub const UNSTAKE_USDQ: u32 = 1 << 5;
    pub const STAKE_AEQUA: u32 = 1 << 6;
    /// `start_aequa_unstake` and `withdraw_aequa`.
    pub const UNSTAKE_AEQUA: u32 = 1 << 7;
    pub const DISTRIBUTE: u32 = 1 << 8;
    pub const LAND: u32 = 1 << 9;
    pub const BUYBACK: u32 = 1 << 10;
    /// `cover_deficit` and `sell_slashed`.
    pub const COVER_DEFICIT: u32 = 1 << 11;
    /// `swap` and `bridge_out`.
    pub const KEEPER: u32 = 1 << 12;
    pub const REPORT: u32 = 1 << 13;
    pub const ALL: u32 = (1 << 14) - 1;
}

/// Every parameter governance can set, each inside its range in `params.rs`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub struct Params {
    /// Short leverage: band low, target, band high, as leverage × 10,000 (2x, 3x, 4x).
    pub leverage_bps: [u32; 3],
    /// Liquidity buffer: band low, target, band high, in bps of supply (5%, 10%, 15%).
    pub buffer_bps: [u16; 3],
    /// Spot against short, per pair, in bps of supply (0.5%).
    pub delta_tolerance_bps: u16,
    /// USDC mint capacity per hour, in dollars at 6 decimals.
    pub usdc_hourly_cap: u64,
    /// Mint and redeem fee without an active stAEQUA position.
    pub base_fee_bps: u16,
    /// Active stAEQUA, in whole AEQUA, for each lower fee tier.
    pub tier_stake: [u64; 2],
    pub tier_fee_bps: [u16; 2],
    pub pyth_freshness_secs: u32,
    pub report_freshness_secs: u32,
    pub distribution_interval_secs: u32,
    pub susdq_vesting_secs: u32,
    /// Kept in the reserve above supply before anything is distributed, in bps of supply.
    pub cushion_bps: u16,
    /// What one distribution may mint, in bps of supply per day elapsed.
    pub yield_cap_bps_per_day: u16,
    /// Surplus shares: the Reserve Fund's (fixed 10%) and the protocol vault's (5%–20%).
    pub reserve_fund_share_bps: u16,
    pub protocol_share_bps: u16,
    pub reserve_fund_target_bps: u16,
    pub susdq_cooldown_secs: u32,
    /// The corporate-action window around a scheduled multiplier change.
    pub ca_window_before_secs: u32,
    pub ca_window_after_secs: u32,
}

impl Params {
    /// The spot share of supply the targets give: `(1 − buffer) × L / (L + 1)`.
    pub fn spot_share_bps(&self) -> u64 {
        let l = self.leverage_bps[1] as u64;
        let b = self.buffer_bps[1] as u64;
        (10_000 - b) * l / (l + 10_000)
    }
}

/// An hourly token bucket: `available = min(cap, available + cap × elapsed / 3600)`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub struct Bucket {
    pub available: u64,
    pub updated_at: i64,
}

impl Bucket {
    pub fn full(cap: u64, now: i64) -> Self {
        Self { available: cap, updated_at: now }
    }

    pub fn level(&self, cap: u64, now: i64) -> u64 {
        math::bucket_level(self.available, self.updated_at, cap, now)
    }

    /// Takes `amount` from the bucket or fails with `CapacityExceeded`.
    pub fn take(&mut self, amount: u64, cap: u64, now: i64) -> Result<()> {
        let level = self.level(cap, now);
        require!(amount <= level, AequaError::CapacityExceeded);
        self.available = level - amount;
        self.updated_at = now;
        Ok(())
    }

    /// Keeps the bucket's level when its cap changes.
    pub fn settle(&mut self, cap: u64, now: i64) {
        self.available = self.level(cap, now);
        self.updated_at = now;
    }
}

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey,
    pub pending_admin: Pubkey,
    pub guardian: Pubkey,
    pub keeper: Pubkey,
    pub reporter: Pubkey,
    pub usdq_mint: Pubkey,
    pub susdq_mint: Pubkey,
    pub usdc_mint: Pubkey,
    /// Paths stopped by the guardian (`path::*` bits).
    pub paused: u32,
    pub params: Params,
    pub usdc_bucket: Bucket,
    /// Σ spot_cap over the listed collateral; the USDq ceiling is this over the spot share.
    pub spot_cap_total: u64,
    pub collateral_count: u8,
    pub collaterals: [Pubkey; MAX_COLLATERALS],
    pub bumps: Bumps,
    pub _reserved: [u8; 128],
}

/// Bumps of the program's PDAs, so signing never searches for one.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub struct Bumps {
    pub config: u8,
    pub reserve: u8,
    pub usdq_vault: u8,
    pub usdq_mint: u8,
    pub susdq_mint: u8,
    pub buffer: u8,
    pub susdq_assets: u8,
    pub susdq_silo: u8,
    pub reserve_fund: u8,
    pub protocol: u8,
}

impl Config {
    pub fn require_open(&self, bits: u32) -> Result<()> {
        require!(self.paused & bits == 0, AequaError::Paused);
        Ok(())
    }

    /// The USDq supply ceiling: `Σ spot_cap / spot share`.
    pub fn supply_cap(&self) -> u64 {
        let share = self.params.spot_share_bps();
        if share == 0 {
            return 0;
        }
        ((self.spot_cap_total as u128) * 10_000 / share as u128).min(u64::MAX as u128) as u64
    }

    /// The Reserve Fund's target for a supply.
    pub fn reserve_fund_target(&self, supply: u64) -> u64 {
        ((supply as u128) * self.params.reserve_fund_target_bps as u128 / 10_000) as u64
    }
}

/// US equity sessions as Pyth Pro's `marketSession` reports them; each indexes
/// a collateral's bands.
pub mod session {
    pub const REGULAR: u8 = 0;
    pub const PRE: u8 = 1;
    pub const POST: u8 = 2;
    pub const OVERNIGHT: u8 = 3;
    pub const CLOSED: u8 = 4;
}

#[account]
#[derive(InitSpace)]
pub struct Collateral {
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub token_program: Pubkey,
    pub decimals: u8,
    /// Position in `Config::collaterals` and in the hedge report's arrays.
    pub index: u8,
    /// Stopped by the guardian; the USDC paths keep running.
    pub paused: bool,
    /// The ETF's Pyth Pro feed (SPY 1398, QQQ 1363). One feed spans every session; each
    /// update names its session.
    pub feed: u32,
    /// Band per session: regular, pre-market, post-market, overnight, in bps (5 / 15 / 15 / 25).
    pub bands_bps: [u16; 4],
    /// Spot value cap, in dollars at 6 decimals.
    pub spot_cap: u64,
    /// Capacity per hour for each xStock path, in dollars at 6 decimals.
    pub hourly_cap: u64,
    pub mint_bucket: Bucket,
    pub redeem_bucket: Bucket,
    /// The hedge market, e.g. "xyz:SP500", and its HyperCore asset id.
    pub perp: [u8; 24],
    pub perp_asset: u32,
    pub bump: u8,
    pub vault_bump: u8,
    pub _reserved: [u8; 64],
}

#[account]
#[derive(InitSpace)]
pub struct Reserve {
    pub last_distribution: i64,
    pub bump: u8,
    pub _reserved: [u8; 64],
}

/// sUSDq's state.
#[account]
#[derive(InitSpace)]
pub struct UsdqVault {
    pub susdq_mint: Pubkey,
    pub bump: u8,
    pub _reserved: [u8; 32],
}
