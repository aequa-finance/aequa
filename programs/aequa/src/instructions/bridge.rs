//! `bridge_out`: the keeper sends buffer USDC to the hedge account. The program
//! burns it through CCTP V2 towards HyperEVM with the route fixed in Config: the CctpForwarder
//! as mint recipient and caller, and a hook naming the hedge account, so Circle's forwarding
//! service mints it there and credits HyperCore. The keeper picks the amount, the fee (within
//! the route's cap) and fast or standard finality; it can't change where the USDC goes.

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, program::invoke_signed},
};
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::{seeds::*, BPS};
use crate::errors::AequaError;
use crate::state::{path, Config, Reserve};

/// `sha256("global:deposit_for_burn_with_hook")[..8]`.
const DEPOSIT_FOR_BURN_WITH_HOOK: [u8; 8] = [111, 245, 62, 131, 204, 108, 223, 155];
pub const FINALITY_FAST: u32 = 1_000;
pub const FINALITY_STANDARD: u32 = 2_000;

#[derive(Accounts)]
pub struct BridgeOut<'info> {
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = keeper @ AequaError::Unauthorized, has_one = usdc_mint)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [RESERVE], bump = reserve.bump)]
    pub reserve: Box<Account<'info, Reserve>>,
    #[account(mut, seeds = [BUFFER], bump = config.bumps.buffer)]
    pub buffer: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub usdc_mint: Box<Account<'info, Mint>>,
    /// A fresh account CCTP writes the message into; it signs, and the keeper pays its rent.
    #[account(mut)]
    pub message_sent_event_data: Signer<'info>,
    /// CHECK: CCTP's sender authority PDA; CCTP checks it.
    pub sender_authority: UncheckedAccount<'info>,
    /// CHECK: CCTP's denylist entry for the buffer; CCTP checks it is empty.
    pub denylist: UncheckedAccount<'info>,
    /// CHECK: CCTP's MessageTransmitter state; CCTP checks it.
    #[account(mut)]
    pub message_transmitter: UncheckedAccount<'info>,
    /// CHECK: CCTP's TokenMessenger state; CCTP checks it.
    pub token_messenger: UncheckedAccount<'info>,
    /// CHECK: CCTP's RemoteTokenMessenger for the destination; CCTP checks its domain.
    pub remote_token_messenger: UncheckedAccount<'info>,
    /// CHECK: CCTP's TokenMinter; CCTP checks it.
    pub token_minter: UncheckedAccount<'info>,
    /// CHECK: CCTP's LocalToken for USDC; CCTP checks it.
    #[account(mut)]
    pub local_token: UncheckedAccount<'info>,
    /// CHECK: CCTP's event authority; CCTP checks it.
    pub cctp_event_authority: UncheckedAccount<'info>,
    /// CHECK: by address, from Config.
    #[account(address = config.bridge.message_transmitter)]
    pub message_transmitter_program: UncheckedAccount<'info>,
    /// CHECK: by address, from Config.
    #[account(address = config.bridge.token_messenger_minter)]
    pub token_messenger_minter_program: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl BridgeOut<'_> {
    pub fn run(&mut self, amount: u64, max_fee: u64, fast: bool) -> Result<()> {
        let config = &self.config;
        config.require_open(path::KEEPER)?;
        let route = config.bridge;
        require!(route.is_set(), AequaError::BridgeNotSet);
        require!(amount > max_fee, AequaError::ZeroAmount);
        require!(amount <= self.buffer.amount, AequaError::BufferInsufficient);
        let cap = (amount as u128 * route.max_fee_bps as u128 / BPS as u128) as u64 + route.max_fee_flat;
        require!(max_fee <= cap, AequaError::ParamOutOfRange);

        let hook = &route.hook_data[..route.hook_len as usize];
        let mut data = Vec::with_capacity(8 + 92 + hook.len());
        data.extend_from_slice(&DEPOSIT_FOR_BURN_WITH_HOOK);
        data.extend_from_slice(&amount.to_le_bytes());
        data.extend_from_slice(&route.destination_domain.to_le_bytes());
        data.extend_from_slice(&route.mint_recipient);
        data.extend_from_slice(&route.destination_caller);
        data.extend_from_slice(&max_fee.to_le_bytes());
        data.extend_from_slice(&(if fast { FINALITY_FAST } else { FINALITY_STANDARD }).to_le_bytes());
        data.extend_from_slice(&(hook.len() as u32).to_le_bytes());
        data.extend_from_slice(hook);

        let tmm = self.token_messenger_minter_program.key();
        let ix = Instruction {
            program_id: tmm,
            accounts: vec![
                AccountMeta::new_readonly(self.buffer.key(), true),
                AccountMeta::new(self.keeper.key(), true),
                AccountMeta::new_readonly(self.sender_authority.key(), false),
                AccountMeta::new(self.buffer.key(), false),
                AccountMeta::new_readonly(self.denylist.key(), false),
                AccountMeta::new(self.message_transmitter.key(), false),
                AccountMeta::new_readonly(self.token_messenger.key(), false),
                AccountMeta::new_readonly(self.remote_token_messenger.key(), false),
                AccountMeta::new_readonly(self.token_minter.key(), false),
                AccountMeta::new(self.local_token.key(), false),
                AccountMeta::new(self.usdc_mint.key(), false),
                AccountMeta::new(self.message_sent_event_data.key(), true),
                AccountMeta::new_readonly(self.message_transmitter_program.key(), false),
                AccountMeta::new_readonly(tmm, false),
                AccountMeta::new_readonly(self.token_program.key(), false),
                AccountMeta::new_readonly(self.system_program.key(), false),
                AccountMeta::new_readonly(self.cctp_event_authority.key(), false),
                AccountMeta::new_readonly(tmm, false),
            ],
            data,
        };
        let bump = [config.bumps.buffer];
        invoke_signed(
            &ix,
            &[
                self.buffer.to_account_info(),
                self.keeper.to_account_info(),
                self.sender_authority.to_account_info(),
                self.denylist.to_account_info(),
                self.message_transmitter.to_account_info(),
                self.token_messenger.to_account_info(),
                self.remote_token_messenger.to_account_info(),
                self.token_minter.to_account_info(),
                self.local_token.to_account_info(),
                self.usdc_mint.to_account_info(),
                self.message_sent_event_data.to_account_info(),
                self.message_transmitter_program.to_account_info(),
                self.token_messenger_minter_program.to_account_info(),
                self.token_program.to_account_info(),
                self.system_program.to_account_info(),
                self.cctp_event_authority.to_account_info(),
            ],
            &[&[BUFFER, &bump]],
        )?;

        // What reaches the hedge account is the amount less the fee, which Circle takes whole
        // when it forwards; the reporter's credited total counts the same.
        let reserve = &mut self.reserve;
        reserve.bridged_out_total = reserve.bridged_out_total.checked_add(amount - max_fee).ok_or(AequaError::MathOverflow)?;
        Ok(())
    }
}
