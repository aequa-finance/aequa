//! The accounts a Pyth Pro verification needs, as one group every priced instruction embeds.

use anchor_lang::prelude::*;

use crate::pyth::{self, PythAccounts};

/// The Pyth Pro accounts every priced instruction carries.
#[derive(Accounts)]
pub struct PythPro<'info> {
    /// CHECK: the Lazer program, by address.
    #[account(address = pyth::LAZER_PROGRAM)]
    pub pyth_program: UncheckedAccount<'info>,
    /// CHECK: the Lazer storage, by address; the Lazer program reads it.
    #[account(address = pyth::LAZER_STORAGE)]
    pub pyth_storage: UncheckedAccount<'info>,
    /// CHECK: the treasury the Lazer storage names; the Lazer program checks it.
    #[account(mut)]
    pub pyth_treasury: UncheckedAccount<'info>,
    /// CHECK: the instructions sysvar, by address.
    #[account(address = pyth::INSTRUCTIONS_SYSVAR)]
    pub instructions: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl<'info> PythPro<'info> {
    pub fn verify(&self, payer: &AccountInfo<'info>, message: &[u8], ed25519_index: u16) -> Result<pyth::Update> {
        let program = self.pyth_program.to_account_info();
        let storage = self.pyth_storage.to_account_info();
        let treasury = self.pyth_treasury.to_account_info();
        let system_program = self.system_program.to_account_info();
        let instructions = self.instructions.to_account_info();
        let accounts = PythAccounts {
            payer,
            program: &program,
            storage: &storage,
            treasury: &treasury,
            system_program: &system_program,
            instructions: &instructions,
        };
        pyth::verify(&accounts, message, ed25519_index)
    }
}
