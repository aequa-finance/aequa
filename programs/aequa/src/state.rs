//! Program accounts. Every token account the program holds sits at its own PDA
//! and is its own authority, so a signature the program gives for one account can move
//! nothing else.

use anchor_lang::prelude::*;

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
