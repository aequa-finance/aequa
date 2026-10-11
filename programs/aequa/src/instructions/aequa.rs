//! AEQUA and stAEQUA: the token's setup once it exists, and non-transferable
//! positions over the stake pool. AEQUA per share rises as buyback rewards stream in and falls
//! when the pool covers a deficit; tokens sent straight to the vault don't count.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::constants::{seeds::*, AEQUA_WINDOW_SECS};
use crate::errors::AequaError;
use crate::math;
use crate::signer;
use crate::state::{path, AequaPool, AequaSetup, AequaStake, Config, StakeState, Twap};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct AequaArgs {
    /// The designated pool and its two reserve accounts.
    pub pool: Pubkey,
    pub pool_aequa_vault: Pubkey,
    pub pool_quote_vault: Pubkey,
    /// Zero when the pool's quote asset is a dollar; otherwise its Pyth Pro feed.
    pub quote_feed: u32,
}

/// Sets the AEQUA mint once it exists and creates the pool's vaults.
#[derive(Accounts)]
pub struct SetAequa<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config, has_one = admin @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub admin: Signer<'info>,
    #[account(mint::token_program = token_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mint::token_program = quote_token_program)]
    pub quote_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(token::mint = mint, token::token_program = token_program)]
    pub pool_aequa_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(token::mint = quote_mint, token::token_program = quote_token_program)]
    pub pool_quote_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = payer,
        seeds = [AEQUA_POOL_VAULT],
        bump,
        token::mint = mint,
        token::authority = aequa_pool_vault,
        token::token_program = token_program,
    )]
    pub aequa_pool_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = payer,
        seeds = [SLASH_SALE],
        bump,
        token::mint = mint,
        token::authority = slash_sale,
        token::token_program = token_program,
    )]
    pub slash_sale: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(init_if_needed, payer = payer, space = 8 + Twap::INIT_SPACE, seeds = [TWAP], bump)]
    pub twap: Box<Account<'info, Twap>>,
    /// CHECK: the swap authority PDA.
    #[account(seeds = [SWAP_AUTHORITY], bump = config.bumps.swap_authority)]
    pub swap_authority: UncheckedAccount<'info>,
    /// The transit account buybacks and slash sales pass through.
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = mint,
        associated_token::authority = swap_authority,
        associated_token::token_program = token_program,
    )]
    pub transit_aequa: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub quote_token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl SetAequa<'_> {
    pub fn run(&mut self, args: AequaArgs, bumps: &SetAequaBumps) -> Result<()> {
        let config = &mut self.config;
        // The mint can be set once; the pool can move by a later call with the same mint.
        require!(!config.aequa.is_set() || config.aequa.mint == self.mint.key(), AequaError::ParamOutOfRange);
        require!(args.pool_aequa_vault == self.pool_aequa_vault.key(), AequaError::ParamOutOfRange);
        require!(args.pool_quote_vault == self.pool_quote_vault.key(), AequaError::ParamOutOfRange);
        config.aequa = AequaSetup {
            mint: self.mint.key(),
            token_program: self.token_program.key(),
            decimals: self.mint.decimals,
            transit: self.transit_aequa.key(),
            pool: args.pool,
            pool_aequa_vault: args.pool_aequa_vault,
            pool_quote_vault: args.pool_quote_vault,
            quote_mint: self.quote_mint.key(),
            quote_decimals: self.quote_mint.decimals,
            quote_feed: args.quote_feed,
        };
        config.bumps.aequa_pool_vault = bumps.aequa_pool_vault;
        config.bumps.slash_sale = bumps.slash_sale;
        config.bumps.twap = bumps.twap;
        // A new pool starts a new price history.
        let twap = &mut self.twap;
        if twap.pool != args.pool {
            twap.pool = args.pool;
            twap.head = 0;
            twap.count = 0;
            twap.last_price = 0;
        }
        twap.bump = bumps.twap;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct StakeAequa<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [AEQUA_POOL], bump = aequa_pool.bump)]
    pub aequa_pool: Box<Account<'info, AequaPool>>,
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + AequaStake::INIT_SPACE,
        seeds = [AEQUA_STAKE, user.key().as_ref()],
        bump,
    )]
    pub aequa_stake: Box<Account<'info, AequaStake>>,
    #[account(address = config.aequa.mint @ AequaError::AequaNotSet)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, token::mint = mint, token::authority = user, token::token_program = token_program)]
    pub user_aequa: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, seeds = [AEQUA_POOL_VAULT], bump = config.bumps.aequa_pool_vault)]
    pub aequa_pool_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = config.aequa.token_program)]
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl StakeAequa<'_> {
    pub fn run(&mut self, amount: u64, min_shares: u64, bump: u8) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        config.require_open(path::STAKE_AEQUA)?;
        require!(config.aequa.is_set(), AequaError::AequaNotSet);
        require!(amount > 0, AequaError::ZeroAmount);
        require!(self.user_aequa.amount >= amount, AequaError::InsufficientFunds);

        let stake = &mut self.aequa_stake;
        let cooldown = config.params.aequa_cooldown_secs as i64;
        match stake.state(now, cooldown, AEQUA_WINDOW_SECS) {
            StakeState::Cooling | StakeState::Window => return err!(AequaError::PositionCooling),
            StakeState::Empty | StakeState::Active => {}
        }
        let pool = &mut self.aequa_pool;
        pool.fold(now);
        let shares = math::shares_for(amount, pool.base, pool.total_shares);
        require!(shares > 0, AequaError::ZeroAmount);
        require!(shares >= min_shares, AequaError::SlippageExceeded);

        token_interface::transfer_checked(
            CpiContext::new(
                self.token_program.key(),
                TransferChecked {
                    from: self.user_aequa.to_account_info(),
                    mint: self.mint.to_account_info(),
                    to: self.aequa_pool_vault.to_account_info(),
                    authority: self.user.to_account_info(),
                },
            ),
            amount,
            self.mint.decimals,
        )?;
        pool.base = pool.base.checked_add(amount).ok_or(AequaError::MathOverflow)?;
        pool.total_shares = pool.total_shares.checked_add(shares).ok_or(AequaError::MathOverflow)?;
        stake.owner = self.user.key();
        stake.bump = bump;
        stake.shares += shares;
        // A position whose window passed is active again; staking more clears the old start.
        stake.cooldown_start = 0;
        Ok(())
    }
}

