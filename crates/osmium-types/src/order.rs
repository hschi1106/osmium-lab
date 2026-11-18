//! The vocabulary of simulated orders and what happens to them. The rules that produce
//! these values live in `osmium-matching` and `osmium-ledger`.

use crate::{Board, Feed, Price, Shares, Symbol, Timestamp};

/// Identifies one simulated order within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }

    /// Whether price `a` is better than price `b` for an order on this side: higher
    /// for a buy, lower for a sell.
    #[must_use]
    pub fn is_better(self, a: Price, b: Price) -> bool {
        match self {
            Self::Buy => a > b,
            Self::Sell => a < b,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderPrice {
    Limit(Price),
    Market,
}

impl OrderPrice {
    #[must_use]
    pub const fn limit(self) -> Option<Price> {
        match self {
            Self::Limit(price) => Some(price),
            Self::Market => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeInForce {
    /// Rest of day.
    Rod,
    /// Immediate or cancel.
    Ioc,
    /// Fill or kill.
    Fok,
}

impl TimeInForce {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rod => "ROD",
            Self::Ioc => "IOC",
            Self::Fok => "FOK",
        }
    }
}

/// How an order is settled and what position it opens or closes
/// (`docs/reference/accounting.md` section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TradeKind {
    /// Cash buy or cash sell.
    Cash,
    /// Day-trade sell without inventory, to be bought back the same day.
    CashSellFirst,
    /// Buy on margin.
    MarginBuy,
    /// Sell a margin position.
    MarginSell,
    /// Sell short.
    ShortSell,
    /// Buy back a short position.
    ShortCover,
}

impl TradeKind {
    /// The side this kind implies; `None` for [`TradeKind::Cash`], which is either.
    #[must_use]
    pub const fn side(self) -> Option<Side> {
        match self {
            Self::Cash => None,
            Self::MarginBuy | Self::ShortCover => Some(Side::Buy),
            Self::CashSellFirst | Self::MarginSell | Self::ShortSell => Some(Side::Sell),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cash => "Cash",
            Self::CashSellFirst => "CashSellFirst",
            Self::MarginBuy => "MarginBuy",
            Self::MarginSell => "MarginSell",
            Self::ShortSell => "ShortSell",
            Self::ShortCover => "ShortCover",
        }
    }
}

/// A new order as the strategy sends it. Orders are valid for one trading day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NewOrder {
    pub symbol: Symbol,
    pub board: Board,
    pub side: Side,
    pub kind: TradeKind,
    pub price: OrderPrice,
    pub time_in_force: TimeInForce,
    /// In shares on both boards.
    pub qty: Shares,
}

/// A change to an order that is already at the exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderChange {
    /// Cancel what remains.
    Cancel,
    /// Reduce what remains by `by` shares; the queue position is kept. Reducing by at
    /// least the remaining quantity cancels the order.
    Reduce { by: Shares },
    /// Move the order to another price; it loses its queue position.
    Reprice { price: Price },
}

/// Which rule produced a simulated fill (`docs/reference/matching.md` section 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FillBasis {
    Aggressive,
    PassiveThrough,
    PassiveQueue,
    CrossedPassive,
    AuctionBetter,
    AuctionOpposite,
    AuctionQueue,
    AuctionResidual,
}

impl FillBasis {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aggressive => "Aggressive",
            Self::PassiveThrough => "PassiveThrough",
            Self::PassiveQueue => "PassiveQueue",
            Self::CrossedPassive => "CrossedPassive",
            Self::AuctionBetter => "AuctionBetter",
            Self::AuctionOpposite => "AuctionOpposite",
            Self::AuctionQueue => "AuctionQueue",
            Self::AuctionResidual => "AuctionResidual",
        }
    }
}

/// One simulated fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fill {
    pub price: Price,
    pub qty: Shares,
    pub basis: FillBasis,
    /// The market message that caused the fill; `None` for a fill on arrival.
    pub trigger: Option<(Feed, u64)>,
}

/// Why a request was refused. The first group comes from the exchange rules
/// (`docs/reference/matching.md` section 5), the second from the account checks
/// (`docs/reference/accounting.md` section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectReason {
    UnknownInstrument,
    UnsupportedInstrument,
    MarketClosed,
    InvalidQuantity,
    InvalidTick,
    PriceOutOfLimit,
    OrderTypeNotAllowed,
    /// The order was already completely filled, cancelled or expired.
    TooLate,
    /// The change is not available for this order.
    ChangeNotAllowed,
    /// No order with this identifier was ever sent.
    UnknownOrder,

    KindNotAllowed,
    InsufficientPosition,
    DayTradeNotAllowed,
    ShortNotAllowed,
    ShortPriceRestricted,
    PrepaymentRequired,
    InsufficientCash,
    DailyLimitExceeded,
}

