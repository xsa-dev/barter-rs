use thiserror::Error;

#[derive(Error, Debug)]
pub enum PortfolioError {
    #[error("Failed to build struct due to incomplete attributes provided")]
    BuilderIncomplete(),

    #[error("Failed to calculate PnL due to no Fee::TotalFee in HashMap<Fee, FeeAmount>")]
    CalcProfitLossError(),

    #[error("Failed to parse Position entry direction due to ambiguous fill quantity & Decision.")]
    ParseEntryDirectionError(),

    #[error("Cannot exit Position with an entry decision FillEvent.")]
    CannotEnterPositionWithExitFill(),

    #[error("Cannot exit Position with an entry decision FillEvent.")]
    CannotExitPositionWithEntryFill(),
}