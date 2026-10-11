//! Setup and the admin's and guardian's powers. Every value goes through its
//! hard-coded range.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::seeds::*;
use crate::errors::AequaError;
use crate::events::{ParamsChanged, PathsPaused};
use crate::state::{path, AequaPool, Bucket, Bumps, BridgeRoute, Collateral, Config, Params, Reserve, UsdqVault};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitializeArgs {
    pub admin: Pubkey,
    pub guardian: Pubkey,
    pub keeper: Pubkey,
    pub reporter: Pubkey,
    pub params: Params,
}

/// Creates the program's state and the USDq and sUSDq mints. Only the upgrade authority can
/// run it, once.
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ AequaError::Unauthorized)]
    pub program: Program<'info, crate::program::Aequa>,
    #[account(constraint = program_data.upgrade_authority_address == Some(payer.key()) @ AequaError::Unauthorized)]
    pub program_data: Box<Account<'info, ProgramData>>,
    #[account(init, payer = payer, space = 8 + Config::INIT_SPACE, seeds = [CONFIG], bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(init, payer = payer, space = 8 + Reserve::INIT_SPACE, seeds = [RESERVE], bump)]
    pub reserve: Box<Account<'info, Reserve>>,
    #[account(init, payer = payer, space = 8 + UsdqVault::INIT_SPACE, seeds = [SUSDQ_VAULT], bump)]
    pub usdq_vault: Box<Account<'info, UsdqVault>>,
    #[account(init, payer = payer, space = 8 + AequaPool::INIT_SPACE, seeds = [AEQUA_POOL], bump)]
    pub aequa_pool: Box<Account<'info, AequaPool>>,
    #[account(init, payer = payer, seeds = [USDQ_MINT], bump, mint::decimals = 6, mint::authority = config, mint::token_program = token_program)]
    pub usdq_mint: Box<Account<'info, Mint>>,
    #[account(init, payer = payer, seeds = [SUSDQ_MINT], bump, mint::decimals = 6, mint::authority = usdq_vault, mint::token_program = token_program)]
    pub susdq_mint: Box<Account<'info, Mint>>,
    #[account(mint::token_program = token_program, constraint = usdc_mint.decimals == 6 @ AequaError::CollateralMismatch)]
    pub usdc_mint: Box<Account<'info, Mint>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl Initialize<'_> {
    pub fn run(&mut self, args: InitializeArgs, bumps: &InitializeBumps) -> Result<()> {
        args.params.validate(None)?;
        let now = Clock::get()?.unix_timestamp;
        let config = &mut self.config;
        config.admin = args.admin;
        config.pending_admin = Pubkey::default();
        config.guardian = args.guardian;
        config.keeper = args.keeper;
        config.reporter = args.reporter;
        config.usdq_mint = self.usdq_mint.key();
        config.susdq_mint = self.susdq_mint.key();
        config.usdc_mint = self.usdc_mint.key();
        // Every path waits for the admin's unpause once the vaults and collateral are in place.
        config.paused = path::ALL;
        config.params = args.params;
        config.usdc_bucket = Bucket::full(args.params.usdc_hourly_cap, now);
        config.bumps = Bumps {
            config: bumps.config,
            reserve: bumps.reserve,
            usdq_vault: bumps.usdq_vault,
            aequa_pool: bumps.aequa_pool,
            usdq_mint: bumps.usdq_mint,
            susdq_mint: bumps.susdq_mint,
            ..Default::default()
        };
        self.reserve.bump = bumps.reserve;
        self.reserve.last_distribution = now;
        self.usdq_vault.bump = bumps.usdq_vault;
        self.usdq_vault.susdq_mint = self.susdq_mint.key();
        self.aequa_pool.bump = bumps.aequa_pool;
        Ok(())
    }
}

