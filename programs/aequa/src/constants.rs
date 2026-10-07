//! Units, limits and seeds shared by every instruction.

/// Basis points in one.
pub const BPS: u64 = 10_000;
pub const HOUR: i64 = 3_600;
pub const DAY: i64 = 86_400;

/// USDq, sUSDq and USDC carry 6 decimals; dollar values inside the program use the same unit.
pub const USD_DECIMALS: u8 = 6;
/// Prices are carried at 8 decimals (Pyth's equity exponent).
pub const PRICE_DECIMALS: u32 = 8;

pub mod seeds {
    pub const CONFIG: &[u8] = b"config";
    pub const RESERVE: &[u8] = b"reserve";
    pub const USDQ_MINT: &[u8] = b"usdq_mint";
    pub const SUSDQ_MINT: &[u8] = b"susdq_mint";
    pub const SUSDQ_VAULT: &[u8] = b"susdq_vault";
    pub const SUSDQ_ASSETS: &[u8] = b"susdq_assets";
    pub const SUSDQ_SILO: &[u8] = b"susdq_silo";
    pub const BUFFER: &[u8] = b"buffer";
    pub const RESERVE_FUND: &[u8] = b"reserve_fund";
    pub const PROTOCOL: &[u8] = b"protocol";
}
