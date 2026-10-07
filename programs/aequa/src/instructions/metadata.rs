//! Names for USDq and sUSDq in wallets: Metaplex Token Metadata, created and updated by the
//! program as their mint authority. The update authority is the Config PDA, so only this
//! instruction (admin) can change them.

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, program::invoke_signed},
};
use anchor_spl::token::Mint;

use crate::constants::seeds::*;
use crate::errors::AequaError;
use crate::state::{Config, UsdqVault};

pub const TOKEN_METADATA: Pubkey = pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
const CREATE_METADATA_V3: u8 = 33;
const UPDATE_METADATA_V2: u8 = 15;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct TokenMetadataArgs {
    pub name: String,
    pub symbol: String,
    pub uri: String,
}

#[derive(Accounts)]
pub struct SetTokenMetadata<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [CONFIG], bump = config.bumps.config, has_one = admin @ AequaError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    pub admin: Signer<'info>,
    #[account(seeds = [SUSDQ_VAULT], bump = usdq_vault.bump)]
    pub usdq_vault: Box<Account<'info, UsdqVault>>,
    #[account(constraint = mint.key() == config.usdq_mint || mint.key() == config.susdq_mint @ AequaError::ParamOutOfRange)]
    pub mint: Box<Account<'info, Mint>>,
    /// CHECK: the mint's metadata PDA under Token Metadata.
    #[account(mut, seeds = [b"metadata", TOKEN_METADATA.as_ref(), mint.key().as_ref()], bump, seeds::program = TOKEN_METADATA)]
    pub metadata: UncheckedAccount<'info>,
    /// CHECK: Token Metadata, by address.
    #[account(address = TOKEN_METADATA)]
    pub token_metadata_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

fn borsh_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// DataV2 with no creators, collection or uses, and no royalty.
fn data_v2(out: &mut Vec<u8>, m: &TokenMetadataArgs) {
    borsh_string(out, &m.name);
    borsh_string(out, &m.symbol);
    borsh_string(out, &m.uri);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0]);
}

impl SetTokenMetadata<'_> {
    pub fn run(&mut self, args: TokenMetadataArgs) -> Result<()> {
        require!(args.name.len() <= 32 && args.symbol.len() <= 10 && args.uri.len() <= 200, AequaError::ParamOutOfRange);
        let config = &self.config;
        let config_bump = [config.bumps.config];
        let vault_bump = [self.usdq_vault.bump];
        let config_seeds: &[&[u8]] = &[CONFIG, &config_bump];
        let vault_seeds: &[&[u8]] = &[SUSDQ_VAULT, &vault_bump];

        if self.metadata.data_is_empty() {
            // The mint authority signs the creation: Config for USDq, the sUSDq vault for sUSDq.
            let authority = if self.mint.key() == config.usdq_mint { config.key() } else { self.usdq_vault.key() };
            let mut data = vec![CREATE_METADATA_V3];
            data_v2(&mut data, &args);
            data.push(1); // is_mutable
            data.push(0); // no collection details
            let ix = Instruction {
                program_id: TOKEN_METADATA,
                accounts: vec![
                    AccountMeta::new(self.metadata.key(), false),
                    AccountMeta::new_readonly(self.mint.key(), false),
                    AccountMeta::new_readonly(authority, true),
                    AccountMeta::new(self.payer.key(), true),
                    AccountMeta::new_readonly(config.key(), true),
                    AccountMeta::new_readonly(self.system_program.key(), false),
                ],
                data,
            };
            invoke_signed(
                &ix,
                &[
                    self.metadata.to_account_info(),
                    self.mint.to_account_info(),
                    if authority == config.key() { config.to_account_info() } else { self.usdq_vault.to_account_info() },
                    self.payer.to_account_info(),
                    config.to_account_info(),
                    self.system_program.to_account_info(),
                    self.token_metadata_program.to_account_info(),
                ],
                &[config_seeds, vault_seeds],
            )?;
        } else {
            let mut data = vec![UPDATE_METADATA_V2];
            data.push(1);
            data_v2(&mut data, &args);
            data.extend_from_slice(&[0, 0, 0]); // keep the update authority, sale flag and mutability
            let ix = Instruction {
                program_id: TOKEN_METADATA,
                accounts: vec![AccountMeta::new(self.metadata.key(), false), AccountMeta::new_readonly(config.key(), true)],
                data,
            };
            invoke_signed(
                &ix,
                &[self.metadata.to_account_info(), config.to_account_info(), self.token_metadata_program.to_account_info()],
                &[config_seeds],
            )?;
        }
        Ok(())
    }
}