/// Creates the buffer, the Reserve Fund and the protocol vault, each at its PDA and its own
/// authority, and records the landing and USDC transit accounts. sUSDq's accounts follow in
/// `create_susdq_vaults` (one instruction would run past the stack).
#[derive(Accounts)]
pub struct CreateVaults<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config, has_one = admin @ AequaError::Unauthorized, has_one = usdq_mint, has_one = usdc_mint)]
    pub config: Box<Account<'info, Config>>,
    pub admin: Signer<'info>,
    /// CHECK: Config's USDq mint (`has_one`); the token accounts below are created for it.
    pub usdq_mint: UncheckedAccount<'info>,
    /// CHECK: Config's USDC mint (`has_one`).
    pub usdc_mint: UncheckedAccount<'info>,
    #[account(init, payer = payer, seeds = [BUFFER], bump, token::mint = usdc_mint, token::authority = buffer, token::token_program = token_program)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    /// CHECK: the landing account's authority: a PDA that holds no data and signs only for it.
    #[account(seeds = [LANDING], bump)]
    pub landing_authority: UncheckedAccount<'info>,
    /// USDC from HyperCore lands here: the associated token account of `landing_authority`,
    /// the form Circle's forwarding service mints into. Anyone can create an associated
    /// account, so the setup creates it (idempotently) and this only checks it.
    #[account(associated_token::mint = usdc_mint, associated_token::authority = landing_authority, associated_token::token_program = token_program)]
    pub landing: Box<Account<'info, TokenAccount>>,
    /// CHECK: the swap authority PDA; holds no data.
    #[account(seeds = [SWAP_AUTHORITY], bump)]
    pub swap_authority: UncheckedAccount<'info>,
    /// The swap authority's USDC transit account, created by the setup like `landing`.
    #[account(associated_token::mint = usdc_mint, associated_token::authority = swap_authority, associated_token::token_program = token_program)]
    pub transit_usdc: Box<Account<'info, TokenAccount>>,
    #[account(init, payer = payer, seeds = [RESERVE_FUND], bump, token::mint = usdq_mint, token::authority = reserve_fund, token::token_program = token_program)]
    pub reserve_fund: Box<Account<'info, TokenAccount>>,
    #[account(init, payer = payer, seeds = [PROTOCOL], bump, token::mint = usdq_mint, token::authority = protocol, token::token_program = token_program)]
    pub protocol: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl CreateVaults<'_> {
    pub fn run(&mut self, bumps: &CreateVaultsBumps) -> Result<()> {
        let b = &mut self.config.bumps;
        b.buffer = bumps.buffer;
        b.landing = bumps.landing_authority;
        b.swap_authority = bumps.swap_authority;
        b.reserve_fund = bumps.reserve_fund;
        b.protocol = bumps.protocol;
        self.config.transit_usdc = self.transit_usdc.key();
        Ok(())
    }
}

/// Creates sUSDq's USDq accounts: the assets behind sUSDq and the silo cooling USDq waits in.
#[derive(Accounts)]
pub struct CreateSusdqVaults<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config, has_one = admin @ AequaError::Unauthorized, has_one = usdq_mint)]
    pub config: Box<Account<'info, Config>>,
    pub admin: Signer<'info>,
    /// CHECK: Config's USDq mint (`has_one`).
    pub usdq_mint: UncheckedAccount<'info>,
    #[account(init, payer = payer, seeds = [SUSDQ_ASSETS], bump, token::mint = usdq_mint, token::authority = susdq_assets, token::token_program = token_program)]
    pub susdq_assets: Box<Account<'info, TokenAccount>>,
    #[account(init, payer = payer, seeds = [SUSDQ_SILO], bump, token::mint = usdq_mint, token::authority = susdq_silo, token::token_program = token_program)]
    pub susdq_silo: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl CreateSusdqVaults<'_> {
    pub fn run(&mut self, bumps: &CreateSusdqVaultsBumps) -> Result<()> {
        let b = &mut self.config.bumps;
        b.susdq_assets = bumps.susdq_assets;
        b.susdq_silo = bumps.susdq_silo;
        Ok(())
    }
}

/// `set_bridge_route`'s arguments: the hedge account on HyperCore and the fee cap.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct BridgeArgs {
    pub recipient: [u8; 20],
    pub max_fee_bps: u16,
    pub max_fee_flat: u64,
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config, has_one = admin @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub admin: Signer<'info>,
}

