//! Swaps through Jupiter Aggregator v6 by CPI. Every swap runs between two
//! transit accounts, the associated token accounts of the swap authority PDA: the program
//! moves the input in, lets Jupiter swap it with the swap authority's signature, checks what
//! came out, and moves it on. The swap authority owns nothing else, so its signature can't
//! reach the buffer, the vaults or any other account.
//!
//! Accepted instructions: `shared_accounts_route_v2` and `route_v2`, whose scalar arguments
//! sit at fixed offsets before the route plan (Jupiter's on-chain IDL, checked 2026-10-08).

use anchor_lang::{
    prelude::*,
    solana_program::{instruction::Instruction, program::invoke_signed},
};

use crate::errors::AequaError;

pub const JUPITER: Pubkey = pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");

/// `sha256("global:shared_accounts_route_v2")[..8]` and `sha256("global:route_v2")[..8]`.
const SHARED_ACCOUNTS_ROUTE_V2: [u8; 8] = [0xd1, 0x98, 0x53, 0x93, 0x7c, 0xfe, 0xd8, 0xe9];
const ROUTE_V2: [u8; 8] = [0xbb, 0x64, 0xfa, 0xcc, 0x31, 0xc4, 0xaf, 0x14];

/// Where `in_amount` starts in each accepted instruction's data; the platform fee and the
/// positive-slippage fee follow at +18 and +20.
fn in_amount_offset(data: &[u8]) -> Result<usize> {
    require!(data.len() >= 8, AequaError::InvalidRoute);
    let disc: [u8; 8] = data[..8].try_into().unwrap();
    let o = if disc == SHARED_ACCOUNTS_ROUTE_V2 {
        9
    } else if disc == ROUTE_V2 {
        8
    } else {
        return err!(AequaError::InvalidRoute);
    };
    // The scalar arguments and the route plan's length prefix.
    require!(data.len() >= o + 26, AequaError::InvalidRoute);
    Ok(o)
}

/// Checks the route's arguments: it spends exactly `amount_in` and takes no platform or
/// positive-slippage fee.
pub fn check_route(data: &[u8], amount_in: u64) -> Result<()> {
    let o = in_amount_offset(data)?;
    let in_amount = u64::from_le_bytes(data[o..o + 8].try_into().unwrap());
    let platform_fee = u16::from_le_bytes(data[o + 18..o + 20].try_into().unwrap());
    let positive_slippage = u16::from_le_bytes(data[o + 20..o + 22].try_into().unwrap());
    require!(in_amount == amount_in, AequaError::InvalidRoute);
    require!(platform_fee == 0 && positive_slippage == 0, AequaError::InvalidRoute);
    Ok(())
}

/// Invokes Jupiter with the route's accounts as given, the swap authority signing. Nothing
/// else signs, and the swap authority is never writable.
pub fn swap<'info>(jupiter: &AccountInfo<'info>, accounts: &[AccountInfo<'info>], data: &[u8], amount_in: u64, swap_authority: &Pubkey, seeds: &[&[u8]]) -> Result<()> {
    require_keys_eq!(*jupiter.key, JUPITER, AequaError::InvalidRoute);
    check_route(data, amount_in)?;
    let metas = accounts
        .iter()
        .map(|a| {
            let signer = a.key == swap_authority;
            AccountMeta { pubkey: *a.key, is_signer: signer, is_writable: a.is_writable && !signer }
        })
        .collect();
    let ix = Instruction { program_id: JUPITER, accounts: metas, data: data.to_vec() };
    let mut infos = accounts.to_vec();
    infos.push(jupiter.clone());
    invoke_signed(&ix, &infos, &[seeds])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(disc: [u8; 8], shared: bool, amount: u64, fee: u16, positive: u16) -> Vec<u8> {
        let mut d = disc.to_vec();
        if shared {
            d.push(3);
        }
        d.extend_from_slice(&amount.to_le_bytes());
        d.extend_from_slice(&123u64.to_le_bytes());
        d.extend_from_slice(&50u16.to_le_bytes());
        d.extend_from_slice(&fee.to_le_bytes());
        d.extend_from_slice(&positive.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        d
    }

    #[test]
    fn accepts_only_v2_routes_that_spend_the_amount_without_fees() {
        check_route(&data(SHARED_ACCOUNTS_ROUTE_V2, true, 1_000, 0, 0), 1_000).unwrap();
        check_route(&data(ROUTE_V2, false, 1_000, 0, 0), 1_000).unwrap();
        assert!(check_route(&data(SHARED_ACCOUNTS_ROUTE_V2, true, 999, 0, 0), 1_000).is_err());
        assert!(check_route(&data(ROUTE_V2, false, 1_000, 5, 0), 1_000).is_err());
        assert!(check_route(&data(ROUTE_V2, false, 1_000, 0, 1), 1_000).is_err());
        let mut other = data(ROUTE_V2, false, 1_000, 0, 0);
        other[0] ^= 1;
        assert!(check_route(&other, 1_000).is_err());
    }
}
