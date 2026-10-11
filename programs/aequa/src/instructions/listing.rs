//! Listing collateral and changing a listing. New collateral must pass the
//! admission checks; caps may at most double in one change.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{seeds::*, MAX_COLLATERALS};
use crate::errors::AequaError;
use crate::events::ParamsChanged;
use crate::params::{check_bands, check_cap_change, LAUNCH_SPOT_CAP, LAUNCH_XSTOCK_HOURLY_CAP};
use crate::state::{Bucket, Collateral, Config};
use crate::token2022::read_mint;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct ListingArgs {
    /// The ETF's Pyth Pro feed id.
    pub feed: u32,
    pub bands_bps: [u16; 4],
    pub spot_cap: u64,
    pub hourly_cap: u64,
    pub perp: [u8; 24],
    pub perp_asset: u32,
}

#[derive(Accounts)]
pub struct AddCollateral<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config, has_one = admin @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub admin: Signer<'info>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(init, payer = payer, space = 8 + Collateral::INIT_SPACE, seeds = [COLLATERAL, mint.key().as_ref()], bump)]
    pub collateral: Box<Account<'info, Collateral>>,
    #[account(
        init,
        payer = payer,
        seeds = [COLLATERAL_VAULT, mint.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
        token::token_program = token_program,
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: the swap authority PDA; holds no data.
    #[account(seeds = [SWAP_AUTHORITY], bump = config.bumps.swap_authority)]
    pub swap_authority: UncheckedAccount<'info>,
    /// The swap authority's transit account for this mint; anyone can create it, so the setup
    /// creates it (idempotently) and this only checks it.
    #[account(associated_token::mint = mint, associated_token::authority = swap_authority, associated_token::token_program = token_program)]
    pub transit: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl AddCollateral<'_> {
    pub fn run(&mut self, args: ListingArgs, bumps: &AddCollateralBumps) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &mut self.config;
        let count = config.collateral_count as usize;
        require!(count < MAX_COLLATERALS, AequaError::TooManyCollaterals);
        // Admission: the mint must read cleanly now.
        let p = &config.params;
        read_mint(&self.mint.to_account_info(), now, p.ca_window_before_secs, p.ca_window_after_secs)?;
        check_bands(&args.bands_bps)?;
        require!(args.spot_cap <= LAUNCH_SPOT_CAP, AequaError::ParamOutOfRange);
        require!(args.hourly_cap <= LAUNCH_XSTOCK_HOURLY_CAP, AequaError::ParamOutOfRange);
        require!(args.feed != 0, AequaError::ParamOutOfRange);

        let c = &mut self.collateral;
        c.mint = self.mint.key();
        c.vault = self.vault.key();
        c.token_program = self.token_program.key();
        c.decimals = self.mint.decimals;
        c.index = count as u8;
        c.paused = false;
        c.feed = args.feed;
        c.bands_bps = args.bands_bps;
        c.spot_cap = args.spot_cap;
        c.hourly_cap = args.hourly_cap;
        c.mint_bucket = Bucket::full(args.hourly_cap, now);
        c.redeem_bucket = Bucket::full(args.hourly_cap, now);
        c.perp = args.perp;
        c.perp_asset = args.perp_asset;
        c.transit = self.transit.key();
        c.bump = bumps.collateral;
        c.vault_bump = bumps.vault;

        config.collaterals[count] = self.mint.key();
        config.collateral_count += 1;
        config.spot_cap_total = config.spot_cap_total.checked_add(args.spot_cap).ok_or(AequaError::MathOverflow)?;
        emit!(ParamsChanged { by: self.admin.key() });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct SetCollateral<'info> {
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config, has_one = admin @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub admin: Signer<'info>,
    #[account(mut, seeds = [COLLATERAL, collateral.mint.as_ref()], bump = collateral.bump)]
    pub collateral: Box<Account<'info, Collateral>>,
}

impl SetCollateral<'_> {
    pub fn run(&mut self, args: ListingArgs) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let c = &mut self.collateral;
        check_bands(&args.bands_bps)?;
        check_cap_change(c.spot_cap, args.spot_cap, LAUNCH_SPOT_CAP)?;
        check_cap_change(c.hourly_cap, args.hourly_cap, LAUNCH_XSTOCK_HOURLY_CAP)?;
        require!(args.feed != 0, AequaError::ParamOutOfRange);

        // Buckets keep their level across a cap change.
        let old_cap = c.hourly_cap;
        c.mint_bucket.settle(old_cap, now);
        c.redeem_bucket.settle(old_cap, now);
        c.mint_bucket.available = c.mint_bucket.available.min(args.hourly_cap);
        c.redeem_bucket.available = c.redeem_bucket.available.min(args.hourly_cap);

        let config = &mut self.config;
        config.spot_cap_total = config.spot_cap_total - c.spot_cap + args.spot_cap;
        c.feed = args.feed;
        c.bands_bps = args.bands_bps;
        c.spot_cap = args.spot_cap;
        c.hourly_cap = args.hourly_cap;
        c.perp = args.perp;
        c.perp_asset = args.perp_asset;
        emit!(ParamsChanged { by: self.admin.key() });
        Ok(())
    }
}