/// Starts the cooldown for the whole position.
#[derive(Accounts)]
pub struct StartAequaUnstake<'info> {
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [AEQUA_STAKE, user.key().as_ref()], bump = aequa_stake.bump)]
    pub aequa_stake: Box<Account<'info, AequaStake>>,
}

impl StartAequaUnstake<'_> {
    pub fn run(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        config.require_open(path::UNSTAKE_AEQUA)?;
        let cooldown = config.params.aequa_cooldown_secs as i64;
        let stake = &mut self.aequa_stake;
        match stake.state(now, cooldown, AEQUA_WINDOW_SECS) {
            StakeState::Empty => return err!(AequaError::NothingStaked),
            StakeState::Cooling | StakeState::Window => return err!(AequaError::PositionCooling),
            StakeState::Active => {}
        }
        stake.cooldown_start = now;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct WithdrawAequa<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [AEQUA_POOL], bump = aequa_pool.bump)]
    pub aequa_pool: Box<Account<'info, AequaPool>>,
    #[account(mut, close = user, seeds = [AEQUA_STAKE, user.key().as_ref()], bump = aequa_stake.bump)]
    pub aequa_stake: Box<Account<'info, AequaStake>>,
    #[account(address = config.aequa.mint @ AequaError::AequaNotSet)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(mut, token::mint = mint, token::token_program = token_program)]
    pub user_aequa: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, seeds = [AEQUA_POOL_VAULT], bump = config.bumps.aequa_pool_vault)]
    pub aequa_pool_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = config.aequa.token_program)]
    pub token_program: Interface<'info, TokenInterface>,
}

impl WithdrawAequa<'_> {
    pub fn run(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        config.require_open(path::UNSTAKE_AEQUA)?;
        let cooldown = config.params.aequa_cooldown_secs as i64;
        let stake = &self.aequa_stake;
        require!(stake.state(now, cooldown, AEQUA_WINDOW_SECS) == StakeState::Window, AequaError::WindowNotOpen);

        let pool = &mut self.aequa_pool;
        pool.fold(now);
        let amount = math::assets_for(stake.shares, pool.base, pool.total_shares);
        pool.base -= amount;
        pool.total_shares -= stake.shares;
        let bump = config.bumps.aequa_pool_vault;
        if amount > 0 {
            token_interface::transfer_checked(
                CpiContext::new_with_signer(
                    self.token_program.key(),
                    TransferChecked {
                        from: self.aequa_pool_vault.to_account_info(),
                        mint: self.mint.to_account_info(),
                        to: self.user_aequa.to_account_info(),
                        authority: self.aequa_pool_vault.to_account_info(),
                    },
                    signer!(AEQUA_POOL_VAULT, bump),
                ),
                amount,
                self.mint.decimals,
            )?;
        }
        Ok(())
    }
}
