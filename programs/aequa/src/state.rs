//! Program accounts. Every token account the program holds sits at its own PDA
//! and is its own authority, so a signature the program gives for one account can move
//! nothing else.

use anchor_lang::prelude::*;

use crate::constants::{cctp, MAX_COLLATERALS};
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
    /// The keeper's swaps must clear the Pyth price less this.
    pub keeper_slippage_bps: u16,
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
    pub aequa_cooldown_secs: u32,
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
    pub fn draw(&mut self, amount: u64, cap: u64, now: i64) -> Result<()> {
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

/// Where `bridge_out` sends USDC: a Circle CCTP V2 burn to HyperEVM whose hook
/// names the hedge account, so Circle's CctpForwarder mints and credits it on HyperCore.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub struct BridgeRoute {
    pub token_messenger_minter: Pubkey,
    pub message_transmitter: Pubkey,
    /// HyperEVM's CCTP domain (19).
    pub destination_domain: u32,
    /// The CctpForwarder, left-padded to 32 bytes; also the only allowed caller.
    pub mint_recipient: [u8; 32],
    pub destination_caller: [u8; 32],
    /// The keeper's fee may be at most `amount × max_fee_bps / 10,000 + max_fee_flat`.
    pub max_fee_bps: u16,
    pub max_fee_flat: u64,
    /// The forwarding hook: "cctp-forward", version 0, the hedge account and its dex.
    pub hook_len: u8,
    pub hook_data: [u8; 96],
}

impl Default for BridgeRoute {
    fn default() -> Self {
        Self {
            token_messenger_minter: Pubkey::default(),
            message_transmitter: Pubkey::default(),
            destination_domain: 0,
            mint_recipient: [0; 32],
            destination_caller: [0; 32],
            max_fee_bps: 0,
            max_fee_flat: 0,
            hook_len: 0,
            hook_data: [0; 96],
        }
    }
}

impl BridgeRoute {
    pub fn is_set(&self) -> bool {
        self.token_messenger_minter != Pubkey::default() && self.mint_recipient != [0u8; 32]
    }

    /// The route to a HyperCore account: CCTP V2 to HyperEVM, Circle's CctpForwarder as mint
    /// recipient and caller, and the forwarding hook naming the account (version 0, 24 bytes
    /// of data: the address and the main perp dex).
    pub fn to_hypercore(recipient: [u8; 20], max_fee_bps: u16, max_fee_flat: u64) -> Self {
        let mut hook_data = [0u8; 96];
        hook_data[..cctp::HOOK_MAGIC.len()].copy_from_slice(cctp::HOOK_MAGIC);
        hook_data[28..32].copy_from_slice(&24u32.to_be_bytes());
        hook_data[32..52].copy_from_slice(&recipient);
        Self {
            token_messenger_minter: cctp::TOKEN_MESSENGER_MINTER,
            message_transmitter: cctp::MESSAGE_TRANSMITTER,
            destination_domain: cctp::HYPEREVM_DOMAIN,
            mint_recipient: cctp::FORWARDER,
            destination_caller: cctp::FORWARDER,
            max_fee_bps,
            max_fee_flat,
            hook_len: cctp::HOOK_LEN as u8,
            hook_data,
        }
    }
}

/// The AEQUA side, set once the token exists.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub struct AequaSetup {
    pub mint: Pubkey,
    pub token_program: Pubkey,
    pub decimals: u8,
}

impl AequaSetup {
    pub fn is_set(&self) -> bool {
        self.mint != Pubkey::default()
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
    pub bridge: BridgeRoute,
    pub aequa: AequaSetup,
    pub bumps: Bumps,
    /// The swap authority's USDC transit account.
    pub transit_usdc: Pubkey,
    pub _reserved: [u8; 128],
}

/// Bumps of the program's PDAs, so signing never searches for one.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub struct Bumps {
    pub config: u8,
    pub reserve: u8,
    pub usdq_vault: u8,
    pub aequa_pool: u8,
    pub usdq_mint: u8,
    pub susdq_mint: u8,
    pub buffer: u8,
    pub landing: u8,
    pub susdq_assets: u8,
    pub susdq_silo: u8,
    pub reserve_fund: u8,
    pub protocol: u8,
    pub aequa_pool_vault: u8,
    pub swap_authority: u8,
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

