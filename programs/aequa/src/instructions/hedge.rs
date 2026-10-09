//! The hedge account's side of the reserve: the reporter's figures and USDC landing
//! back from HyperCore.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

use crate::constants::{seeds::*, MAX_COLLATERALS};
use crate::errors::AequaError;
use crate::signer;
use crate::state::{path, Config, HedgeReport, Reserve};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct ReportArgs {
    pub account_value: i64,
    pub short_notional: [u64; MAX_COLLATERALS],
    pub market_live: u8,
    pub credited_total: u64,
    pub withdrawn_total: u64,
    pub hl_time: i64,
}

#[derive(Accounts)]
pub struct ReportHedge<'info> {
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = reporter @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub reporter: Signer<'info>,
    #[account(mut, seeds = [RESERVE], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
}

impl ReportHedge<'_> {
    pub fn run(&mut self, args: ReportArgs) -> Result<()> {
        self.config.require_open(path::REPORT)?;
        let now = Clock::get()?.unix_timestamp;
        // Reports land in HyperCore's order; an older one never replaces a newer one.
        require!(args.hl_time > self.reserve.report.hl_time, AequaError::ReportOutOfOrder);
        // Markets past the listed collateral carry nothing.
        let count = self.config.collateral_count as usize;
        let mut short = args.short_notional;
        for v in short.iter_mut().skip(count) {
            *v = 0;
        }
        let live_mask = if count >= 8 { u8::MAX } else { (1u8 << count) - 1 };
        self.reserve.report = HedgeReport {
            account_value: args.account_value,
            short_notional: short,
            market_live: args.market_live & live_mask,
            credited_total: args.credited_total,
            withdrawn_total: args.withdrawn_total,
            hl_time: args.hl_time,
            reported_at: now,
        };
        Ok(())
    }
}

/// Moves USDC that CCTP minted into the landing account over to the buffer. Anyone may call it.
#[derive(Accounts)]
pub struct LandUsdc<'info> {
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdc_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [RESERVE], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
    pub usdc_mint: Box<Account<'info, Mint>>,
    /// CHECK: the landing account's authority PDA.
    #[account(seeds = [LANDING], bump = config.bumps.landing)]
    pub landing_authority: UncheckedAccount<'info>,
    #[account(mut, associated_token::mint = usdc_mint, associated_token::authority = landing_authority, associated_token::token_program = token_program)]
    pub landing: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [BUFFER], bump = config.bumps.buffer)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl LandUsdc<'_> {
    pub fn run(&mut self) -> Result<()> {
        self.config.require_open(path::LAND)?;
        let amount = self.landing.amount;
        require!(amount > 0, AequaError::NothingToMove);
        let bump = self.config.bumps.landing;
        token::transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.key(),
                TransferChecked {
                    from: self.landing.to_account_info(),
                    mint: self.usdc_mint.to_account_info(),
                    to: self.buffer.to_account_info(),
                    authority: self.landing_authority.to_account_info(),
                },
                signer!(LANDING, bump),
            ),
            amount,
            6,
        )?;
        let reserve = &mut self.reserve;
        reserve.landed_total = reserve.landed_total.checked_add(amount).ok_or(AequaError::MathOverflow)?;
        Ok(())
    }
}
