use crate::execution::fill::{FillEvent, Fees, FeeAmount};
use crate::portfolio::error::PortfolioError;
use crate::data::market::MarketEvent;
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use crate::strategy::signal::Decision;

/// Enters a new [Position].
pub trait PositionEnterer {
    /// Returns a new [Position], given an input [FillEvent].
    fn enter(fill: &FillEvent) -> Result<Position, PortfolioError>;
}

/// Updates an open [Position].
pub trait PositionUpdater {
    /// Updates an open [Position] using the latest input [MarketEvent].
    fn update(&mut self, market: &MarketEvent);
}

/// Exits an open [Position].
pub trait PositionExiter {
    /// Exits an open [Position], given the input Portfolio equity & the [FillEvent] returned from
    /// a execution::handler.
    fn exit(&mut self, portfolio_value: f64, fill: &FillEvent) -> Result<(), PortfolioError>;
}

/// Data encapsulating the state of an ongoing or closed [Position].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    /// Metadata detailing trace UUIDs, timestamps & equity associated with entering, updating & exiting.
    pub meta: PositionMeta,

    /// Exchange associated with this [Position] instance.
    pub exchange: String,

    /// Ticker symbol associated with this [Position] instance.
    pub symbol: String,

    /// Long or Short.
    pub direction: Direction,

    /// +ve or -ve quantity of symbol contracts opened.
    pub quantity: f64,

    /// All fees types incurred from entering a [Position], and their associated [FeeAmount].
    pub enter_fees: Fees,

    /// Total of enter_fees incurred. Sum of every [FeeAmount] in [Fees] when entering a [Position].
    pub enter_fees_total: FeeAmount,

    /// Enter average price excluding the entry_fees_total.
    pub enter_avg_price_gross: f64,

    /// abs(Quantity) * enter_avg_price_gross.
    pub enter_value_gross: f64,

    /// All fees types incurred from exiting a [Position], and their associated [FeeAmount].
    pub exit_fees: Fees,

    /// Total of exit_fees incurred. Sum of every [FeeAmount] in [Fees] when entering a [Position].
    pub exit_fees_total: FeeAmount,

    /// Exit average price excluding the exit_fees_total.
    pub exit_avg_price_gross: f64,

    /// abs(Quantity) * exit_avg_price_gross.
    pub exit_value_gross: f64,

    /// Symbol current close price.
    pub current_symbol_price: f64,

    /// abs(Quantity) * current_symbol_price.
    pub current_value_gross: f64,

    /// Unrealised P&L whilst the [Position] is open.
    pub unreal_profit_loss: f64,

    /// Realised P&L after the [Position] has closed.
    pub result_profit_loss: f64,
}

impl PositionEnterer for Position {
    fn enter(fill: &FillEvent) -> Result<Position, PortfolioError> {
        // Initialise Position Metadata
        let metadata = PositionMeta {
            enter_trace_id: fill.trace_id,
            enter_bar_timestamp: fill.market_meta.timestamp,
            last_update_trace_id: fill.trace_id,
            last_update_timestamp: fill.timestamp,
            exit_trace_id: None,
            exit_bar_timestamp: None,
            exit_equity_point: None
        };

        // Enter fees
        let enter_fees_total = fill.fees.calculate_total_fees();

        // Enter price
        let enter_avg_price_gross = Position::calculate_avg_price_gross(fill);

        // Unreal profit & loss
        let unreal_profit_loss = -enter_fees_total * 2.0;

        Ok(Position {
            meta: metadata,
            exchange: fill.exchange.clone(),
            symbol: fill.symbol.clone(),
            direction: Position::parse_entry_direction(&fill)?,
            quantity: fill.quantity,
            enter_fees: fill.fees.clone(),
            enter_fees_total,
            enter_avg_price_gross,
            enter_value_gross: fill.fill_value_gross,
            exit_fees: Fees::default(),
            exit_fees_total: 0.0,
            exit_avg_price_gross: 0.0,
            exit_value_gross: 0.0,
            current_symbol_price: enter_avg_price_gross,
            current_value_gross: fill.fill_value_gross,
            unreal_profit_loss,
            result_profit_loss: 0.0,
        })
    }
}

impl PositionUpdater for Position {
    fn update(&mut self, market: &MarketEvent) {
        self.meta.last_update_trace_id = market.trace_id;
        self.meta.last_update_timestamp = market.timestamp;

        self.current_symbol_price = market.bar.close;

        // Market value gross
        self.current_value_gross = market.bar.close * self.quantity.abs();

        // Unreal profit & loss
        self.unreal_profit_loss = self.calculate_unreal_profit_loss();
    }
}

impl PositionExiter for Position {
    fn exit(&mut self, mut portfolio_value: f64, fill: &FillEvent) -> Result<(), PortfolioError> {
        if fill.decision.is_entry() {
            return Err(PortfolioError::CannotExitPositionWithEntryFill)
        }
        
        // Exit fees
        self.exit_fees = fill.fees.clone();
        self.exit_fees_total = fill.fees.calculate_total_fees();

        // Exit value & price
        self.exit_value_gross = fill.fill_value_gross;
        self.exit_avg_price_gross = Position::calculate_avg_price_gross(fill);

        // Result profit & loss
        self.result_profit_loss = self.calculate_result_profit_loss();
        self.unreal_profit_loss = self.result_profit_loss;

        // Metadata
        portfolio_value += self.result_profit_loss;
        self.meta.last_update_trace_id = fill.trace_id;
        self.meta.last_update_timestamp = fill.timestamp;
        self.meta.exit_trace_id = Some(fill.trace_id);
        self.meta.exit_equity_point = Some(EquityPoint {
            equity: portfolio_value,
            timestamp: fill.market_meta.timestamp
        });

        Ok(())
    }
}

