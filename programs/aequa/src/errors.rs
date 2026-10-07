use anchor_lang::prelude::*;

/// Every refusal has a name.
#[error_code]
pub enum AequaError {
    // Amounts and balances
    #[msg("Enter an amount")]
    ZeroAmount,
    #[msg("The balance is lower than that")]
    InsufficientFunds,
    #[msg("The amount out is below the minimum asked for")]
    SlippageExceeded,

    // Capacity
    #[msg("This hour's capacity on this path was used in the meantime")]
    CapacityExceeded,
    #[msg("The buffer holds less than that right now")]
    BufferInsufficient,

    // Collateral state
    #[msg("No fresh price for this collateral: the session is closed")]
    CollateralNotLive,
    #[msg("The signed price is older than the freshness limit")]
    StalePrice,

    // Pyth
    #[msg("The Pyth message could not be read")]
    InvalidPythMessage,
    #[msg("The Pyth message is on a channel slower than 200 ms")]
    WrongPythChannel,
    #[msg("The Pyth price is missing or not positive")]
    InvalidPythPrice,

    // Configuration
    #[msg("The signer does not hold this role")]
    Unauthorized,
    #[msg("This path is paused")]
    Paused,
    #[msg("A parameter is outside its hard-coded range")]
    ParamOutOfRange,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
