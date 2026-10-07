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