impl Default for Position {
    fn default() -> Self {
        Self {
            meta: Default::default(),
            exchange: String::from("BINANCE"),
            symbol: String::from("ETH-USD"),
            direction: Direction::default(),
            quantity: 1.0,
            enter_fees: Default::default(),
            enter_fees_total: 0.0,
            enter_avg_price_gross: 100.0,
            enter_value_gross: 100.0,
            exit_fees: Default::default(),
            exit_fees_total: 0.0,
            exit_avg_price_gross: 0.0,
            exit_value_gross: 0.0,
            current_symbol_price: 100.0,
            current_value_gross: 100.0,
            unreal_profit_loss: 0.0,
            result_profit_loss: 0.0,
        }
    }
}

impl Position {
    /// Returns a [PositionBuilder] instance.
    pub fn builder() -> PositionBuilder {
        PositionBuilder::new()
    }

    /// Calculates the [Position::enter_avg_price_gross] or [Position::exit_avg_price_gross] of
    /// a [FillEvent].
    pub fn calculate_avg_price_gross(fill: &FillEvent) -> f64 {
        (fill.fill_value_gross / fill.quantity).abs()
    }

    /// Determine the [Position] entry [Direction] by analysing the input [FillEvent].
    pub fn parse_entry_direction(fill: &FillEvent) -> Result<Direction, PortfolioError> {
        match fill.decision {
            Decision::Long if fill.quantity.is_sign_positive() => Ok(Direction::Long),
            Decision::Short if fill.quantity.is_sign_negative() => Ok(Direction::Short),
            Decision::CloseLong | Decision::CloseShort => Err(PortfolioError::CannotEnterPositionWithExitFill),
            _ => Err(PortfolioError::ParseEntryDirectionError)
        }
    }

    /// Calculate the approximate [Position::unreal_profit_loss] of a [Position].
    pub fn calculate_unreal_profit_loss(&self) -> f64 {
        let approx_total_fees = self.enter_fees_total * 2.0;

        match self.direction {
            Direction::Long => self.current_value_gross - self.enter_value_gross - approx_total_fees,
            Direction::Short => self.enter_value_gross - self.current_value_gross - approx_total_fees,
        }
    }

    /// Calculate the exact [Position::result_profit_loss] of a [Position].
    pub fn calculate_result_profit_loss(&self) -> f64 {
        let total_fees = self.enter_fees_total + self.exit_fees_total;

        match self.direction {
            Direction::Long => self.exit_value_gross - self.enter_value_gross - total_fees,
            Direction::Short => self.enter_value_gross - self.exit_value_gross - total_fees,
        }
    }

    /// Calculate the PnL return of a closed [Position] - assumed [Position::result_profit_loss] is
    /// appropriately calculated.
    pub fn calculate_profit_loss_return(&self) -> f64 {
        self.result_profit_loss / self.enter_value_gross
    }
}

/// Builder to construct [Position] instances.
#[derive(Debug, Default)]
pub struct PositionBuilder {
    pub meta: Option<PositionMeta>,
    pub exchange: Option<String>,
    pub symbol: Option<String>,
    pub direction: Option<Direction>,
    pub quantity: Option<f64>,
    pub enter_fees: Option<Fees>,
    pub enter_fees_total: Option<FeeAmount>,
    pub enter_avg_price_gross: Option<f64>,
    pub enter_value_gross: Option<f64>,
    pub exit_fees: Option<Fees>,
    pub exit_fees_total: Option<FeeAmount>,
    pub exit_avg_price_gross: Option<f64>,
    pub exit_value_gross: Option<f64>,
    pub current_symbol_price: Option<f64>,
    pub current_value_gross: Option<f64>,
    pub unreal_profit_loss: Option<f64>,
    pub result_profit_loss: Option<f64>,
}

