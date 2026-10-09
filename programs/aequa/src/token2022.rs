//! Reading a collateral mint on every use: admission checks, the issuer's pause,
//! the ScaledUiAmount multiplier in force and the corporate-action window around a change.

use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{
        confidential_transfer::ConfidentialTransferMint, interest_bearing_mint::InterestBearingConfig, non_transferable::NonTransferable,
        pausable::PausableConfig, scaled_ui_amount::ScaledUiAmountConfig, transfer_fee::TransferFeeConfig, transfer_hook::TransferHook,
        BaseStateWithExtensions, StateWithExtensions,
    },
    state::Mint,
};

use crate::constants::MULT_ONE;
use crate::errors::AequaError;
use crate::math;

/// What a collateral mint says right now.
#[derive(Clone, Copy, Debug)]
pub struct MintView {
    pub decimals: u8,
    pub supply: u64,
    /// The multiplier in force, at 1e12.
    pub multiplier_e12: u128,
    /// The issuer has paused the mint.
    pub paused: bool,
    /// Inside the window around a scheduled multiplier change.
    pub in_ca_window: bool,
    /// When that window closes (zero outside one).
    pub ca_window_end: i64,
}

/// Reads the mint and runs the admission checks: no transfer fee, no transfer-hook program,
/// confidential-transfer auto-approve off, transferable, no interest-bearing scale, and a
/// positive finite multiplier. A failure closes the collateral's paths.
pub fn read_mint(info: &AccountInfo, now: i64, before: u32, after: u32) -> Result<MintView> {
    read(info, now, before, after, true)
}

/// The mint for valuation only (`distribute`): its multiplier and window, without the admission
/// checks, so a vault whose issuer turned on a fee or a hook still counts at its value.
pub fn read_mint_value(info: &AccountInfo, now: i64, before: u32, after: u32) -> Result<MintView> {
    read(info, now, before, after, false)
}

fn read(info: &AccountInfo, now: i64, before: u32, after: u32, admit: bool) -> Result<MintView> {
    let data = info.try_borrow_data()?;
    let mint = StateWithExtensions::<Mint>::unpack(&data).map_err(|_| error!(AequaError::CollateralNotAdmissible))?;
    if admit {
        admission(&mint)?;
    }
    view(&mint, now, before, after)
}

fn admission(mint: &StateWithExtensions<Mint>) -> Result<()> {
    if let Ok(fee) = mint.get_extension::<TransferFeeConfig>() {
        for f in [fee.older_transfer_fee, fee.newer_transfer_fee] {
            require!(u16::from(f.transfer_fee_basis_points) == 0, AequaError::CollateralNotAdmissible);
        }
    }
    if let Ok(hook) = mint.get_extension::<TransferHook>() {
        require!(Option::<Pubkey>::from(hook.program_id).is_none(), AequaError::CollateralNotAdmissible);
    }
    if let Ok(ct) = mint.get_extension::<ConfidentialTransferMint>() {
        require!(!bool::from(ct.auto_approve_new_accounts), AequaError::CollateralNotAdmissible);
    }
    require!(mint.get_extension::<NonTransferable>().is_err(), AequaError::CollateralNotAdmissible);
    require!(mint.get_extension::<InterestBearingConfig>().is_err(), AequaError::CollateralNotAdmissible);
    Ok(())
}

fn view(mint: &StateWithExtensions<Mint>, now: i64, before: u32, after: u32) -> Result<MintView> {
    let paused = mint.get_extension::<PausableConfig>().map(|p| bool::from(p.paused)).unwrap_or(false);

    let (multiplier_e12, in_ca_window, ca_window_end) = match mint.get_extension::<ScaledUiAmountConfig>() {
        Ok(s) => {
            let current: f64 = s.multiplier.into();
            let next: f64 = s.new_multiplier.into();
            let at: i64 = s.new_multiplier_effective_timestamp.into();
            let m = if now >= at { next } else { current };
            let m = math::multiplier_e12(m).ok_or(AequaError::CollateralNotAdmissible)?;
            // A scheduled change: closed from `before` ahead of it until `after` past it.
            let scheduled = current.to_bits() != next.to_bits();
            let start = at.saturating_sub(before as i64);
            let end = at.saturating_add(after as i64);
            let inside = scheduled && now >= start && now < end;
            (m, inside, if inside { end } else { 0 })
        }
        Err(_) => (MULT_ONE, false, 0),
    };

    Ok(MintView {
        decimals: mint.base.decimals,
        supply: mint.base.supply,
        multiplier_e12,
        paused,
        in_ca_window,
        ca_window_end,
    })
}

impl MintView {
    /// The xStock paths need an admissible mint that is neither paused nor inside a window.
    pub fn require_open(&self) -> Result<()> {
        require!(!self.paused, AequaError::IssuerPaused);
        require!(!self.in_ca_window, AequaError::CorporateActionWindow);
        Ok(())
    }
}
