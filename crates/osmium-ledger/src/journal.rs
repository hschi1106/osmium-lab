//! What the ledger writes down: cash entries and realised results.

use osmium_types::{Date, Money, OrderId, Symbol};

/// What a cash entry is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    /// The trade value of an order's fills of one day: negative for a buy.
    TradeValue,
    Fee,
    Tax,
    /// The fee for borrowing shares to sell short.
    ShortFee,
    /// The broker's loan: received when a margin lot opens, repaid when it closes.
    MarginLoan,
    MarginInterest,
    /// The margin of a short sale: posted when the lot opens, returned when it closes.
    ShortMargin,
    /// The sale proceeds the broker holds for a short lot.
    ShortCollateral,
    ShortInterest,
    CashDividend,
    /// Cash in place of a fraction of a share.
    FractionCash,
    /// Cash returned by a capital reduction.
    CapitalRefund,
    /// The supplementary health-insurance premium on a dividend.
    NhiPremium,
    /// The fee for shares sold without inventory and not bought back.
    BorrowFee,
}

impl Item {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TradeValue => "TradeValue",
            Self::Fee => "Fee",
            Self::Tax => "Tax",
            Self::ShortFee => "ShortFee",
            Self::MarginLoan => "MarginLoan",
            Self::MarginInterest => "MarginInterest",
            Self::ShortMargin => "ShortMargin",
            Self::ShortCollateral => "ShortCollateral",
            Self::ShortInterest => "ShortInterest",
            Self::CashDividend => "CashDividend",
            Self::FractionCash => "FractionCash",
            Self::CapitalRefund => "CapitalRefund",
            Self::NhiPremium => "NhiPremium",
            Self::BorrowFee => "BorrowFee",
        }
    }
}

/// One amount that reaches cash on `settles`. Cash at any time is the initial cash
/// plus every entry that has settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The day the entry was made.
    pub date: Date,
    pub settles: Date,
    pub symbol: Symbol,
    /// The order whose fills caused it; `None` for corporate actions and fees that
    /// belong to no order.
    pub order: Option<OrderId>,
    pub item: Item,
    /// Positive is a receipt.
    pub amount: Money,
    /// The amount rests on an approximation the specification names.
    pub approx: bool,
}

/// How a position was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RealizedKind {
    /// A cash buy and a cash sell of the same day.
    DayTrade,
    /// A sale from cash inventory.
    Cash,
    /// A margin buy and a short sale of the same day.
    CreditOffset,
    Margin,
    Short,
    /// Shares sold without inventory and bought back on a later day.
    SellFirstCover,
    /// What a corporate action realised: a refund above the cost of a lot, or the cost
    /// of a lot that was left without a share.
    CorporateAction,
}

impl RealizedKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DayTrade => "DayTrade",
            Self::Cash => "Cash",
            Self::CreditOffset => "CreditOffset",
            Self::Margin => "Margin",
            Self::Short => "Short",
            Self::SellFirstCover => "SellFirstCover",
            Self::CorporateAction => "CorporateAction",
        }
    }
}

/// The result of closing `shares` shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Realized {
    pub date: Date,
    pub symbol: Symbol,
    pub kind: RealizedKind,
    pub shares: i64,
    /// The day the shares were opened.
    pub opened: Date,
    /// Sale value less the selling fee, tax and short fee.
    pub proceeds: Money,
    /// Purchase value plus the buying fee.
    pub cost: Money,
    /// Interest received (short) or paid (margin, negative).
    pub interest: Money,
    /// `proceeds - cost + interest`.
    pub pnl: Money,
    pub approx: bool,
}