impl PositionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn meta(self, value: PositionMeta) -> Self {
        Self {
            meta: Some(value),
            ..self
        }
    }

    pub fn exchange(self, value: String) -> Self {
        Self {
            exchange: Some(value),
            ..self
        }
    }

    pub fn symbol(self, value: String) -> Self {
        Self {
            symbol: Some(value),
            ..self
        }
    }

    pub fn direction(self, value: Direction) -> Self {
        Self {
            direction: Some(value),
            ..self
        }
    }

    pub fn quantity(self, value: f64) -> Self {
        Self {
            quantity: Some(value),
            ..self
        }
    }

    pub fn enter_fees(self, value: Fees) -> Self {
        Self {
            enter_fees: Some(value),
            ..self
        }
    }

    pub fn enter_fees_total(self, value: FeeAmount) -> Self {
        Self {
            enter_fees_total: Some(value),
            ..self
        }
    }

    pub fn enter_avg_price_gross(self, value: f64) -> Self {
        Self {
            enter_avg_price_gross: Some(value),
            ..self
        }
    }

    pub fn enter_value_gross(self, value: f64) -> Self {
        Self {
            enter_value_gross: Some(value),
            ..self
        }
    }

    pub fn exit_fees(self, value: Fees) -> Self {
        Self {
            exit_fees: Some(value),
            ..self
        }
    }

    pub fn exit_fees_total(self, value: FeeAmount) -> Self {
        Self {
            exit_fees_total: Some(value),
            ..self
        }
    }

    pub fn exit_avg_price_gross(self, value: f64) -> Self {
        Self {
            exit_avg_price_gross: Some(value),
            ..self
        }
    }

    pub fn exit_value_gross(self, value: f64) -> Self {
        Self {
            exit_value_gross: Some(value),
            ..self
        }
    }

    pub fn current_symbol_price(self, value: f64) -> Self {
        Self {
            current_symbol_price: Some(value),
            ..self
        }
    }

    pub fn current_value_gross(self, value: f64) -> Self {
        Self {
            current_value_gross: Some(value),
            ..self
        }
    }

    pub fn unreal_profit_loss(self, value: f64) -> Self {
        Self {
            unreal_profit_loss: Some(value),
            ..self
        }
    }

    pub fn result_profit_loss(self, value: f64) -> Self {
        Self {
            result_profit_loss: Some(value),
            ..self
        }
    }

    pub fn build(self) -> Result<Position, PortfolioError> {
        let meta = self.meta.ok_or(PortfolioError::BuilderIncomplete)?;
        let exchange = self.exchange.ok_or(PortfolioError::BuilderIncomplete)?;
        let symbol = self.symbol.ok_or(PortfolioError::BuilderIncomplete)?;
        let direction = self.direction.ok_or(PortfolioError::BuilderIncomplete)?;
        let quantity = self.quantity.ok_or(PortfolioError::BuilderIncomplete)?;
        let enter_fees = self.enter_fees.ok_or(PortfolioError::BuilderIncomplete)?;
        let enter_fees_total = self.enter_fees_total.ok_or(PortfolioError::BuilderIncomplete)?;
        let enter_avg_price_gross = self.enter_avg_price_gross.ok_or(PortfolioError::BuilderIncomplete)?;
        let enter_value_gross = self.enter_value_gross.ok_or(PortfolioError::BuilderIncomplete)?;
        let exit_fees = self.exit_fees.ok_or(PortfolioError::BuilderIncomplete)?;
        let exit_fees_total = self.exit_fees_total.ok_or(PortfolioError::BuilderIncomplete)?;
        let exit_avg_price_gross = self.exit_avg_price_gross.ok_or(PortfolioError::BuilderIncomplete)?;
        let exit_value_gross = self.exit_value_gross.ok_or(PortfolioError::BuilderIncomplete)?;
        let current_symbol_price = self.current_symbol_price.ok_or(PortfolioError::BuilderIncomplete)?;
        let current_value_gross = self.current_value_gross.ok_or(PortfolioError::BuilderIncomplete)?;
        let unreal_profit_loss = self.unreal_profit_loss.ok_or(PortfolioError::BuilderIncomplete)?;
        let result_profit_loss = self.result_profit_loss.ok_or(PortfolioError::BuilderIncomplete)?;

        Ok(Position {
            meta,
            exchange,
            symbol,
            direction,
            quantity,
            enter_fees,
            enter_fees_total,
            enter_avg_price_gross,
            enter_value_gross,
            exit_fees,
            exit_fees_total,
            exit_avg_price_gross,
            exit_value_gross,
            current_symbol_price,
            current_value_gross,
            unreal_profit_loss,
            result_profit_loss
        })
    }
}

/// Metadata detailing the trace UUIDs & timestamps associated with entering, updating & exiting
/// a [Position].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionMeta {
    /// Trace UUID of the MarketEvent that triggered the entering of this [Position].
    pub enter_trace_id: Uuid,

    /// MarketEvent Bar timestamp that triggered the entering of this [Position].
    pub enter_bar_timestamp: DateTime<Utc>,

    /// Trace UUID of the last event to trigger a [Position] update.
    pub last_update_trace_id: Uuid,

    /// Event timestamp of the last event to trigger a [Position] update.9
    pub last_update_timestamp: DateTime<Utc>,

    /// Trace UUID of the MarketEvent that triggered the exiting of this [Position].
    pub exit_trace_id: Option<Uuid>,

    /// MarketEvent Bar timestamp that triggered the exiting of this [Position].
    pub exit_bar_timestamp: Option<DateTime<Utc>>,

    /// Portfolio [EquityPoint] calculated after the [Position] exit.
    pub exit_equity_point: Option<EquityPoint>,
}

impl Default for PositionMeta {
    fn default() -> Self {
        Self {
            enter_trace_id: Default::default(),
            enter_bar_timestamp: Utc::now(),
            last_update_trace_id: Default::default(),
            last_update_timestamp: Utc::now(),
            exit_trace_id: None,
            exit_bar_timestamp: None,
            exit_equity_point: None
        }
    }
}

/// Equity value at a point in time.
#[derive(Debug, Clone, PartialOrd, PartialEq, Serialize, Deserialize)]
pub struct EquityPoint {
    pub equity: f64,
    pub timestamp: DateTime<Utc>,
}

impl Default for EquityPoint {
    fn default() -> Self {
        Self {
            equity: 0.0,
            timestamp: Utc::now(),
        }
    }
}

