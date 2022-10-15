use self::{
    consume::Consume,
    terminate::Terminate,
};
use crate::{
    engine::{
        Engine, Trader,
    },
    portfolio::{Initialiser, AccountUpdater, MarketUpdater}
};
use barter_integration::model::{Exchange, Instrument};
use std::{
    collections::HashMap,
    marker::PhantomData,
};

pub mod consume;
pub mod market;
pub mod order;
pub mod account;
pub mod command;
pub mod terminate;

/// [`Initialise`] can transition to one of:
/// a) [`Consumer`]
/// b) [`Terminate`]
pub struct Initialise<Portfolio> {
    pub instruments: HashMap<Exchange, Vec<Instrument>>,
    pub phantom: PhantomData<Portfolio>,
}

impl<Strategy, Portfolio> Trader<Strategy, Initialise<Portfolio>>
where
    Portfolio: Initialiser<Output = Portfolio> + MarketUpdater + AccountUpdater,
{
    pub fn init(self) -> Engine<Strategy, Portfolio> {
        // De-structure Self to access attributes required for Portfolio Initialiser
        let Self {
            mut feed,
            strategy,
            execution_tx,
            state: Initialise { instruments, .. },
        } = self;

        match Portfolio::init(instruments, &execution_tx, &mut feed) {
            // a) Initialise -> Consume
            Ok(portfolio) => {
                Engine::Consume(Trader {
                    feed,
                    strategy,
                    execution_tx,
                    state: Consume {
                        portfolio
                    }
                })
            }
            // b) Initialise -> Terminate
            Err(error) => {
                Engine::Terminate(Trader {
                    feed,
                    strategy,
                    execution_tx,
                    state: Terminate {
                        reason: Err(error)
                    }
                })
            }
        }
    }
}