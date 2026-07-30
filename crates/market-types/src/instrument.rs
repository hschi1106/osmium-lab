use std::cmp::Ordering;

use crate::{MarketId, Symbol};

/// Stable instrument identity, independent of metadata and source display labels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstrumentId {
    market: MarketId,
    symbol: Symbol,
}

impl InstrumentId {
    #[must_use]
    pub const fn new(market: MarketId, symbol: Symbol) -> Self {
        Self { market, symbol }
    }

    #[must_use]
    pub const fn market(&self) -> MarketId {
        self.market
    }

    #[must_use]
    pub const fn symbol(&self) -> &Symbol {
        &self.symbol
    }
}

impl PartialOrd for InstrumentId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InstrumentId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.market
            .cmp(&other.market)
            .then_with(|| self.symbol.cmp(&other.symbol))
    }
}
