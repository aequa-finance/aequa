//! `swap`: the keeper exchanges buffer USDC and a collateral vault through Jupiter.
//! The program checks what comes back against the Pyth price less the keeper's slippage cap
//! (50 bps), and a purchase against the collateral's spot cap. USDC counts at a dollar, as on
//! the USDC paths. The route passes through the swap authority's transit accounts
//! (`jupiter.rs`); the keeper's own signature never reaches Jupiter.

use anchor_lang::prelude::*;
use anchor_spl::{
    token::{self, Mint, Token, TokenAccount, TransferChecked},
    token_interface::{self, Mint as XMint, TokenAccount as XAccount, TokenInterface, TransferChecked as XTransferChecked},
};

use crate::constants::{seeds::*, BPS};
use crate::errors::AequaError;
use crate::events::Swapped;
use crate::instructions::pyth_pro::*;
use crate::jupiter;
use crate::math;
use crate::state::{path, Collateral, Config};
use crate::token2022::read_mint;

#[derive(Accounts)]
pub struct Swap<'info> {
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = keeper @ AequaError::Unauthorized, has_one = usdc_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(seeds = [COLLATERAL, mint.key().as_ref()], bump = collateral.bump, has_one = mint, has_one = vault)]
    pub collateral: Box<Account<'info, Collateral>>,
    pub mint: Box<InterfaceAccount<'info, XMint>>,
    #[account(mut)]
    pub vault: Box<InterfaceAccount<'info, XAccount>>,
    pub usdc_mint: Box<Account<'info, Mint>>,
    #[account(mut, seeds = [BUFFER], bump = config.bumps.buffer)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    /// CHECK: the swap authority PDA.
    #[account(seeds = [SWAP_AUTHORITY], bump = config.bumps.swap_authority)]
    pub swap_authority: UncheckedAccount<'info>,
    #[account(mut, address = config.transit_usdc)]
    pub transit_usdc: Box<Account<'info, TokenAccount>>,
    #[account(mut, address = collateral.transit)]
    pub transit_xstock: Box<InterfaceAccount<'info, XAccount>>,
    pub token_program: Program<'info, Token>,
    #[account(address = collateral.token_program)]
    pub xstock_token_program: Interface<'info, TokenInterface>,
    /// CHECK: Jupiter Aggregator v6, by address in `jupiter::swap`.
    pub jupiter_program: UncheckedAccount<'info>,
    pub pyth: PythPro<'info>,
}

impl<'info> Swap<'info> {
    /// `buy`: USDC from the buffer into the collateral vault; otherwise the reverse.
    pub fn run(&mut self, message: &[u8], ed25519_index: u16, buy: bool, amount_in: u64, route: &[u8], remaining: &[AccountInfo<'info>]) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let config = &self.config;
        let p = config.params;
        config.require_open(path::KEEPER)?;
        require!(amount_in > 0, AequaError::ZeroAmount);

        let update = self.pyth.verify(&self.keeper.to_account_info(), message, ed25519_index)?;
        update.require_fresh(now, p.pyth_freshness_secs)?;
        let price = update.price(self.collateral.feed, p.pyth_freshness_secs)?;
        let view = read_mint(&self.mint.to_account_info(), now, p.ca_window_before_secs, p.ca_window_after_secs)?;
        require!(!self.collateral.paused, AequaError::Paused);
        view.require_open()?;

        let keep = BPS - p.keeper_slippage_bps as u64;
        let min_out = if buy {
            require!(amount_in <= self.buffer.amount, AequaError::BufferInsufficient);
            let fair = math::usd_to_xstock_mid(amount_in, view.multiplier_e12, price.price_e8, view.decimals)?;
            math::mul_div(fair, keep, BPS)?
        } else {
            require!(amount_in <= self.vault.amount, AequaError::VaultInsufficient);
            let fair = math::xstock_value(amount_in, view.multiplier_e12, price.price_e8, view.decimals)?;
            math::mul_div(fair, keep, BPS)?
        };

        // Into the transit account.
        let usdc_program = self.token_program.key();
        let x_program = self.xstock_token_program.key();
        let mint_key = self.mint.key();
        let (buffer_bump, vault_bump, authority_bump) = ([config.bumps.buffer], [self.collateral.vault_bump], [config.bumps.swap_authority]);
        let authority_seeds: &[&[u8]] = &[SWAP_AUTHORITY, &authority_bump];
        if buy {
            token::transfer_checked(
                CpiContext::new_with_signer(
                    usdc_program,
                    TransferChecked {
                        from: self.buffer.to_account_info(),
                        mint: self.usdc_mint.to_account_info(),
                        to: self.transit_usdc.to_account_info(),
                        authority: self.buffer.to_account_info(),
                    },
                    &[&[BUFFER, &buffer_bump]],
                ),
                amount_in,
                6,
            )?;
        } else {
            token_interface::transfer_checked(
                CpiContext::new_with_signer(
                    x_program,
                    XTransferChecked {
                        from: self.vault.to_account_info(),
                        mint: self.mint.to_account_info(),
                        to: self.transit_xstock.to_account_info(),
                        authority: self.vault.to_account_info(),
                    },
                    &[&[COLLATERAL_VAULT, mint_key.as_ref(), &vault_bump]],
                ),
                amount_in,
                view.decimals,
            )?;
        }
        self.transit_usdc.reload()?;
        self.transit_xstock.reload()?;
        let (in_before, out_before) = if buy { (self.transit_usdc.amount, self.transit_xstock.amount) } else { (self.transit_xstock.amount, self.transit_usdc.amount) };

        jupiter::swap(&self.jupiter_program.to_account_info(), remaining, route, amount_in, &self.swap_authority.key(), authority_seeds)?;

        self.transit_usdc.reload()?;
        self.transit_xstock.reload()?;
        let (in_after, out_after) = if buy { (self.transit_usdc.amount, self.transit_xstock.amount) } else { (self.transit_xstock.amount, self.transit_usdc.amount) };
        require!(in_before - in_after == amount_in, AequaError::SwapOverspent);
        let received = out_after.checked_sub(out_before).ok_or(AequaError::SwapBelowMinimum)?;
        require!(received >= min_out, AequaError::SwapBelowMinimum);

        // Out of the transit account, into the vault or the buffer.
        if buy {
            let spot = math::xstock_value(self.vault.amount + received, view.multiplier_e12, price.price_e8, view.decimals)?;
            require!(spot <= self.collateral.spot_cap, AequaError::SpotCapExceeded);
            token_interface::transfer_checked(
                CpiContext::new_with_signer(
                    x_program,
                    XTransferChecked {
                        from: self.transit_xstock.to_account_info(),
                        mint: self.mint.to_account_info(),
                        to: self.vault.to_account_info(),
                        authority: self.swap_authority.to_account_info(),
                    },
                    &[authority_seeds],
                ),
                received,
                view.decimals,
            )?;
        } else {
            token::transfer_checked(
                CpiContext::new_with_signer(
                    usdc_program,
                    TransferChecked {
                        from: self.transit_usdc.to_account_info(),
                        mint: self.usdc_mint.to_account_info(),
                        to: self.buffer.to_account_info(),
                        authority: self.swap_authority.to_account_info(),
                    },
                    &[authority_seeds],
                ),
                received,
                6,
            )?;
        }
        emit!(Swapped { collateral: mint_key, buy, amount_in, amount_out: received, min_out, price_e8: price.price_e8 });
        Ok(())
    }
}