    /// The fee for a holder whose active stAEQUA is `active`.
    pub fn fee_bps(&self, active: u64) -> u16 {
        let p = &self.params;
        let mut fee = p.base_fee_bps;
        for i in 0..2 {
            if p.tier_stake[i] > 0 && active >= p.tier_stake[i] {
                fee = fee.min(p.tier_fee_bps[i]);
            }
        }
        fee
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
    /// The swap authority's transit account for this collateral.
    pub transit: Pubkey,
    pub bump: u8,
    pub vault_bump: u8,
    pub _reserved: [u8; 64],
}

/// The reporter's latest figures for the hedge account.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub struct HedgeReport {
    /// Account value on HyperCore, dollars at 6 decimals.
    pub account_value: i64,
    /// Short notional per collateral index, dollars at 6 decimals.
    pub short_notional: [u64; MAX_COLLATERALS],
    /// Bit i: the perp of collateral i trades in its external session.
    pub market_live: u8,
    /// USDC credited to the hedge account from `bridge_out`, cumulative.
    pub credited_total: u64,
    /// USDC withdrawn from the hedge account towards the landing account, cumulative.
    pub withdrawn_total: u64,
    /// HyperCore's time for these figures, in milliseconds.
    pub hl_time: i64,
    /// Solana time the report landed.
    pub reported_at: i64,
}

#[account]
#[derive(InitSpace)]
pub struct Reserve {
    pub report: HedgeReport,
    pub bridged_out_total: u64,
    pub landed_total: u64,
    pub last_distribution: i64,
    /// NAV and supply at the last distribution, and the shortfall it left after burning the
    /// Reserve Fund (zero when NAV covered supply).
    pub last_nav: i64,
    pub last_supply: u64,
    pub deficit: u64,
    /// Distributions in a row that found NAV below supply with the Reserve Fund spent.
    pub deficit_streak: u8,
    pub distributions: u64,
    pub distributed_total: u64,
    pub deficit_burned_total: u64,
    pub bump: u8,
    pub _reserved: [u8; 64],
}

impl Reserve {
    /// `(bridged_out − credited) + (withdrawn − landed)`: USDC between the chains.
    pub fn in_flight(&self) -> i128 {
        (self.bridged_out_total as i128 - self.report.credited_total as i128)
            + (self.report.withdrawn_total as i128 - self.landed_total as i128)
    }

    /// A report counts while HyperCore's time for its figures is within `freshness` of now.
    pub fn report_fresh(&self, now: i64, freshness: u32) -> bool {
        self.report.reported_at > 0 && now.saturating_mul(1_000).saturating_sub(self.report.hl_time) <= freshness as i64 * 1_000
    }

    /// Hedge live for a collateral: a fresh report that marks its perp in session.
    pub fn hedge_live(&self, index: u8, now: i64, freshness: u32) -> bool {
        self.report_fresh(now, freshness) && self.report.market_live & (1u8 << index) != 0
    }
}

/// The front of `AequaPool`: what the fee tier reads. Borsh lays fields out in order, so these
/// are the pool account's first bytes after its discriminator; reading only them keeps the
/// paths that take the pool well inside the stack.
#[derive(AnchorDeserialize, Clone, Copy, Debug)]
pub struct PoolHead {
    pub base: u64,
    pub stream: Vesting,
    pub total_shares: u64,
}

impl PoolHead {
    /// The head of this program's AequaPool account.
    pub fn read(info: &AccountInfo) -> Result<Self> {
        require_keys_eq!(*info.owner, crate::ID, anchor_lang::error::ErrorCode::AccountOwnedByWrongProgram);
        let data = info.try_borrow_data()?;
        require!(data.len() >= 8 && &data[..8] == AequaPool::DISCRIMINATOR, anchor_lang::error::ErrorCode::AccountDiscriminatorMismatch);
        Ok(Self::deserialize(&mut &data[8..])?)
    }

