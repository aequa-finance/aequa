//! The AEQUA buyback and the slash sale, both through Jupiter and both
//! bounded by the designated pool's 30-minute TWAP.
//!
//! `buyback` spends at most `buyback_max_chunk` of the protocol vault an hour, however the hour
//! is split across calls (a bucket that refills continuously). Its USDq is retired at par
//! against buffer USDC, which buys AEQUA; half the AEQUA bought is burned and half streams to
//! stAEQUA over 7 days, in the same transaction. The fill must be within 1% of the TWAP.
//!
//! `sell_slashed` sells AEQUA from the slash-sale vault for USDC, at most `buyback_max_chunk`
//! dollars of it an hour at the TWAP, each fill within 5% of the TWAP. The USDC joins the
//! buffer, which closes the shortfall; whatever the sale raises beyond the shortfall becomes
//! USDq for the Reserve Fund, backed by that USDC.

use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Burn, Mint, MintTo, Token, TokenAccount, TransferChecked},
    token_interface::{self, Burn as ABurn, Mint as AMint, TokenAccount as AAccount, TokenInterface, TransferChecked as ATransferChecked},
};

use crate::constants::{seeds::*, BPS};
use crate::errors::AequaError;
use crate::instructions::deficit::aequa_price_usd;
use crate::instructions::pyth_pro::*;
use crate::jupiter;
use crate::math::U256;
use crate::state::{path, AequaPool, Config, Reserve, Twap};

/// Dollars per whole AEQUA at 1e12 for a fill of `usd` (6 decimals) against `aequa` base units.
fn fill_price(usd: u64, aequa: u64, decimals: u8) -> Result<u128> {
    require!(aequa > 0, AequaError::TwapBound);
    let v = U256::from(usd) * U256::from(10u64).pow(U256::from(decimals as u32)) * U256::from(1_000_000_000_000u64) / (U256::from(aequa) * U256::from(1_000_000u64));
    require!(v <= U256::from(u128::MAX), AequaError::MathOverflow);
    Ok(v.as_u128())
}

/// Dollars (6 decimals) of `aequa` base units at `price_e12` dollars per whole AEQUA, rounded up.
fn usd_at(aequa: u64, price_e12: u128, decimals: u8) -> Result<u64> {
    let n = U256::from(aequa) * U256::from(price_e12) * U256::from(1_000_000u64);
    let d = U256::from(10u64).pow(U256::from(decimals as u32)) * U256::from(1_000_000_000_000u64);
    let v = (n + d - U256::from(1u8)) / d;
    require!(v <= U256::from(u64::MAX), AequaError::MathOverflow);
    Ok(v.as_u64())
}

/// The AEQUA side of a Jupiter swap: the mint, the swap authority and its two transit
/// accounts. Checked against Config by `AequaSwap::check`.
#[derive(Accounts)]
pub struct AequaSwap<'info> {
    #[account(mut)]
    pub aequa_mint: Box<InterfaceAccount<'info, AMint>>,
    pub aequa_token_program: Interface<'info, TokenInterface>,
    /// CHECK: the swap authority PDA, checked in `check`.
    pub swap_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub transit_usdc: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub transit_aequa: Box<InterfaceAccount<'info, AAccount>>,
    /// CHECK: Jupiter Aggregator v6, by address in `jupiter::swap`.
    pub jupiter_program: UncheckedAccount<'info>,
}

impl AequaSwap<'_> {
    pub fn check(&self, config: &Config) -> Result<()> {
        let a = &config.aequa;
        require!(a.is_set(), AequaError::AequaNotSet);
        require_keys_eq!(self.aequa_mint.key(), a.mint, AequaError::AequaNotSet);
        require_keys_eq!(self.aequa_token_program.key(), a.token_program, AequaError::AequaNotSet);
        require_keys_eq!(self.transit_aequa.key(), a.transit, AequaError::AequaNotSet);
        require_keys_eq!(self.transit_usdc.key(), config.transit_usdc, AequaError::InvalidRoute);
        let authority = Pubkey::create_program_address(&[SWAP_AUTHORITY, &[config.bumps.swap_authority]], &crate::ID).map_err(|_| error!(AequaError::InvalidRoute))?;
        require_keys_eq!(self.swap_authority.key(), authority, AequaError::InvalidRoute);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Buyback<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint, has_one = usdc_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [AEQUA_POOL], bump = aequa_pool.bump)]
    pub aequa_pool: Box<Account<'info, AequaPool>>,
    #[account(seeds = [TWAP], bump = twap.bump)]
    pub twap: Box<Account<'info, Twap>>,
    #[account(mut)]
    pub usdq_mint: Box<Account<'info, Mint>>,
    pub usdc_mint: Box<Account<'info, Mint>>,
    #[account(mut, seeds = [PROTOCOL], bump = config.bumps.protocol)]
    pub protocol: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [BUFFER], bump = config.bumps.buffer)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [AEQUA_POOL_VAULT], bump = config.bumps.aequa_pool_vault)]
    pub aequa_pool_vault: Box<InterfaceAccount<'info, AAccount>>,
    pub token_program: Program<'info, Token>,
    pub swap: AequaSwap<'info>,
    pub pyth: PythPro<'info>,
}

