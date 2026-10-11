//! The USDC paths: USDq 1:1 less the fee, always open while unpaused. Minting draws
//! on the hourly bucket; redeeming is bounded by the buffer.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, MintTo, Token, TokenAccount, Transfer, TransferChecked};

use crate::constants::seeds::*;
use crate::errors::AequaError;
use crate::events::{Minted, Redeemed, TO_PROTOCOL, TO_RESERVE_FUND};
use crate::instructions::common::{fee_bps, fee_to_reserve_fund};
use crate::math;
use crate::signer;
use crate::state::{path, AequaStake, Config};

#[derive(Accounts)]
pub struct MintWithUsdc<'info> {
    pub user: Signer<'info>,
    #[account(mut, seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint, has_one = usdc_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut)]
    pub usdq_mint: Box<Account<'info, Mint>>,
    pub usdc_mint: Box<Account<'info, Mint>>,
    #[account(mut, token::mint = usdc_mint, token::authority = user)]
    pub user_usdc: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = usdq_mint)]
    pub user_usdq: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [BUFFER], bump = config.bumps.buffer)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [RESERVE_FUND], bump = config.bumps.reserve_fund)]
    pub reserve_fund: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [PROTOCOL], bump = config.bumps.protocol)]
    pub protocol: Box<Account<'info, TokenAccount>>,
    #[account(seeds = [AEQUA_STAKE, user.key().as_ref()], bump = aequa_stake.bump)]
    pub aequa_stake: Option<Box<Account<'info, AequaStake>>>,
    /// CHECK: the stAEQUA pool, by its seeds; the fee tier reads its head (`PoolHead`).
    #[account(seeds = [AEQUA_POOL], bump = config.bumps.aequa_pool)]
    pub aequa_pool: Option<UncheckedAccount<'info>>,
    pub token_program: Program<'info, Token>,
}

impl MintWithUsdc<'_> {
    pub fn run(&mut self, amount: u64, min_out: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &mut self.config;
        config.require_open(path::MINT_USDC)?;
        require!(amount > 0, AequaError::ZeroAmount);
        require!(self.user_usdc.amount >= amount, AequaError::InsufficientFunds);

        let supply = self.usdq_mint.supply.checked_add(amount).ok_or(AequaError::MathOverflow)?;
        require!(supply <= config.supply_cap(), AequaError::SupplyCapExceeded);
        let cap = config.params.usdc_hourly_cap;
        config.usdc_bucket.draw(amount, cap, now)?;

        let fee_bps = fee_bps(config, self.aequa_stake.as_ref().map(|a| &***a), self.aequa_pool.as_ref().map(|a| a.as_ref()), &self.user.key(), now)?;
        let (out, fee) = math::after_fee(amount, fee_bps);
        require!(out > 0, AequaError::ZeroAmount);
        require!(out >= min_out, AequaError::SlippageExceeded);
        let to_fund = fee_to_reserve_fund(config, self.reserve_fund.amount, supply);

        let program = self.token_program.key();
        token::transfer_checked(
            CpiContext::new(
                program,
                TransferChecked {
                    from: self.user_usdc.to_account_info(),
                    mint: self.usdc_mint.to_account_info(),
                    to: self.buffer.to_account_info(),
                    authority: self.user.to_account_info(),
                },
            ),
            amount,
            6,
        )?;
        let config_bump = [config.bumps.config];
        let seeds: &[&[&[u8]]] = &[&[CONFIG, &config_bump]];
        token::mint_to(
            CpiContext::new_with_signer(
                program,
                MintTo { mint: self.usdq_mint.to_account_info(), to: self.user_usdq.to_account_info(), authority: config.to_account_info() },
                seeds,
            ),
            out,
        )?;
        if fee > 0 {
            let to = if to_fund { self.reserve_fund.to_account_info() } else { self.protocol.to_account_info() };
            token::mint_to(
                CpiContext::new_with_signer(program, MintTo { mint: self.usdq_mint.to_account_info(), to, authority: config.to_account_info() }, seeds),
                fee,
            )?;
        }
        emit!(Minted {
            user: self.user.key(),
            asset: self.usdc_mint.key(),
            amount_in: amount,
            usdq_out: out,
            fee,
            fee_to: if to_fund { TO_RESERVE_FUND } else { TO_PROTOCOL },
            fee_bps,
            price_e8: 0,
            session: 0,
        });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct RedeemForUsdc<'info> {
    pub user: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint, has_one = usdc_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut)]
    pub usdq_mint: Box<Account<'info, Mint>>,
    pub usdc_mint: Box<Account<'info, Mint>>,
    #[account(mut, token::mint = usdq_mint, token::authority = user)]
    pub user_usdq: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = usdc_mint)]
    pub user_usdc: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [BUFFER], bump = config.bumps.buffer)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [RESERVE_FUND], bump = config.bumps.reserve_fund)]
    pub reserve_fund: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [PROTOCOL], bump = config.bumps.protocol)]
    pub protocol: Box<Account<'info, TokenAccount>>,
    #[account(seeds = [AEQUA_STAKE, user.key().as_ref()], bump = aequa_stake.bump)]
    pub aequa_stake: Option<Box<Account<'info, AequaStake>>>,
    /// CHECK: the stAEQUA pool, by its seeds; the fee tier reads its head (`PoolHead`).
    #[account(seeds = [AEQUA_POOL], bump = config.bumps.aequa_pool)]
    pub aequa_pool: Option<UncheckedAccount<'info>>,
    pub token_program: Program<'info, Token>,
}

