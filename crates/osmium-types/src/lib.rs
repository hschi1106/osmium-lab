//! Exact value types and the marketdata message model shared by every osmium crate.
//!
//! Prices, quantities and times are integers; nothing here uses binary floating point.
//! The types carry no trading rules: those live in the crates that own them.

mod account;
mod instrument;
mod market;
mod message;
mod money;
mod order;
mod price;
mod reference;
mod shares;
mod symbol;
mod time;

pub use account::Position;
pub use instrument::{Currency, DayTradeFlag, Instrument, InstrumentType};
pub use market::{Board, Feed, Venue};
pub use message::{
    BOOK_DEPTH, Book, BookSide, BookSideError, DelayDirection, Level, Message, MessageBody, Phase,
    Trade,
};
pub use money::Money;
pub use order::{
    CancelReason, Fill, FillBasis, NewOrder, OrderChange, OrderEvent, OrderEventKind, OrderId,
    OrderPrice, RejectReason, Request, Side, TimeInForce, TradeKind,
};
pub use price::{ParsePriceError, Price};
pub use reference::{
    CorporateAction, CorporateActionKind, CreditQuota, DailyQuote, Decimal8, MarketEvent,
    MarketEventKind, PerShare,
};
pub use shares::Shares;
pub use symbol::{Symbol, SymbolError};
pub use time::{Date, DateError, Timestamp};