impl<'info> Buyback<'info> {
    pub fn run(&mut self, message: &[u8], ed25519_index: u16, amount: u64, route: &[u8], remaining: &[AccountInfo<'info>]) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        let p = config.params;
        config.require_open(path::BUYBACK)?;
        self.swap.check(config)?;
        require!(self.twap.pool == config.aequa.pool, AequaError::AequaNotSet);
        require!(amount > 0, AequaError::ZeroAmount);
        require!(amount <= self.protocol.amount, AequaError::InsufficientFunds);
        require!(amount <= self.buffer.amount, AequaError::BufferInsufficient);
        self.aequa_pool.buyback_bucket.draw(amount, p.buyback_max_chunk, now)?;
        let twap = aequa_price_usd(config, &self.twap, &self.pyth, &self.caller.to_account_info(), message, ed25519_index, now)?;

        // The protocol vault's USDq retires at par; the same dollars leave the buffer as USDC.
        let usdq_program = self.token_program.key();
        let (protocol_bump, buffer_bump, authority_bump) = ([config.bumps.protocol], [config.bumps.buffer], [config.bumps.swap_authority]);
        token::burn(
            CpiContext::new_with_signer(
                usdq_program,
                Burn { mint: self.usdq_mint.to_account_info(), from: self.protocol.to_account_info(), authority: self.protocol.to_account_info() },
                &[&[PROTOCOL, &protocol_bump]],
            ),
            amount,
        )?;
        token::transfer_checked(
            CpiContext::new_with_signer(
                usdq_program,
                TransferChecked {
                    from: self.buffer.to_account_info(),
                    mint: self.usdc_mint.to_account_info(),
                    to: self.swap.transit_usdc.to_account_info(),
                    authority: self.buffer.to_account_info(),
                },
                &[&[BUFFER, &buffer_bump]],
            ),
            amount,
            6,
        )?;
        self.swap.transit_usdc.reload()?;
        let (usdc_before, aequa_before) = (self.swap.transit_usdc.amount, self.swap.transit_aequa.amount);
        let authority_seeds: &[&[u8]] = &[SWAP_AUTHORITY, &authority_bump];
        jupiter::swap(&self.swap.jupiter_program.to_account_info(), remaining, route, amount, &self.swap.swap_authority.key(), authority_seeds)?;
        self.swap.transit_usdc.reload()?;
        self.swap.transit_aequa.reload()?;
        require!(usdc_before - self.swap.transit_usdc.amount == amount, AequaError::SwapOverspent);
        let bought = self.swap.transit_aequa.amount.checked_sub(aequa_before).ok_or(AequaError::TwapBound)?;

        // The fill must be within the bound above the TWAP.
        let decimals = self.swap.aequa_mint.decimals;
        let fill = fill_price(amount, bought, decimals)?;
        require!(fill <= twap * (BPS + p.buyback_twap_bound_bps as u64) as u128 / BPS as u128, AequaError::TwapBound);

        // Half burned, half streamed to stakers; with no stakers yet, all of it is burned.
        let pool = &mut self.aequa_pool;
        let streamed = if pool.total_shares == 0 { 0 } else { bought - (bought as u128 * p.buyback_burn_bps as u128 / BPS as u128) as u64 };
        let burned = bought - streamed;
        let a_program = self.swap.aequa_token_program.key();
        if burned > 0 {
            token_interface::burn(
                CpiContext::new_with_signer(
                    a_program,
                    ABurn { mint: self.swap.aequa_mint.to_account_info(), from: self.swap.transit_aequa.to_account_info(), authority: self.swap.swap_authority.to_account_info() },
                    &[authority_seeds],
                ),
                burned,
            )?;
        }
        if streamed > 0 {
            token_interface::transfer_checked(
                CpiContext::new_with_signer(
                    a_program,
                    ATransferChecked {
                        from: self.swap.transit_aequa.to_account_info(),
                        mint: self.swap.aequa_mint.to_account_info(),
                        to: self.aequa_pool_vault.to_account_info(),
                        authority: self.swap.swap_authority.to_account_info(),
                    },
                    &[authority_seeds],
                ),
                streamed,
                decimals,
            )?;
            let done = pool.stream.restart(now, streamed, p.aequa_stream_secs as i64)?;
            pool.base += done;
        }
        pool.usdq_spent = pool.usdq_spent.saturating_add(amount);
        pool.aequa_bought = pool.aequa_bought.saturating_add(bought);
        pool.aequa_burned = pool.aequa_burned.saturating_add(burned);
        pool.aequa_streamed = pool.aequa_streamed.saturating_add(streamed);
        pool.buybacks += 1;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct SellSlashed<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = usdq_mint, has_one = usdc_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [RESERVE], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
    #[account(mut, seeds = [AEQUA_POOL], bump = aequa_pool.bump)]
    pub aequa_pool: Box<Account<'info, AequaPool>>,
    #[account(seeds = [TWAP], bump = twap.bump)]
    pub twap: Box<Account<'info, Twap>>,
    #[account(mut)]
    pub usdq_mint: Box<Account<'info, Mint>>,
    pub usdc_mint: Box<Account<'info, Mint>>,
    #[account(mut, seeds = [SLASH_SALE], bump = config.bumps.slash_sale)]
    pub slash_sale: Box<InterfaceAccount<'info, AAccount>>,
    #[account(mut, seeds = [BUFFER], bump = config.bumps.buffer)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    #[account(mut, seeds = [RESERVE_FUND], bump = config.bumps.reserve_fund)]
    pub reserve_fund: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub swap: AequaSwap<'info>,
    pub pyth: PythPro<'info>,
}

impl<'info> SellSlashed<'info> {
    pub fn run(&mut self, message: &[u8], ed25519_index: u16, amount: u64, route: &[u8], remaining: &[AccountInfo<'info>]) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        let p = config.params;
        config.require_open(path::COVER_DEFICIT)?;
        self.swap.check(config)?;
        require!(self.twap.pool == config.aequa.pool, AequaError::AequaNotSet);
        require!(amount > 0 && amount <= self.slash_sale.amount, AequaError::NothingToMove);
        let twap = aequa_price_usd(config, &self.twap, &self.pyth, &self.caller.to_account_info(), message, ed25519_index, now)?;

        let decimals = self.swap.aequa_mint.decimals;
        let value = usd_at(amount, twap, decimals)?;
        self.aequa_pool.slash_bucket.draw(value.max(1), p.buyback_max_chunk, now)?;
        let (sale_bump, authority_bump, config_bump) = ([config.bumps.slash_sale], [config.bumps.swap_authority], [config.bumps.config]);
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                self.swap.aequa_token_program.key(),
                ATransferChecked {
                    from: self.slash_sale.to_account_info(),
                    mint: self.swap.aequa_mint.to_account_info(),
                    to: self.swap.transit_aequa.to_account_info(),
                    authority: self.slash_sale.to_account_info(),
                },
                &[&[SLASH_SALE, &sale_bump]],
            ),
            amount,
            decimals,
        )?;
        self.swap.transit_aequa.reload()?;
        let (aequa_before, usdc_before) = (self.swap.transit_aequa.amount, self.swap.transit_usdc.amount);
        let authority_seeds: &[&[u8]] = &[SWAP_AUTHORITY, &authority_bump];
        jupiter::swap(&self.swap.jupiter_program.to_account_info(), remaining, route, amount, &self.swap.swap_authority.key(), authority_seeds)?;
        self.swap.transit_aequa.reload()?;
        self.swap.transit_usdc.reload()?;
        require!(aequa_before - self.swap.transit_aequa.amount == amount, AequaError::SwapOverspent);
        let raised = self.swap.transit_usdc.amount.checked_sub(usdc_before).ok_or(AequaError::TwapBound)?;
        let fill = fill_price(raised, amount, decimals)?;
        require!(fill >= twap * (BPS - p.slash_twap_bound_bps as u64) as u128 / BPS as u128, AequaError::TwapBound);

        // The USDC joins the buffer; beyond the shortfall it backs new Reserve Fund USDq.
        let usdc_program = self.token_program.key();
        token::transfer_checked(
            CpiContext::new_with_signer(
                usdc_program,
                TransferChecked {
                    from: self.swap.transit_usdc.to_account_info(),
                    mint: self.usdc_mint.to_account_info(),
                    to: self.buffer.to_account_info(),
                    authority: self.swap.swap_authority.to_account_info(),
                },
                &[authority_seeds],
            ),
            raised,
            6,
        )?;
        let reserve = &mut self.reserve;
        let cover = raised.min(reserve.deficit);
        reserve.deficit -= cover;
        let excess = raised - cover;
        if excess > 0 {
            token::mint_to(
                CpiContext::new_with_signer(
                    usdc_program,
                    MintTo { mint: self.usdq_mint.to_account_info(), to: self.reserve_fund.to_account_info(), authority: config.to_account_info() },
                    &[&[CONFIG, &config_bump]],
                ),
                excess,
            )?;
        }
        let pool = &mut self.aequa_pool;
        pool.slash_sold_total = pool.slash_sold_total.saturating_add(amount);
        pool.slash_usdq_burned = pool.slash_usdq_burned.saturating_add(cover);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::fill_price;

    #[test]
    fn dollars_at_a_price_round_up() {
        // 100M AEQUA (6 dp) at $0.00005: $5,000.
        assert_eq!(super::usd_at(100_000_000_000_000, 50_000_000, 6).unwrap(), 5_000_000_000);
        assert_eq!(super::usd_at(1, 50_000_000, 6).unwrap(), 1);
    }

    #[test]
    fn fill_prices() {
        // $5,000 for 100M AEQUA (6 dp): $0.00005 each, at 1e12.
        assert_eq!(fill_price(5_000_000_000, 100_000_000_000_000, 6).unwrap(), 50_000_000);
    }
}