impl EquityPoint {
    /// Updates using the input [Position]'s PnL & associated timestamp.
    pub fn update(&mut self, position: &Position) {
        match position.meta.exit_bar_timestamp {
            None => {
                // Position is not exited
                self.equity += position.unreal_profit_loss;
                self.timestamp = position.meta.last_update_timestamp;
            },
            Some(exit_timestamp) => {
                self.equity += position.result_profit_loss;
                self.timestamp = exit_timestamp;
            }
        }
    }
}

/// Direction of the [Position] when it was opened, Long or Short.
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Long,
    Short,
}

impl Default for Direction {
    fn default() -> Self {
        Self::Long
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::signal::Decision;
    use chrono::Duration;
    use std::ops::Add;

    #[test]
    fn enter_new_position_with_long_decision_provided() {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Long;
        input_fill.quantity = 1.0;
        input_fill.fill_value_gross = 100.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        let position = Position::enter(&input_fill).unwrap();

        assert_eq!(position.direction, Direction::Long);
        assert_eq!(position.quantity, input_fill.quantity);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, input_fill.fees.exchange);
        assert_eq!(position.enter_fees.slippage, input_fill.fees.slippage);
        assert_eq!(position.enter_fees.network, input_fill.fees.network);
        assert_eq!(position.enter_avg_price_gross, (input_fill.fill_value_gross / input_fill.quantity.abs()));
        assert_eq!(position.enter_value_gross, input_fill.fill_value_gross);
        assert_eq!(position.exit_fees_total, 0.0);
        assert_eq!(position.exit_avg_price_gross, 0.0);
        assert_eq!(position.exit_value_gross, 0.0);
        assert_eq!(position.current_symbol_price, (input_fill.fill_value_gross / input_fill.quantity.abs()));
        assert_eq!(position.current_value_gross, input_fill.fill_value_gross);
        assert_eq!(position.unreal_profit_loss, -6.0); // -2 * enter_fees_total
        assert_eq!(position.result_profit_loss, 0.0);
    }

