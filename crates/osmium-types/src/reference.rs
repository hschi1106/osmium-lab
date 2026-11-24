//! Daily reference facts from marketdata that are not part of the tick stream: closing
//! quotes, credit quotas, trading restrictions and corporate actions. The rules that
//! read them live in `osmium-ledger`.

use crate::{Date, Price, Symbol};

/// The official close of an instrument on one trading date (marketdata `daily`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyQuote {
    pub symbol: Symbol,
    /// `None` when the instrument did not trade.
    pub close: Option<Price>,
    pub close_bid: Option<Price>,
    pub close_ask: Option<Price>,
    pub ref_price: Option<Price>,
    pub limit_down: Option<Price>,
}

/// The margin and short quotas of an instrument on one trading date (marketdata
/// `daily_ext`), in shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditQuota {
    pub symbol: Symbol,
    pub margin_limit: Option<i64>,
    pub short_limit: Option<i64>,
}

/// The marketdata `events.event` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarketEventKind {
    Disposition,
    Halt,
    ShortSuspended,
    DayTradeSuspended,
}

impl MarketEventKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disposition => "disposition",
            Self::Halt => "halt",
            Self::ShortSuspended => "short_suspended",
            Self::DayTradeSuspended => "daytrade_suspended",
        }
    }

    #[must_use]
    pub fn from_marketdata(value: &str) -> Option<Self> {
        [
            Self::Disposition,
            Self::Halt,
            Self::ShortSuspended,
            Self::DayTradeSuspended,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == value)
    }
}

/// One row of marketdata `events`: a restriction on an instrument from `start` to
/// `end`, both inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketEvent {
    pub symbol: Symbol,
    pub kind: MarketEventKind,
    pub start: Date,
    /// `None` when the source does not say when the restriction ended.
    pub end: Option<Date>,
    /// For a disposition, which one it is (1 or 2, marketdata `disp_nth`); `None` when
    /// the source does not say.
    pub nth: Option<u8>,
}

/// An amount per share in units of 1e-10 yuan: issuers state stock dividends to eight
/// decimals of a share per thousand shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PerShare(i64);

impl PerShare {
    pub const UNITS_PER_YUAN: i64 = 10_000_000_000;

    #[must_use]
    pub const fn from_units(units: i64) -> Self {
        Self(units)
    }

    #[must_use]
    pub const fn units(self) -> i64 {
        self.0
    }
}

/// A decimal with eight places in units of 1e-8, as marketdata's `decimal(18,8)`
/// columns state share counts and ratios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal8(i64);

impl Decimal8 {
    pub const UNITS_PER_ONE: i64 = 100_000_000;

    #[must_use]
    pub const fn from_units(units: i64) -> Self {
        Self(units)
    }

    #[must_use]
    pub const fn units(self) -> i64 {
        self.0
    }
}

/// The marketdata `corp_actions.action` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CorporateActionKind {
    Dividend,
    Rights,
    Split,
    ParChange,
    ReverseSplit,
    ReductionLoss,
    ReductionCash,
}

impl CorporateActionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dividend => "dividend",
            Self::Rights => "rights",
            Self::Split => "split",
            Self::ParChange => "par_change",
            Self::ReverseSplit => "reverse_split",
            Self::ReductionLoss => "reduction_loss",
            Self::ReductionCash => "reduction_cash",
        }
    }

    #[must_use]
    pub fn from_marketdata(value: &str) -> Option<Self> {
        [
            Self::Dividend,
            Self::Rights,
            Self::Split,
            Self::ParChange,
            Self::ReverseSplit,
            Self::ReductionLoss,
            Self::ReductionCash,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == value)
    }
}

/// One row of marketdata `corp_actions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorporateAction {
    pub symbol: Symbol,
    pub ex_date: Date,
    pub kind: CorporateActionKind,
    /// The close before the ex-date.
    pub before_price: Price,
    /// The reference price on the ex-date: for a dividend, the price less the
    /// dividends but not the rights of a cash offering on the same day.
    pub ref_price: Price,
    /// The exchange's price after the event, rights included. `None` when marketdata
    /// has none.
    pub after_price: Option<Price>,
    /// Cash dividend per share.
    pub cash_dividend: Option<PerShare>,
    /// The day the cash dividend is paid.
    pub pay_date: Option<Date>,
    /// Shares of a stock dividend per 1,000 shares held.
    pub stock_per_1000: Option<Decimal8>,
    /// What the ex-date takes off the price of one share, for a dividend or rights
    /// row. `None` when marketdata has no value or the value is not positive.
    pub value: Option<PerShare>,
    /// `(new, old)`: `old` shares become `new` shares, for a capital reduction, a
    /// split, a reverse split or a change of par value.
    pub share_ratio: Option<(Decimal8, Decimal8)>,
    /// Cash a capital reduction returns per old share; zero for a reduction that
    /// covers losses.
    pub refund_per_share: Option<PerShare>,
}

#[cfg(test)]
mod tests {
    use super::{CorporateActionKind, MarketEventKind};

    #[test]
    fn kinds_use_marketdata_names() {
        for name in [
            "disposition",
            "halt",
            "short_suspended",
            "daytrade_suspended",
        ] {
            assert_eq!(
                MarketEventKind::from_marketdata(name).unwrap().as_str(),
                name
            );
        }
        assert_eq!(MarketEventKind::from_marketdata("attention"), None);
        for name in [
            "dividend",
            "rights",
            "split",
            "par_change",
            "reverse_split",
            "reduction_loss",
            "reduction_cash",
        ] {
            assert_eq!(
                CorporateActionKind::from_marketdata(name).unwrap().as_str(),
                name
            );
        }
        assert_eq!(CorporateActionKind::from_marketdata("merger"), None);
    }
}