impl AdminOnly<'_> {
    pub fn set_params(&mut self, params: Params) -> Result<()> {
        let config = &mut self.config;
        params.validate(Some(&config.params))?;
        let now = Clock::get()?.unix_timestamp;
        // The bucket keeps its level across a cap change.
        let old_cap = config.params.usdc_hourly_cap;
        config.usdc_bucket.settle(old_cap, now);
        config.usdc_bucket.available = config.usdc_bucket.available.min(params.usdc_hourly_cap);
        config.params = params;
        emit!(ParamsChanged { by: self.admin.key() });
        Ok(())
    }

    /// Sets the operating roles. A new admin takes over only once it accepts.
    pub fn set_roles(&mut self, guardian: Pubkey, keeper: Pubkey, reporter: Pubkey, pending_admin: Pubkey) -> Result<()> {
        let config = &mut self.config;
        config.guardian = guardian;
        config.keeper = keeper;
        config.reporter = reporter;
        config.pending_admin = pending_admin;
        emit!(ParamsChanged { by: self.admin.key() });
        Ok(())
    }

    /// Names the hedge account bridged margin goes to; the rest of the route is fixed.
    pub fn set_bridge_route(&mut self, args: BridgeArgs) -> Result<()> {
        require!(args.recipient != [0u8; 20], AequaError::ParamOutOfRange);
        // A fee cap of at most 10 bps and $5 a transfer; Circle's fast fee is 1 bps and its
        // forwarding fee about $0.25.
        require!(args.max_fee_bps <= 10 && args.max_fee_flat <= 5_000_000, AequaError::ParamOutOfRange);
        self.config.bridge = BridgeRoute::to_hypercore(args.recipient, args.max_fee_bps, args.max_fee_flat);
        emit!(ParamsChanged { by: self.admin.key() });
        Ok(())
    }

    pub fn unpause(&mut self, paths: u32) -> Result<()> {
        self.config.paused &= !paths;
        emit!(PathsPaused { by: self.admin.key(), paused: self.config.paused, collateral: None });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config, constraint = config.pending_admin == pending_admin.key() @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub pending_admin: Signer<'info>,
}

impl AcceptAdmin<'_> {
    pub fn run(&mut self) -> Result<()> {
        self.config.admin = self.pending_admin.key();
        self.config.pending_admin = Pubkey::default();
        emit!(ParamsChanged { by: self.pending_admin.key() });
        Ok(())
    }
}

/// The guardian (or the admin) stops paths; only the admin starts them again.
#[derive(Accounts)]
pub struct Pause<'info> {
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config)]
    pub config: Box<Account<'info, Config>>,
    #[account(constraint = signer.key() == config.guardian || signer.key() == config.admin @ AequaError::Unauthorized)]
    pub signer: Signer<'info>,
    #[account(mut, seeds = [COLLATERAL, collateral.mint.as_ref()], bump = collateral.bump)]
    pub collateral: Option<Box<Account<'info, Collateral>>>,
}

impl Pause<'_> {
    pub fn run(&mut self, paths: u32) -> Result<()> {
        self.config.paused |= paths & path::ALL;
        let mut which = None;
        if let Some(c) = self.collateral.as_mut() {
            c.paused = true;
            which = Some(c.mint);
        }
        emit!(PathsPaused { by: self.signer.key(), paused: self.config.paused, collateral: which });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct UnpauseCollateral<'info> {
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = admin @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub admin: Signer<'info>,
    #[account(mut, seeds = [COLLATERAL, collateral.mint.as_ref()], bump = collateral.bump)]
    pub collateral: Box<Account<'info, Collateral>>,
}

impl UnpauseCollateral<'_> {
    pub fn run(&mut self) -> Result<()> {
        self.collateral.paused = false;
        emit!(PathsPaused { by: self.admin.key(), paused: self.config.paused, collateral: Some(self.collateral.mint) });
        Ok(())
    }
}
