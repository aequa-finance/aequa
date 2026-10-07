//! aequa: USDq, a dollar backed by tokenized S&P 500 and Nasdaq-100 ETFs held on Solana and
//! hedged with short index perps on Hyperliquid; sUSDq, staked USDq; and stAEQUA.

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod state;

declare_id!("9xhUvFQy1yAnyuZTygfK1nD6naicXQoWMR6doxFxuTbL");

#[program]
pub mod aequa {
    use super::*;

    pub fn initialize(_ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
