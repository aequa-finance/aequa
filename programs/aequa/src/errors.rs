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
    #[msg("That would take USDq past its supply ceiling")]
    SupplyCapExceeded,
    #[msg("That would take this collateral past its spot cap")]
    SpotCapExceeded,
    #[msg("The buffer holds less than that right now")]
    BufferInsufficient,
    #[msg("The vault holds less of this collateral than that")]
    VaultInsufficient,

    // Collateral state
    #[msg("No fresh price for this collateral: the session is closed")]
    CollateralNotLive,
    #[msg("The hedge report is stale or the hedge market is paused")]
    HedgeNotLive,
    #[msg("This collateral is inside a corporate-action window")]
    CorporateActionWindow,
    #[msg("The issuer has paused this collateral's mint")]
    IssuerPaused,
    #[msg("The collateral mint fails an admission check")]
    CollateralNotAdmissible,
    #[msg("The signed price is older than the freshness limit")]
    StalePrice,

    // Pyth
    #[msg("The Pyth message could not be read")]
    InvalidPythMessage,
    #[msg("The Pyth message is on a channel slower than 200 ms")]
    WrongPythChannel,
    #[msg("The Pyth price is missing or not positive")]
    InvalidPythPrice,

    // sUSDq
    #[msg("The cooldown is still running")]
    CooldownActive,
    #[msg("There is nothing to claim")]
    NothingToClaim,

    // stAEQUA
    #[msg("There is no stake to unstake")]
    NothingStaked,
    #[msg("The position is cooling down")]
    PositionCooling,
    #[msg("The withdraw window isn't open")]
    WindowNotOpen,
    #[msg("AEQUA has not been set up yet")]
    AequaNotSet,

    // Cranks
    #[msg("Not due yet")]
    NotDue,
    #[msg("Nothing to move")]
    NothingToMove,
    #[msg("The deficit has not held long enough")]
    DeficitNotHeld,
    #[msg("The slash cap for the rolling window is used")]
    SlashCapReached,
    #[msg("The fill is outside the bound around the TWAP")]
    TwapBound,
    #[msg("Not enough price observations for the TWAP")]
    TwapNotReady,

    // Hedge reporting and the desk
    #[msg("The report is older than the last one")]
    ReportOutOfOrder,
    #[msg("The swap route is not a Jupiter route over the expected accounts")]
    InvalidRoute,
    #[msg("The swap returned less than the Pyth price allows")]
    SwapBelowMinimum,
    #[msg("The swap spent more than it was given")]
    SwapOverspent,
    #[msg("The bridge route is not set")]
    BridgeNotSet,

    // Configuration
    #[msg("The signer does not hold this role")]
    Unauthorized,
    #[msg("This path is paused")]
    Paused,
    #[msg("A parameter is outside its hard-coded range")]
    ParamOutOfRange,
    #[msg("The collateral list is full")]
    TooManyCollaterals,
    #[msg("The collateral accounts don't match the listing")]
    CollateralMismatch,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
