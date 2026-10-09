//! Units, limits and seeds shared by every instruction.

/// Basis points in one.
pub const BPS: u64 = 10_000;
pub const HOUR: i64 = 3_600;
pub const DAY: i64 = 86_400;

/// USDq, sUSDq and USDC carry 6 decimals; dollar values inside the program use the same unit.
pub const USD_DECIMALS: u8 = 6;
/// Prices are carried at 8 decimals (Pyth's equity exponent), multipliers at 12.
pub const PRICE_DECIMALS: u32 = 8;
pub const MULT_ONE: u128 = 1_000_000_000_000;

/// Collateral listings the program can hold (new collateral joins by vote).
pub const MAX_COLLATERALS: usize = 8;

/// The bridge route's fixed parts: Circle's CCTP V2 programs, HyperEVM's domain and
/// Circle's CctpForwarder there. The admin names only the HyperCore account.
pub mod cctp {
    use anchor_lang::prelude::*;

    pub const TOKEN_MESSENGER_MINTER: Pubkey = pubkey!("CCTPV2vPZJS2u2BBsUoscuikbYjnpFmbFsvVuJdgUMQe");
    pub const MESSAGE_TRANSMITTER: Pubkey = pubkey!("CCTPV2Sm4AdWt5296sk4P66VBZ7bEhcARwFaaS9YPbeC");
    pub const HYPEREVM_DOMAIN: u32 = 19;
    /// 0xb21D281DEdb17AE5B501F6AA8256fe38C4e45757, left-padded to 32 bytes.
    pub const FORWARDER: [u8; 32] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xb2, 0x1d, 0x28, 0x1d, 0xed, 0xb1, 0x7a, 0xe5, 0xb5, 0x01, 0xf6, 0xaa, 0x82, 0x56, 0xfe, 0x38, 0xc4, 0xe4, 0x57, 0x57,
    ];
    /// The forwarding hook's magic, padded to 24 bytes.
    pub const HOOK_MAGIC: &[u8] = b"cctp-forward";
    pub const HOOK_LEN: usize = 56;
}

pub mod seeds {
    pub const CONFIG: &[u8] = b"config";
    pub const RESERVE: &[u8] = b"reserve";
    pub const COLLATERAL: &[u8] = b"collateral";
    pub const COLLATERAL_VAULT: &[u8] = b"collateral_vault";
    pub const USDQ_MINT: &[u8] = b"usdq_mint";
    pub const SUSDQ_MINT: &[u8] = b"susdq_mint";
    pub const SUSDQ_VAULT: &[u8] = b"susdq_vault";
    pub const SUSDQ_ASSETS: &[u8] = b"susdq_assets";
    pub const SUSDQ_SILO: &[u8] = b"susdq_silo";
    pub const BUFFER: &[u8] = b"buffer";
    pub const LANDING: &[u8] = b"landing";
    pub const RESERVE_FUND: &[u8] = b"reserve_fund";
    pub const PROTOCOL: &[u8] = b"protocol";
}