impl RedeemForUsdc<'_> {
    pub fn run(&mut self, amount: u64, min_out: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        config.require_open(path::REDEEM_USDC)?;
        require!(amount > 0, AequaError::ZeroAmount);
        require!(self.user_usdq.amount >= amount, AequaError::InsufficientFunds);

        let fee_bps = fee_bps(config, self.aequa_stake.as_ref().map(|a| &***a), self.aequa_pool.as_ref().map(|a| a.as_ref()), &self.user.key(), now)?;
        let (out, fee) = math::after_fee(amount, fee_bps);
        require!(out > 0, AequaError::ZeroAmount);
        require!(out <= self.buffer.amount, AequaError::BufferInsufficient);
        require!(out >= min_out, AequaError::SlippageExceeded);
        let supply = self.usdq_mint.supply - out;
        let to_fund = fee_to_reserve_fund(config, self.reserve_fund.amount, supply);

        let program = self.token_program.key();
        token::burn(
            CpiContext::new(
                program,
                Burn { mint: self.usdq_mint.to_account_info(), from: self.user_usdq.to_account_info(), authority: self.user.to_account_info() },
            ),
            out,
        )?;
        if fee > 0 {
            let to = if to_fund { self.reserve_fund.to_account_info() } else { self.protocol.to_account_info() };
            token::transfer(
                CpiContext::new(program, Transfer { from: self.user_usdq.to_account_info(), to, authority: self.user.to_account_info() }),
                fee,
            )?;
        }
        let bump = config.bumps.buffer;
        token::transfer_checked(
            CpiContext::new_with_signer(
                program,
                TransferChecked {
                    from: self.buffer.to_account_info(),
                    mint: self.usdc_mint.to_account_info(),
                    to: self.user_usdc.to_account_info(),
                    authority: self.buffer.to_account_info(),
                },
                signer!(BUFFER, bump),
            ),
            out,
            6,
        )?;
        emit!(Redeemed {
            user: self.user.key(),
            asset: self.usdc_mint.key(),
            usdq_in: amount,
            amount_out: out,
            fee,
            fee_to: if to_fund { TO_RESERVE_FUND } else { TO_PROTOCOL },
            fee_bps,
            price_e8: 0,
            session: 0,
        });
        Ok(())
    }
}
