use std::{error::Error, fmt};

use osmium_types::{CorporateActionKind, Date, Money, Symbol};

/// Why the ledger cannot go on. Each of these ends the run: the ledger does not guess
/// (`docs/reference/accounting.md` section 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// A settlement or trading date outside the calendar is needed.
    OutsideCalendar { needed_after: Date },
    /// `date` is not a trading day of the calendar.
    NotATradingDay { date: Date },
    /// Days must be opened in increasing order and closed before the next one.
    DayOutOfOrder { date: Date },
    /// Cash went below zero on a settlement date (`account.cash_check = "at_settlement"`).
    SettlementShortfall { date: Date, cash: Money },
    /// Shares sold without inventory were not bought back by the close
    /// (`daytrade.uncovered = "error"`).
    UncoveredSellFirst { symbol: Symbol, shares: i64 },
    /// A forced order could not be filled within the allowed number of trading days.
    ForcedOrderUnfilled { symbol: Symbol, shares: i64 },
    /// A position is held through a corporate action whose terms marketdata does not
    /// have; `missing` names what `corporate_actions.overrides` must provide.
    MissingActionTerms {
        symbol: Symbol,
        ex_date: Date,
        action: CorporateActionKind,
        missing: &'static str,
    },
    /// A short position is still open on an ex-date.
    ShortAcrossExDate { symbol: Symbol, ex_date: Date },
    /// No price to value a held position with.
    NoValuationPrice { symbol: Symbol, date: Date },
}

impl fmt::Display for LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideCalendar { needed_after } => write!(
                formatter,
                "the calendar has no settlement day far enough after {needed_after}"
            ),
            Self::NotATradingDay { date } => {
                write!(formatter, "{date} is not a trading day of the calendar")
            }
            Self::DayOutOfOrder { date } => {
                write!(formatter, "{date} was opened or closed out of order")
            }
            Self::SettlementShortfall { date, cash } => {
                write!(formatter, "cash is {cash} after settlement on {date}")
            }
            Self::UncoveredSellFirst { symbol, shares } => write!(
                formatter,
                "{shares} shares of {symbol} sold without inventory were not bought back"
            ),
            Self::ForcedOrderUnfilled { symbol, shares } => write!(
                formatter,
                "a forced order for {shares} shares of {symbol} could not be filled"
            ),
            Self::MissingActionTerms {
                symbol,
                ex_date,
                action,
                missing,
            } => write!(
                formatter,
                "{symbol} {} on {ex_date}: marketdata does not have {missing}",
                action.as_str()
            ),
            Self::ShortAcrossExDate { symbol, ex_date } => write!(
                formatter,
                "a short position in {symbol} is still open on the ex-date {ex_date}"
            ),
            Self::NoValuationPrice { symbol, date } => {
                write!(formatter, "no price to value {symbol} with on {date}")
            }
        }
    }
}

impl Error for LedgerError {}