    pub fn assets(&self, now: i64) -> u64 {
        self.base + self.stream.vested(now)
    }
}

/// A linear release of `amount` between `start` and `end` (sUSDq vesting, the stAEQUA stream).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub struct Vesting {
    pub amount: u64,
    pub start: i64,
    pub end: i64,
}

impl Vesting {
    pub fn vested(&self, now: i64) -> u64 {
        math::vested(self.amount, self.start, self.end, now)
    }

    /// Moves what has vested out of the schedule, keeping the rest on the same rate.
    pub fn fold(&mut self, now: i64) -> u64 {
        let done = self.vested(now);
        self.amount -= done;
        if now > self.start {
            self.start = now.min(self.end);
        }
        done
    }

    /// Restarts the schedule with what is left plus `more`, over `period` from now.
    pub fn restart(&mut self, now: i64, more: u64, period: i64) -> Result<u64> {
        let done = self.fold(now);
        self.amount = self.amount.checked_add(more).ok_or(AequaError::MathOverflow)?;
        self.start = now;
        self.end = now + period;
        Ok(done)
    }
}

/// sUSDq's state: USDq it holds for stakers and what is still vesting.
#[account]
#[derive(InitSpace)]
pub struct UsdqVault {
    /// Vested USDq behind sUSDq. Tokens sent straight to the assets account don't count.
    pub base: u64,
    pub vesting: Vesting,
    pub susdq_mint: Pubkey,
    pub bump: u8,
    pub _reserved: [u8; 32],
}

impl UsdqVault {
    pub fn total_assets(&self, now: i64) -> u64 {
        self.base + self.vesting.vested(now)
    }

    /// Moves vested yield into `base`.
    pub fn fold(&mut self, now: i64) {
        self.base += self.vesting.fold(now);
    }
}

#[account]
#[derive(InitSpace)]
pub struct UsdqCooldown {
    pub owner: Pubkey,
    pub amount: u64,
    pub unlock_at: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct AequaPool {
    /// Vested AEQUA in the pool. Tokens sent straight to the vault don't count.
    pub base: u64,
    pub stream: Vesting,
    pub total_shares: u64,
    pub bump: u8,
    pub _reserved: [u8; 48],
}

impl AequaPool {
    pub fn assets(&self, now: i64) -> u64 {
        self.base + self.stream.vested(now)
    }

    pub fn fold(&mut self, now: i64) {
        self.base += self.stream.fold(now);
    }
}

#[account]
#[derive(InitSpace)]
pub struct AequaStake {
    pub owner: Pubkey,
    pub shares: u64,
    /// When unstaking started; zero for an active position.
    pub cooldown_start: i64,
    pub bump: u8,
}

/// Where a position is in its unstaking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StakeState {
    Empty,
    Active,
    Cooling,
    Window,
}

impl AequaStake {
    pub fn state(&self, now: i64, cooldown: i64, window: i64) -> StakeState {
        if self.shares == 0 {
            return StakeState::Empty;
        }
        if self.cooldown_start == 0 {
            return StakeState::Active;
        }
        let opens = self.cooldown_start + cooldown;
        if now < opens {
            StakeState::Cooling
        } else if now < opens + window {
            StakeState::Window
        } else {
            // The window passed: the position is active again.
            StakeState::Active
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> AequaPool {
        AequaPool {
            base: 0,
            stream: Vesting::default(),
            total_shares: 0,
            bump: 0,
            _reserved: [0; 48],
        }
    }

    #[test]
    fn the_pool_head_reads_the_pool_account() {
        let mut p = pool();
        p.base = 7;
        p.stream = Vesting { amount: 11, start: 13, end: 17 };
        p.total_shares = 19;
        let mut data = AequaPool::DISCRIMINATOR.to_vec();
        p.serialize(&mut data).unwrap();
        let head = PoolHead::deserialize(&mut &data[8..]).unwrap();
        assert_eq!((head.base, head.stream, head.total_shares), (7, Vesting { amount: 11, start: 13, end: 17 }, 19));
    }
}
