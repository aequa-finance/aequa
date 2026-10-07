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

    pub fn set_params(ctx: Context<AdminOnly>, params: Params) -> Result<()> {
        ctx.accounts.set_params(params)
    }

    pub fn set_roles(ctx: Context<AdminOnly>, guardian: Pubkey, keeper: Pubkey, reporter: Pubkey, pending_admin: Pubkey) -> Result<()> {
        ctx.accounts.set_roles(guardian, keeper, reporter, pending_admin)
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        ctx.accounts.run()
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

    // USDC paths

    pub fn mint_with_usdc(ctx: Context<MintWithUsdc>, amount: u64, min_out: u64) -> Result<()> {
        ctx.accounts.run(amount, min_out)
    }

    pub fn redeem_for_usdc(ctx: Context<RedeemForUsdc>, amount: u64, min_out: u64) -> Result<()> {
        ctx.accounts.run(amount, min_out)
    }
}
