//! Position lots (`docs/reference/accounting.md` section 2).

use osmium_types::{Date, Money};

/// Identifies a margin or short lot within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LotId(pub u64);

/// Shares held for cash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashLot {
    pub shares: i64,
    /// Trade value plus the buying fee; 0 for shares from a stock dividend.
    pub cost: Money,
    pub acquired: Date,
    /// The day the purchase settles; the shares count as settled inventory from then.
    pub settles: Date,
}

/// Shares bought on margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarginLot {
    pub id: LotId,
    pub shares: i64,
    /// Trade value plus the buying fee.
    pub cost: Money,
    /// What the broker lent.
    pub loan: Money,
    pub opened: Date,
    /// Interest runs from this day.
    pub settles: Date,
    /// The position must be closed by the close of this day.
    pub due: Date,
}

/// Shares sold short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortLot {
    pub id: LotId,
    pub shares: i64,
    /// The margin the account posted.
    pub margin: Money,
    /// The sale proceeds the broker holds: trade value less tax, short fee and fee.
    pub collateral: Money,
    pub opened: Date,
    pub settles: Date,
    pub due: Date,
}

/// Shares sold without inventory and not bought back by the close (section 8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwedLot {
    pub shares: i64,
    /// Trade value less fee and tax.
    pub proceeds: Money,
    pub sold: Date,
}

/// The lots of one instrument, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Holding {
    pub cash: Vec<CashLot>,
    pub margin: Vec<MarginLot>,
    pub short: Vec<ShortLot>,
    pub owed: Vec<OwedLot>,
}

impl Holding {
    #[must_use]
    pub fn cash_shares(&self) -> i64 {
        self.cash.iter().map(|lot| lot.shares).sum()
    }

    /// Cash shares whose purchase has settled by `on`.
    #[must_use]
    pub fn settled_cash_shares(&self, on: Date) -> i64 {
        self.cash
            .iter()
            .filter(|lot| lot.settles <= on)
            .map(|lot| lot.shares)
            .sum()
    }

    #[must_use]
    pub fn margin_shares(&self) -> i64 {
        self.margin.iter().map(|lot| lot.shares).sum()
    }

    #[must_use]
    pub fn short_shares(&self) -> i64 {
        self.short.iter().map(|lot| lot.shares).sum()
    }

    #[must_use]
    pub fn owed_shares(&self) -> i64 {
        self.owed.iter().map(|lot| lot.shares).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cash.is_empty()
            && self.margin.is_empty()
            && self.short.is_empty()
            && self.owed.is_empty()
    }
}