impl RejectReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownInstrument => "UnknownInstrument",
            Self::UnsupportedInstrument => "UnsupportedInstrument",
            Self::MarketClosed => "MarketClosed",
            Self::InvalidQuantity => "InvalidQuantity",
            Self::InvalidTick => "InvalidTick",
            Self::PriceOutOfLimit => "PriceOutOfLimit",
            Self::OrderTypeNotAllowed => "OrderTypeNotAllowed",
            Self::TooLate => "TooLate",
            Self::ChangeNotAllowed => "ChangeNotAllowed",
            Self::UnknownOrder => "UnknownOrder",
            Self::KindNotAllowed => "KindNotAllowed",
            Self::InsufficientPosition => "InsufficientPosition",
            Self::DayTradeNotAllowed => "DayTradeNotAllowed",
            Self::ShortNotAllowed => "ShortNotAllowed",
            Self::ShortPriceRestricted => "ShortPriceRestricted",
            Self::PrepaymentRequired => "PrepaymentRequired",
            Self::InsufficientCash => "InsufficientCash",
            Self::DailyLimitExceeded => "DailyLimitExceeded",
        }
    }
}

/// Which request an [`OrderEventKind::Rejected`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Request {
    New,
    Cancel,
    Reduce,
    Reprice,
}

impl Request {
    /// A stable name for output files.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Cancel => "Cancel",
            Self::Reduce => "Reduce",
            Self::Reprice => "Reprice",
        }
    }
}

/// Why the exchange cancelled a quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancelReason {
    /// The strategy cancelled, or reduced by at least what remained.
    Requested,
    /// The unfilled part of an immediate-or-cancel order.
    IocRemainder,
    /// A fill-or-kill order that could not be filled completely.
    FokUnfilled,
    /// The unfilled part of a market order.
    MarketRemainder,
}

impl CancelReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "Requested",
            Self::IocRemainder => "IocRemainder",
            Self::FokUnfilled => "FokUnfilled",
            Self::MarketRemainder => "MarketRemainder",
        }
    }
}

/// What happened to an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderEventKind {
    /// A new order passed every check and is at the exchange.
    Accepted,
    /// A request was refused; an order that already existed is unchanged.
    Rejected {
        request: Request,
        reason: RejectReason,
    },
    Filled(Fill),
    /// `qty` shares were cancelled and the order is finished.
    Cancelled {
        qty: Shares,
        reason: CancelReason,
    },
    /// The remaining quantity was reduced by `by` and is now `remaining`.
    Reduced {
        by: Shares,
        remaining: Shares,
    },
    /// The order moved to `price`.
    Repriced {
        price: Price,
    },
    /// `qty` shares were still unfilled when the book closed.
    Expired {
        qty: Shares,
    },
    /// The price-stabilisation band kept the order from a quantity it would otherwise
    /// have been filled against. Reported once per order.
    PriceBandLimited,
}

/// One event in the life of an order, at the time it happens at the exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderEvent {
    pub order: OrderId,
    pub time: Timestamp,
    pub kind: OrderEventKind,
}

#[cfg(test)]
mod tests {
    use super::{Side, TradeKind};
    use crate::Price;

    #[test]
    fn a_better_price_is_higher_for_a_buy_and_lower_for_a_sell() {
        let (low, high) = (Price::from_units(100_000), Price::from_units(100_500));
        assert!(Side::Buy.is_better(high, low));
        assert!(!Side::Buy.is_better(low, high));
        assert!(Side::Sell.is_better(low, high));
        assert!(!Side::Sell.is_better(high, low));
        assert!(!Side::Buy.is_better(low, low));
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite(), Side::Buy);
    }

    #[test]
    fn only_cash_orders_leave_the_side_open() {
        assert_eq!(TradeKind::Cash.side(), None);
        assert_eq!(TradeKind::MarginBuy.side(), Some(Side::Buy));
        assert_eq!(TradeKind::ShortCover.side(), Some(Side::Buy));
        assert_eq!(TradeKind::CashSellFirst.side(), Some(Side::Sell));
        assert_eq!(TradeKind::MarginSell.side(), Some(Side::Sell));
        assert_eq!(TradeKind::ShortSell.side(), Some(Side::Sell));
    }
}
