//! aequa: USDq, a dollar backed by tokenized S&P 500 and Nasdaq-100 ETFs held on Solana and
//! hedged with short index perps on Hyperliquid; sUSDq, staked USDq; and stAEQUA.

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod instructions;
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
}