    #[test]
    fn enter_new_position_with_short_decision_provided() {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Short;
        input_fill.quantity = -1.0;
        input_fill.fill_value_gross = 100.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        let position = Position::enter(&input_fill).unwrap();

        assert_eq!(position.direction, Direction::Short);
        assert_eq!(position.quantity, input_fill.quantity);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, input_fill.fees.exchange);
        assert_eq!(position.enter_fees.slippage, input_fill.fees.slippage);
        assert_eq!(position.enter_fees.network, input_fill.fees.network);
        assert_eq!(position.enter_avg_price_gross, (input_fill.fill_value_gross / input_fill.quantity.abs()));
        assert_eq!(position.enter_value_gross, input_fill.fill_value_gross);
        assert_eq!(position.exit_fees_total, 0.0);
        assert_eq!(position.exit_avg_price_gross, 0.0);
        assert_eq!(position.exit_value_gross, 0.0);
        assert_eq!(position.current_symbol_price, (input_fill.fill_value_gross / input_fill.quantity.abs()));
        assert_eq!(position.current_value_gross, input_fill.fill_value_gross);
        assert_eq!(position.unreal_profit_loss, -6.0); // -2 * enter_fees_total
        assert_eq!(position.result_profit_loss, 0.0);
    }

    #[test]
    fn enter_new_position_and_return_err_with_close_long_decision_provided() -> Result<(), String> {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::CloseLong;
        input_fill.quantity = -1.0;
        input_fill.fill_value_gross = 100.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        if let Err(_) = Position::enter(&input_fill) {
            Ok(())
        }
        else {
            Err(String::from("Position::enter did not return an Err and it should have."))
        }
    }

    #[test]
    fn enter_new_position_and_return_err_with_close_short_decision_provided() -> Result<(), String> {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::CloseShort;
        input_fill.quantity = 1.0;
        input_fill.fill_value_gross = 100.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        if let Err(_) = Position::enter(&input_fill) {
            Ok(())
        }
        else {
            Err(String::from("Position::enter did not return an Err and it should have."))
        }
    }

    #[test]
    fn enter_new_position_and_return_err_with_negative_quantity_long_decision_provided() -> Result<(), String> {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Long;
        input_fill.quantity = -1.0;
        input_fill.fill_value_gross = 100.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        if let Err(_) = Position::enter(&input_fill) {
            Ok(())
        }
        else {
            Err(String::from("Position::enter did not return an Err and it should have."))
        }
    }

    #[test]
    fn enter_new_position_and_return_err_with_positive_quantity_short_decision_provided() -> Result<(), String> {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Short;
        input_fill.quantity = 1.0;
        input_fill.fill_value_gross = 100.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        if let Err(_) = Position::enter(&input_fill) {
            Ok(())
        }
        else {
            Err(String::from("Position::enter did not return an Err and it should have."))
        }
    }

    #[test]
    fn update_long_position_so_unreal_pnl_increases() {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Long;
        position.quantity = 1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input MarketEvent
        let mut input_market = MarketEvent::default();
        input_market.bar.close = 200.0; // +100.0 higher than current_symbol_price

        // Update Position
        position.update(&input_market);

        // Assert update hasn't changed fields that are constant after creation
        assert_eq!(position.direction, Direction::Long);
        assert_eq!(position.quantity, 1.0);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, 1.0);
        assert_eq!(position.enter_fees.slippage, 1.0);
        assert_eq!(position.enter_fees.network, 1.0);
        assert_eq!(position.enter_avg_price_gross, 100.0);
        assert_eq!(position.enter_value_gross, 100.0);

        // Assert updated fields are correct
        assert_eq!(position.current_symbol_price, input_market.bar.close);
        assert_eq!(position.current_value_gross, input_market.bar.close * position.quantity.abs());

        // current_value_gross - enter_value_gross - approx_total_fees
        assert_eq!(position.unreal_profit_loss, (200.0 - 100.0 - 6.0));
    }

    #[test]
    fn update_long_position_so_unreal_pnl_decreases() {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Long;
        position.quantity = 1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input MarketEvent
        let mut input_market = MarketEvent::default();
        input_market.bar.close = 50.0; // -50.0 lower than current_symbol_price

        // Update Position
        position.update(&input_market);

        // Assert update hasn't changed fields that are constant after creation
        assert_eq!(position.direction, Direction::Long);
        assert_eq!(position.quantity, 1.0);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, 1.0);
        assert_eq!(position.enter_fees.slippage, 1.0);
        assert_eq!(position.enter_fees.network, 1.0);
        assert_eq!(position.enter_avg_price_gross, 100.0);
        assert_eq!(position.enter_value_gross, 100.0);

        // Assert updated fields are correct
        assert_eq!(position.current_symbol_price, input_market.bar.close);
        assert_eq!(position.current_value_gross, input_market.bar.close * position.quantity.abs());

        // current_value_gross - enter_value_gross - approx_total_fees
        assert_eq!(position.unreal_profit_loss, (50.0 - 100.0 - 6.0));
    }

    #[test]
    fn update_short_position_so_unreal_pnl_increases() {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Short;
        position.quantity = -1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input MarketEvent
        let mut input_market = MarketEvent::default();
        input_market.bar.close = 50.0; // -50.0 lower than current_symbol_price

        // Update Position
        position.update(&input_market);

        // Assert update hasn't changed fields that are constant after creation
        assert_eq!(position.direction, Direction::Short);
        assert_eq!(position.quantity, -1.0);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, 1.0);
        assert_eq!(position.enter_fees.slippage, 1.0);
        assert_eq!(position.enter_fees.network, 1.0);
        assert_eq!(position.enter_avg_price_gross, 100.0);
        assert_eq!(position.enter_value_gross, 100.0);

        // Assert updated fields are correct
        assert_eq!(position.current_symbol_price, input_market.bar.close);
        assert_eq!(position.current_value_gross, input_market.bar.close * position.quantity.abs());

        // enter_value_gross - current_value_gross - approx_total_fees
        assert_eq!(position.unreal_profit_loss, (100.0 - 50.0 - 6.0));
    }

    #[test]
    fn update_short_position_so_unreal_pnl_decreases() {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Short;
        position.quantity = -1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input MarketEvent
        let mut input_market = MarketEvent::default();
        input_market.bar.close = 200.0; // +100.0 higher than current_symbol_price

        // Update Position
        position.update(&input_market);

        // Assert update hasn't changed fields that are constant after creation
        assert_eq!(position.direction, Direction::Short);
        assert_eq!(position.quantity, -1.0);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, 1.0);
        assert_eq!(position.enter_fees.slippage, 1.0);
        assert_eq!(position.enter_fees.network, 1.0);
        assert_eq!(position.enter_avg_price_gross, 100.0);
        assert_eq!(position.enter_value_gross, 100.0);

        // Assert updated fields are correct
        assert_eq!(position.current_symbol_price, input_market.bar.close);
        assert_eq!(position.current_value_gross, input_market.bar.close * position.quantity.abs());

        // enter_value_gross - current_value_gross - approx_total_fees
        assert_eq!(position.unreal_profit_loss, (100.0 - 200.0 - 6.0));
    }

    #[test]
    fn exit_long_position_with_positive_real_pnl() {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Long;
        position.quantity = 1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input Portfolio Current Value
        let current_value = 10000.0;

        // Input FillEvent
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::CloseLong;
        input_fill.quantity = -position.quantity;
        input_fill.fill_value_gross = 200.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        // Exit Position
        position.exit(current_value, &input_fill).unwrap();

        // Assert exit hasn't changed fields that are constant after creation
        assert_eq!(position.direction, Direction::Long);
        assert_eq!(position.quantity, 1.0);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, 1.0);
        assert_eq!(position.enter_fees.slippage, 1.0);
        assert_eq!(position.enter_fees.network, 1.0);
        assert_eq!(position.enter_avg_price_gross, 100.0);
        assert_eq!(position.enter_value_gross, 100.0);

        // Assert fields changed by exit are correct
        assert_eq!(position.exit_fees_total, 3.0);
        assert_eq!(position.exit_fees.exchange, 1.0);
        assert_eq!(position.exit_fees.slippage, 1.0);
        assert_eq!(position.exit_fees.network, 1.0);
        assert_eq!(position.exit_value_gross, input_fill.fill_value_gross);
        assert_eq!(position.exit_avg_price_gross, input_fill.fill_value_gross / input_fill.quantity.abs());

        // exit_value_gross - enter_value_gross - total_fees
        assert_eq!(position.result_profit_loss, (200.0 - 100.0 - 6.0));
        assert_eq!(position.unreal_profit_loss, (200.0 - 100.0 - 6.0));

        // Assert EquityPoint on Exit is correct
        assert_eq!(position.meta.exit_equity_point.unwrap().equity, current_value + (200.0 - 100.0 - 6.0))
    }

    #[test]
    fn exit_long_position_with_negative_real_pnl() {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Long;
        position.quantity = 1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input Portfolio Current Value
        let current_value = 10000.0;

        // Input FillEvent
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::CloseLong;
        input_fill.quantity = -position.quantity;
        input_fill.fill_value_gross = 50.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        // Exit Position
        position.exit(current_value, &input_fill).unwrap();

        // Assert exit hasn't changed fields that are constant after creation
        assert_eq!(position.direction, Direction::Long);
        assert_eq!(position.quantity, 1.0);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, 1.0);
        assert_eq!(position.enter_fees.slippage, 1.0);
        assert_eq!(position.enter_fees.network, 1.0);
        assert_eq!(position.enter_avg_price_gross, 100.0);
        assert_eq!(position.enter_value_gross, 100.0);

        // Assert fields changed by exit are correct
        assert_eq!(position.exit_fees_total, 3.0);
        assert_eq!(position.exit_fees.exchange, 1.0);
        assert_eq!(position.exit_fees.slippage, 1.0);
        assert_eq!(position.exit_fees.network, 1.0);
        assert_eq!(position.exit_value_gross, input_fill.fill_value_gross);
        assert_eq!(position.exit_avg_price_gross, input_fill.fill_value_gross / input_fill.quantity.abs());

        // exit_value_gross - enter_value_gross - total_fees
        assert_eq!(position.result_profit_loss, (50.0 - 100.0 - 6.0));
        assert_eq!(position.unreal_profit_loss, (50.0 - 100.0 - 6.0));

        // Assert EquityPoint on Exit is correct
        assert_eq!(position.meta.exit_equity_point.unwrap().equity, current_value + (50.0 - 100.0 - 6.0))
    }

    #[test]
    fn exit_short_position_with_positive_real_pnl() {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Short;
        position.quantity = -1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input Portfolio Current Value
        let current_value = 10000.0;

        // Input FillEvent
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::CloseShort;
        input_fill.quantity = -position.quantity;
        input_fill.fill_value_gross = 50.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        // Exit Position
        position.exit(current_value, &input_fill).unwrap();

        // Assert exit hasn't changed fields that are constant after creation
        assert_eq!(position.direction, Direction::Short);
        assert_eq!(position.quantity, -1.0);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, 1.0);
        assert_eq!(position.enter_fees.slippage, 1.0);
        assert_eq!(position.enter_fees.network, 1.0);
        assert_eq!(position.enter_avg_price_gross, 100.0);
        assert_eq!(position.enter_value_gross, 100.0);

        // Assert fields changed by exit are correct
        assert_eq!(position.exit_fees_total, 3.0);
        assert_eq!(position.exit_fees.exchange, 1.0);
        assert_eq!(position.exit_fees.slippage, 1.0);
        assert_eq!(position.exit_fees.network, 1.0);
        assert_eq!(position.exit_value_gross, input_fill.fill_value_gross);
        assert_eq!(position.exit_avg_price_gross, input_fill.fill_value_gross / input_fill.quantity.abs());

        // enter_value_gross - current_value_gross - approx_total_fees
        assert_eq!(position.result_profit_loss, (100.0 - 50.0 - 6.0));
        assert_eq!(position.unreal_profit_loss, (100.0 - 50.0 - 6.0));

        // Assert EquityPoint on Exit is correct
        assert_eq!(position.meta.exit_equity_point.unwrap().equity, current_value + (100.0 - 50.0 - 6.0))
    }

    #[test]
    fn exit_short_position_with_negative_real_pnl() {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Short;
        position.quantity = -1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input Portfolio Current Value
        let current_value = 10000.0;

        // Input FillEvent
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::CloseShort;
        input_fill.quantity = -position.quantity;
        input_fill.fill_value_gross = 200.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        // Exit Position
        position.exit(current_value, &input_fill).unwrap();

        // Assert exit hasn't changed fields that are constant after creation
        assert_eq!(position.direction, Direction::Short);
        assert_eq!(position.quantity, -1.0);
        assert_eq!(position.enter_fees_total, 3.0);
        assert_eq!(position.enter_fees.exchange, 1.0);
        assert_eq!(position.enter_fees.slippage, 1.0);
        assert_eq!(position.enter_fees.network, 1.0);
        assert_eq!(position.enter_avg_price_gross, 100.0);
        assert_eq!(position.enter_value_gross, 100.0);

        // Assert fields changed by exit are correct
        assert_eq!(position.exit_fees_total, 3.0);
        assert_eq!(position.exit_fees.exchange, 1.0);
        assert_eq!(position.exit_fees.slippage, 1.0);
        assert_eq!(position.exit_fees.network, 1.0);
        assert_eq!(position.exit_value_gross, input_fill.fill_value_gross);
        assert_eq!(position.exit_avg_price_gross, input_fill.fill_value_gross / input_fill.quantity.abs());

        // enter_value_gross - current_value_gross - approx_total_fees
        assert_eq!(position.result_profit_loss, (100.0 - 200.0 - 6.0));
        assert_eq!(position.unreal_profit_loss, (100.0 - 200.0 - 6.0));

        // Assert EquityPoint on Exit is correct
        assert_eq!(position.meta.exit_equity_point.unwrap().equity, current_value + (100.0 - 200.0 - 6.0))
    }

    #[test]
    fn exit_long_position_with_long_entry_fill_and_return_err() -> Result<(), String> {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Short;
        position.quantity = -1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input Portfolio Current Value
        let current_value = 10000.0;

        // Input FillEvent
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Long;
        input_fill.quantity = position.quantity;
        input_fill.fill_value_gross = 200.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        // Exit Position
        if let Err(_) = position.exit(current_value, &input_fill) {
            Ok(())
        }
        else {
            Err(String::from("Position::exit did not return an Err and it should have."))
        }
    }

    #[test]
    fn exit_short_position_with_short_entry_fill_and_return_err() -> Result<(), String> {
        // Initial Position
        let mut position = Position::default();
        position.direction = Direction::Short;
        position.quantity = -1.0;
        position.enter_fees_total = 3.0;
        position.enter_fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };
        position.enter_avg_price_gross = 100.0;
        position.enter_value_gross = 100.0;
        position.current_symbol_price = 100.0;
        position.current_value_gross = 100.0;
        position.unreal_profit_loss = position.enter_fees_total * -2.0;

        // Input Portfolio Current Value
        let current_value = 10000.0;

        // Input FillEvent
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Short;
        input_fill.quantity = -position.quantity;
        input_fill.fill_value_gross = 200.0;
        input_fill.fees = Fees {
            exchange: 1.0,
            slippage: 1.0,
            network: 1.0
        };

        // Exit Position
        if let Err(_) = position.exit(current_value, &input_fill) {
            Ok(())
        }
        else {
            Err(String::from("Position::exit did not return an Err and it should have."))
        }
    }

    #[test]
    fn calculate_avg_price_gross_correctly_with_positive_quantity() {
        let mut input_fill = FillEvent::default();
        input_fill.fill_value_gross = 1000.0;
        input_fill.quantity = 1.0;

        let actual = Position::calculate_avg_price_gross(&input_fill);

        assert_eq!(actual, 1000.0)
    }

    #[test]
    fn calculate_avg_price_gross_correctly_with_negative_quantity() {
        let mut input_fill = FillEvent::default();
        input_fill.fill_value_gross = 1000.0;
        input_fill.quantity = -1.0;

        let actual = Position::calculate_avg_price_gross(&input_fill);

        assert_eq!(actual, 1000.0)
    }

    #[test]
    fn parse_entry_direction_as_long_with_positive_quantity_long_decision_provided() {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Long;
        input_fill.quantity = 1.0;

        let actual = Position::parse_entry_direction(&input_fill).unwrap();

        assert_eq!(actual, Direction::Long)
    }

    #[test]
    fn parse_entry_direction_as_short_with_negative_quantity_short_decision_provided() {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Short;
        input_fill.quantity = -1.0;

        let actual = Position::parse_entry_direction(&input_fill).unwrap();

        assert_eq!(actual, Direction::Short)
    }

    #[test]
    fn parse_entry_direction_and_return_err_with_close_long_decision_provided() -> Result<(), String> {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::CloseLong;
        input_fill.quantity = -1.0;

        if let Err(_) = Position::parse_entry_direction(&input_fill) {
            Ok(())
        }
        else {
            Err(String::from("parse_entry_direction() did not return an Err & it should."))
        }
    }

    #[test]
    fn parse_entry_direction_and_return_err_with_close_short_decision_provided() -> Result<(), String> {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::CloseShort;
        input_fill.quantity = 1.0;

        if let Err(_) = Position::parse_entry_direction(&input_fill) {
            Ok(())
        }
        else {
            Err(String::from("parse_entry_direction() did not return an Err & it should."))
        }
    }

    #[test]
    fn parse_entry_direction_and_return_err_with_negative_quantity_long_decision_provided() -> Result<(), String> {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Long;
        input_fill.quantity = -1.0;

        if let Err(_) = Position::parse_entry_direction(&input_fill) {
            Ok(())
        }
        else {
            Err(String::from("parse_entry_direction() did not return an Err & it should."))
        }
    }

    #[test]
    fn parse_entry_direction_and_return_err_with_positive_quantity_short_decision_provided() -> Result<(), String> {
        let mut input_fill = FillEvent::default();
        input_fill.decision = Decision::Short;
        input_fill.quantity = 1.0;

        if let Err(_) = Position::parse_entry_direction(&input_fill) {
            Ok(())
        }
        else {
            Err(String::from("parse_entry_direction() did not return an Err & it should."))
        }
    }

    #[test]
    fn calculate_unreal_profit_loss() {
        let mut long_win = Position::default(); // Expected PnL = +8.0
        long_win.direction = Direction::Long;
        long_win.enter_value_gross = 100.0;
        long_win.enter_fees_total = 1.0;
        long_win.current_value_gross = 110.0;

        let mut long_lose = Position::default(); // Expected PnL = -12.0
        long_lose.direction = Direction::Long;
        long_lose.enter_value_gross = 100.0;
        long_lose.enter_fees_total = 1.0;
        long_lose.current_value_gross = 90.0;

        let mut short_win = Position::default(); // Expected PnL = +8.0
        short_win.direction = Direction::Short;
        short_win.enter_value_gross = 100.0;
        short_win.enter_fees_total = 1.0;
        short_win.current_value_gross = 90.0;

        let mut short_lose = Position::default(); // Expected PnL = -12.0
        short_lose.direction = Direction::Short;
        short_lose.enter_value_gross = 100.0;
        short_lose.enter_fees_total = 1.0;
        short_lose.current_value_gross = 110.0;

        let inputs = vec![long_win, long_lose, short_win, short_lose];

        let expected_pnl = vec![8.0, -12.0, 8.0, -12.0];

        for (position, expected) in inputs.into_iter().zip(expected_pnl.into_iter()) {
            let actual = position.calculate_unreal_profit_loss();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn calculate_real_profit_loss() {
        let mut long_win = Position::default(); // Expected PnL = +18.0
        long_win.direction = Direction::Long;
        long_win.enter_value_gross = 100.0;
        long_win.enter_fees_total = 1.0;
        long_win.exit_value_gross = 120.0;
        long_win.exit_fees_total = 1.0;

        let mut long_lose = Position::default(); // Expected PnL = -22.0
        long_lose.direction = Direction::Long;
        long_lose.enter_value_gross = 100.0;
        long_lose.enter_fees_total = 1.0;
        long_lose.exit_value_gross = 80.0;
        long_lose.exit_fees_total = 1.0;

        let mut short_win = Position::default(); // Expected PnL = +18.0
        short_win.direction = Direction::Short;
        short_win.enter_value_gross = 100.0;
        short_win.enter_fees_total = 1.0;
        short_win.exit_value_gross = 80.0;
        short_win.exit_fees_total = 1.0;

        let mut short_lose = Position::default(); // Expected PnL = -22.0
        short_lose.direction = Direction::Short;
        short_lose.enter_value_gross = 100.0;
        short_lose.enter_fees_total = 1.0;
        short_lose.exit_value_gross = 120.0;
        short_lose.exit_fees_total = 1.0;

        let inputs = vec![long_win, long_lose, short_win, short_lose];

        let expected_pnl = vec![18.0, -22.0, 18.0, -22.0];

        for (position, expected) in inputs.into_iter().zip(expected_pnl.into_iter()) {
            let actual = position.calculate_result_profit_loss();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn calculate_profit_loss_return() {
        let mut long_win = Position::default(); // Expected Return = 0.08
        long_win.direction = Direction::Long;
        long_win.enter_value_gross = 100.0;
        long_win.result_profit_loss = 8.0;

        let mut long_lose = Position::default(); // Expected Return = -0.12
        long_lose.direction = Direction::Long;
        long_lose.enter_value_gross = 100.0;
        long_lose.result_profit_loss = -12.0;

        let mut short_win = Position::default(); // Expected Return = 0.08
        short_win.direction = Direction::Short;
        short_win.enter_value_gross = 100.0;
        short_win.result_profit_loss = 8.0;

        let mut short_lose = Position::default(); // Expected Return = -0.12
        short_lose.direction = Direction::Short;
        short_lose.enter_value_gross = 100.0;
        short_lose.result_profit_loss = -12.0;

        let inputs = vec![long_win, long_lose, short_win, short_lose];

        let expected_return = vec![0.08, -0.12, 0.08, -0.12];

        for (position, expected) in inputs.into_iter().zip(expected_return.into_iter()) {
            let actual = position.calculate_profit_loss_return();
            assert_eq!(actual, expected);
        }
    }

    fn equity_update_position_closed(exit_timestamp: DateTime<Utc>, result_pnl: f64) -> Position {
        let mut position = Position::default();
        position.meta.exit_bar_timestamp = Some(exit_timestamp);
        position.result_profit_loss = result_pnl;
        position
    }

    fn equity_update_position_open(last_update_timestamp: DateTime<Utc>, unreal_pnl: f64) -> Position {
        let mut position = Position::default();
        position.meta.last_update_timestamp = last_update_timestamp;
        position.unreal_profit_loss = unreal_pnl;
        position
    }

    #[test]
    fn equity_point_update() {
        struct TestCase {
            position: Position,
            expected_equity: f64,
            expected_timestamp: DateTime<Utc>,
        }

        let base_timestamp = Utc::now();

        let mut equity_point = EquityPoint {
            equity: 100.0,
            timestamp: base_timestamp
        };

        let test_cases = vec![
            TestCase {
                position: equity_update_position_closed(base_timestamp.add(Duration::days(1)), 10.0),
                expected_equity: 110.0, expected_timestamp: base_timestamp.add(Duration::days(1))
            },
            TestCase {
                position: equity_update_position_open(base_timestamp.add(Duration::days(2)), -10.0),
                expected_equity: 100.0, expected_timestamp: base_timestamp.add(Duration::days(2))
            },
            TestCase {
                position: equity_update_position_closed(base_timestamp.add(Duration::days(3)), -55.9),
                expected_equity: 44.1, expected_timestamp: base_timestamp.add(Duration::days(3))
            },
            TestCase {
                position: equity_update_position_open(base_timestamp.add(Duration::days(4)), 68.7),
                expected_equity: 112.8, expected_timestamp: base_timestamp.add(Duration::days(4))
            },
            TestCase {
                position: equity_update_position_closed(base_timestamp.add(Duration::days(5)), 99999.0),
                expected_equity: 100111.8, expected_timestamp: base_timestamp.add(Duration::days(5))
            },
            TestCase {
                position: equity_update_position_open(base_timestamp.add(Duration::days(5)), 0.2),
                expected_equity: 100112.0, expected_timestamp: base_timestamp.add(Duration::days(5))
            },
        ];

        for test in test_cases {
            equity_point.update(&test.position);
            let equity_diff = equity_point.equity - test.expected_equity;
            assert!(equity_diff < 1e-10);
            assert_eq!(equity_point.timestamp, test.expected_timestamp)
        }
    }
}
