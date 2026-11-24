//! Cash, positions, credit trading, settlement, corporate actions, fees and tax, as
//! specified in `docs/reference/accounting.md`.
//!
//! The ledger reads no files and keeps no clock: the caller opens each trading day
//! with the facts of that day, asks it about every order before the exchange accepts
//! it, reports every order event, and closes the day with the official closing prices.

pub mod amount;
mod calendar;
mod close;
mod config;
mod corporate;
mod error;
mod facts;
mod journal;
mod ledger;
mod position;
mod tax;

pub use calendar::Calendar;
pub use config::{
    AccountConfig, ActionOverride, CashCheck, Config, CorporateActionConfig, CreditConfig,
    DayTradeConfig, Eligibility, FeeConfig, MissingTerms, Prepay, Uncovered,
};
pub use error::LedgerError;
pub use facts::{
    DayFacts, DayTrade, PreviousDay, SecurityClass, SecurityFacts, short_margin_ratio,
};
pub use journal::{Entry, Item, Realized, RealizedKind};
pub use ledger::{DayClose, DayOpen, Diagnostic, ForcedOrder, ForcedReason, Ledger, Maintenance};
pub use osmium_types::Position;
pub use position::{CashLot, Holding, LotId, MarginLot, OwedLot, ShortLot};

/// The version of the accounting rules (`docs/reference/accounting.md`), including
/// its built-in tables (tax rates, margin ratios, settlement-only days). It changes
/// when the same fills and facts give different entries.
pub const RULES_VERSION: u32 = 2;
