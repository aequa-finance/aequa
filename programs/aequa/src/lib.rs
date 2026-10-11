//! aequa: USDq, a dollar backed by tokenized S&P 500 and Nasdaq-100 ETFs held on Solana and
//! hedged with short index perps on Hyperliquid; sUSDq, staked USDq; and stAEQUA.

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod instructions;
pub mod jupiter;
pub mod math;
pub mod params;
pub mod pyth;
pub mod state;
pub mod token2022;

use instructions::*;
use state::Params;

declare_id!("9xhUvFQy1yAnyuZTygfK1nD6naicXQoWMR6doxFxuTbL");

#[program]
pub mod aequa {
    use super::*;

    // Setup and roles

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        ctx.accounts.run(args, &ctx.bumps)
    }

    pub fn create_vaults(ctx: Context<CreateVaults>) -> Result<()> {
        ctx.accounts.run(&ctx.bumps)
    }

    pub fn create_susdq_vaults(ctx: Context<CreateSusdqVaults>) -> Result<()> {
        ctx.accounts.run(&ctx.bumps)
    }

    pub fn add_collateral(ctx: Context<AddCollateral>, args: ListingArgs) -> Result<()> {
        ctx.accounts.run(args, &ctx.bumps)
    }

    pub fn set_collateral(ctx: Context<SetCollateral>, args: ListingArgs) -> Result<()> {
        ctx.accounts.run(args)
    }

    pub fn set_params(ctx: Context<AdminOnly>, params: Params) -> Result<()> {
        ctx.accounts.set_params(params)
    }

    pub fn set_roles(ctx: Context<AdminOnly>, guardian: Pubkey, keeper: Pubkey, reporter: Pubkey, pending_admin: Pubkey) -> Result<()> {
        ctx.accounts.set_roles(guardian, keeper, reporter, pending_admin)
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        ctx.accounts.run()
    }

    pub fn set_bridge_route(ctx: Context<AdminOnly>, args: BridgeArgs) -> Result<()> {
        ctx.accounts.set_bridge_route(args)
    }

    pub fn set_token_metadata(ctx: Context<SetTokenMetadata>, args: TokenMetadataArgs) -> Result<()> {
        ctx.accounts.run(args)
    }

    pub fn pause(ctx: Context<Pause>, paths: u32) -> Result<()> {
        ctx.accounts.run(paths)
    }

    pub fn unpause(ctx: Context<AdminOnly>, paths: u32) -> Result<()> {
        ctx.accounts.unpause(paths)
    }

    pub fn unpause_collateral(ctx: Context<UnpauseCollateral>) -> Result<()> {
        ctx.accounts.run()
    }

    // USDC paths

    pub fn mint_with_usdc(ctx: Context<MintWithUsdc>, amount: u64, min_out: u64) -> Result<()> {
        ctx.accounts.run(amount, min_out)
    }

    pub fn redeem_for_usdc(ctx: Context<RedeemForUsdc>, amount: u64, min_out: u64) -> Result<()> {
        ctx.accounts.run(amount, min_out)
    }

    // xStock paths. The signed Pyth Pro update is the first argument, so it sits at a fixed
    // offset (12) the ed25519 instruction points at.

    pub fn mint_with_collateral(ctx: Context<MintWithCollateral>, message: Vec<u8>, ed25519_index: u16, amount: u64, min_out: u64) -> Result<()> {
        ctx.accounts.run(&message, ed25519_index, amount, min_out)
    }

    pub fn redeem_for_collateral(ctx: Context<RedeemForCollateral>, message: Vec<u8>, ed25519_index: u16, amount: u64, min_out: u64) -> Result<()> {
        ctx.accounts.run(&message, ed25519_index, amount, min_out)
    }

    // sUSDq

    pub fn stake_usdq(ctx: Context<StakeUsdq>, amount: u64, min_shares: u64) -> Result<()> {
        ctx.accounts.run(amount, min_shares)
    }

    pub fn start_usdq_cooldown(ctx: Context<StartUsdqCooldown>, shares: u64) -> Result<()> {
        let bump = ctx.bumps.cooldown;
        ctx.accounts.run(shares, bump)
    }

    pub fn claim_usdq(ctx: Context<ClaimUsdq>) -> Result<()> {
        ctx.accounts.run()
    }

    // Hedge account

    pub fn report_hedge(ctx: Context<ReportHedge>, args: ReportArgs) -> Result<()> {
        ctx.accounts.run(args)
    }

    pub fn land_usdc(ctx: Context<LandUsdc>) -> Result<()> {
        ctx.accounts.run()
    }

    pub fn bridge_out(ctx: Context<BridgeOut>, amount: u64, max_fee: u64, fast: bool) -> Result<()> {
        ctx.accounts.run(amount, max_fee, fast)
    }

    /// The keeper's swap between the buffer and a collateral vault. The Jupiter route's
    /// accounts follow as remaining accounts.
    pub fn swap<'info>(ctx: Context<'info, Swap<'info>>, message: Vec<u8>, ed25519_index: u16, buy: bool, amount_in: u64, route: Vec<u8>) -> Result<()> {
        ctx.accounts.run(&message, ed25519_index, buy, amount_in, &route, ctx.remaining_accounts)
    }

    // Cranks

    pub fn distribute<'info>(ctx: Context<'info, Distribute<'info>>, message: Vec<u8>, ed25519_index: u16) -> Result<()> {
        ctx.accounts.run(&message, ed25519_index, ctx.remaining_accounts)
    }

    pub fn cover_deficit(ctx: Context<CoverDeficit>, message: Vec<u8>, ed25519_index: u16) -> Result<()> {
        ctx.accounts.run(&message, ed25519_index)
    }

    pub fn buyback<'info>(ctx: Context<'info, Buyback<'info>>, message: Vec<u8>, ed25519_index: u16, amount: u64, route: Vec<u8>) -> Result<()> {
        ctx.accounts.run(&message, ed25519_index, amount, &route, ctx.remaining_accounts)
    }

    pub fn sell_slashed<'info>(ctx: Context<'info, SellSlashed<'info>>, message: Vec<u8>, ed25519_index: u16, amount: u64, route: Vec<u8>) -> Result<()> {
        ctx.accounts.run(&message, ed25519_index, amount, &route, ctx.remaining_accounts)
    }

    // AEQUA and stAEQUA

    pub fn set_aequa(ctx: Context<SetAequa>, args: AequaArgs) -> Result<()> {
        ctx.accounts.run(args, &ctx.bumps)
    }

    pub fn stake_aequa(ctx: Context<StakeAequa>, amount: u64, min_shares: u64) -> Result<()> {
        let bump = ctx.bumps.aequa_stake;
        ctx.accounts.run(amount, min_shares, bump)
    }

    pub fn start_aequa_unstake(ctx: Context<StartAequaUnstake>) -> Result<()> {
        ctx.accounts.run()
    }

    pub fn withdraw_aequa(ctx: Context<WithdrawAequa>) -> Result<()> {
        ctx.accounts.run()
    }

    pub fn observe_pool(ctx: Context<ObservePool>) -> Result<()> {
        ctx.accounts.run()
    }

    // Governance: the Realms voter-weight plugin

    pub fn create_voter_weight_record(ctx: Context<CreateVoterWeightRecord>, realm: Pubkey) -> Result<()> {
        ctx.accounts.run(realm)
    }

    pub fn update_voter_weight_record(ctx: Context<UpdateVoterWeightRecord>, action: Option<VoterWeightAction>, target: Option<Pubkey>) -> Result<()> {
        ctx.accounts.run(action, target)
    }

    pub fn create_max_voter_weight_record(ctx: Context<CreateMaxVoterWeightRecord>, realm: Pubkey) -> Result<()> {
        ctx.accounts.run(realm)
    }

    pub fn update_max_voter_weight_record(ctx: Context<UpdateMaxVoterWeightRecord>) -> Result<()> {
        ctx.accounts.run()
    }
}
