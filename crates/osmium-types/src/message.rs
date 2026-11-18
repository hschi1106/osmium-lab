use std::{error::Error, fmt};

use crate::{Feed, Price, Shares, Symbol, Timestamp};

/// Number of price levels the exchange discloses per side.
pub const BOOK_DEPTH: usize = 5;

/// The marketdata `phase` of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Opening call auction, including its indicative rows before the open.
    OpenAuction,
    /// Continuous trading.
    Continuous,
    /// Intraday call auction: odd lots, periodically matched instruments, and the
    /// auction that ends a volatility interruption.
    PeriodicAuction,
    /// Closing call auction, including its indicative rows.
    CloseAuction,
}

impl Phase {
    /// The marketdata `phase` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAuction => "OPEN_AUCTION",
            Self::Continuous => "CONTINUOUS",
            Self::PeriodicAuction => "PERIODIC_AUCTION",
            Self::CloseAuction => "CLOSE_AUCTION",
        }
    }

    #[must_use]
    pub fn from_marketdata(value: &str) -> Option<Self> {
        [
            Self::OpenAuction,
            Self::Continuous,
            Self::PeriodicAuction,
            Self::CloseAuction,
        ]
        .into_iter()
        .find(|phase| phase.as_str() == value)
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One disclosed limit-price level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub price: Price,
    pub qty: Shares,
}

/// One side of a disclosed book: up to [`BOOK_DEPTH`] levels, best first.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BookSide {
    levels: [Level; BOOK_DEPTH],
    len: u8,
    market_qty: Shares,
}

impl BookSide {
    /// A side with no limit levels and no market orders.
    pub const EMPTY: Self = Self {
        levels: [Level {
            price: Price::from_units(0),
            qty: Shares::ZERO,
        }; BOOK_DEPTH],
        len: 0,
        market_qty: Shares::ZERO,
    };

    /// Builds a side from its levels, best first, and its market-order quantity
    /// (zero when there are no market orders).
    ///
    /// Level prices and quantities must be positive. Price ordering depends on which
    /// side this is and is checked by [`Book::new`].
    pub fn new(levels: &[Level], market_qty: Shares) -> Result<Self, BookSideError> {
        if levels.len() > BOOK_DEPTH {
            return Err(BookSideError::TooManyLevels);
        }
        if levels
            .iter()
            .any(|level| level.price.units() <= 0 || level.qty <= Shares::ZERO)
        {
            return Err(BookSideError::NonPositiveLevel);
        }
        if market_qty < Shares::ZERO {
            return Err(BookSideError::NegativeMarketQty);
        }
        let mut side = Self::EMPTY;
        side.levels[..levels.len()].copy_from_slice(levels);
        side.len = u8::try_from(levels.len()).expect("at most BOOK_DEPTH levels");
        side.market_qty = market_qty;
        Ok(side)
    }

    /// The disclosed limit levels, best first.
    #[must_use]
    pub fn levels(&self) -> &[Level] {
        &self.levels[..usize::from(self.len)]
    }

    #[must_use]
    pub fn best(&self) -> Option<Level> {
        self.levels().first().copied()
    }

    /// Quantity of resting market orders; zero when there are none.
    #[must_use]
    pub const fn market_qty(&self) -> Shares {
        self.market_qty
    }
}

impl fmt::Debug for BookSide {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BookSide")
            .field("levels", &self.levels())
            .field("market_qty", &self.market_qty)
            .finish()
    }
}

/// The five disclosed levels of both sides of one book.
///
/// Bid prices strictly decrease, ask prices strictly increase, and the best bid is
/// below the best ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Book {
    bid: BookSide,
    ask: BookSide,
}

impl Book {
    pub const EMPTY: Self = Self {
        bid: BookSide::EMPTY,
        ask: BookSide::EMPTY,
    };

    pub fn new(bid: BookSide, ask: BookSide) -> Result<Self, BookSideError> {
        let improving = |side: &BookSide, better: fn(Price, Price) -> bool| {
            side.levels()
                .windows(2)
                .all(|pair| better(pair[0].price, pair[1].price))
        };
        if !improving(&bid, |near, far| near > far) || !improving(&ask, |near, far| near < far) {
            return Err(BookSideError::UnorderedLevels);
        }
        if let (Some(best_bid), Some(best_ask)) = (bid.best(), ask.best())
            && best_bid.price >= best_ask.price
        {
            return Err(BookSideError::Crossed);
        }
        Ok(Self { bid, ask })
    }

    #[must_use]
    pub const fn bid(&self) -> &BookSide {
        &self.bid
    }

    #[must_use]
    pub const fn ask(&self) -> &BookSide {
        &self.ask
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookSideError {
    TooManyLevels,
    NonPositiveLevel,
    NegativeMarketQty,
    UnorderedLevels,
    Crossed,
}

impl fmt::Display for BookSideError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyLevels => "more than five levels on one side",
            Self::NonPositiveLevel => "level price or quantity is not positive",
            Self::NegativeMarketQty => "market-order quantity is negative",
            Self::UnorderedLevels => "level prices are not strictly ordered from the best",
            Self::Crossed => "best bid is not below best ask",
        })
    }
}

impl Error for BookSideError {}

/// One fill reported by the exchange: one aggressing order at one price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trade {
    pub price: Price,
    pub qty: Shares,
    /// The book's cumulative traded quantity for the day after this fill.
    pub cum_qty: Option<Shares>,
}

/// Which way prices were moving when the exchange postponed matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DelayDirection {
    /// `delay_state == 1` (趨漲).
    Up,
    /// `delay_state == -1` (趨跌).
    Down,
}

/// What one exchange message carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// A message is a short-lived value; boxing the book would allocate per message.
#[allow(clippy::large_enum_variant)]
pub enum MessageBody {
    /// A real five-level disclosure. With `trade`, the book is the state after that fill.
    Disclosure { book: Book, trade: Option<Trade> },
    /// A fill without a book: an earlier price level of an order that swept several.
    IntermediateTrade(Trade),
    /// A trial match before an auction. The book is what would remain after matching,
    /// not the resting orders.
    Indicative {
        book: Book,
        /// The trial match price and quantity; `None` when nothing would match.
        matched: Option<Level>,
        /// The exchange announced a delayed open; `None` when the source lacks the flag.
        delay_open: Option<bool>,
        /// The exchange announced a delayed close; `None` when the source lacks the flag.
        delay_close: Option<bool>,
    },
    /// Matching is postponed (暫緩撮合); no book is disclosed.
    Delay(DelayDirection),
}

/// One message of one feed: everything the exchange sent under one `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    pub feed: Feed,
    /// The feed's transmission sequence number; unique within a feed and day.
    pub seq: u64,
    pub symbol: Symbol,
    /// Exchange time of the message. Not monotonic: a delay message carries the time
    /// the postponement began.
    pub exch_ts: Timestamp,
    /// The largest `exch_ts` among the feed's messages up to and including this one.
    pub observed_time: Timestamp,
    pub phase: Phase,
    pub body: MessageBody,
}

#[cfg(test)]
mod tests {
    use super::{BOOK_DEPTH, Book, BookSide, BookSideError, Level, Phase};
    use crate::{Price, Shares};

    fn level(price: &str, qty: i64) -> Level {
        Level {
            price: price.parse().unwrap(),
            qty: Shares::new(qty),
        }
    }

    fn side(levels: &[Level]) -> BookSide {
        BookSide::new(levels, Shares::ZERO).unwrap()
    }

    #[test]
    fn phases_use_marketdata_names() {
        for phase in [
            Phase::OpenAuction,
            Phase::Continuous,
            Phase::PeriodicAuction,
            Phase::CloseAuction,
        ] {
            assert_eq!(Phase::from_marketdata(phase.as_str()), Some(phase));
        }
        assert_eq!(Phase::from_marketdata("continuous"), None);
    }

    #[test]
    fn a_side_exposes_only_its_disclosed_levels() {
        let levels = [level("100", 3_000), level("99.5", 1_000)];
        let bid = BookSide::new(&levels, Shares::new(2_000)).unwrap();
        assert_eq!(bid.levels(), levels);
        assert_eq!(bid.best(), Some(levels[0]));
        assert_eq!(bid.market_qty(), Shares::new(2_000));
        assert_eq!(BookSide::EMPTY.levels(), []);
        assert_eq!(BookSide::EMPTY.best(), None);
        assert_eq!(side(&[]), BookSide::EMPTY);
    }

    #[test]
    fn a_side_rejects_values_the_exchange_cannot_disclose() {
        let six = [level("1", 1); BOOK_DEPTH + 1];
        assert_eq!(
            BookSide::new(&six, Shares::ZERO),
            Err(BookSideError::TooManyLevels)
        );
        for bad in [
            level("100", 0),
            level("100", -1),
            Level {
                price: Price::from_units(0),
                qty: Shares::new(1),
            },
        ] {
            assert_eq!(
                BookSide::new(&[bad], Shares::ZERO),
                Err(BookSideError::NonPositiveLevel)
            );
        }
        assert_eq!(
            BookSide::new(&[], Shares::new(-1)),
            Err(BookSideError::NegativeMarketQty)
        );
    }

    #[test]
    fn a_book_requires_ordered_uncrossed_sides() {
        let bid = side(&[level("100", 1), level("99.5", 2)]);
        let ask = side(&[level("100.5", 1), level("101", 2)]);
        let book = Book::new(bid, ask).unwrap();
        assert_eq!(book.bid(), &bid);
        assert_eq!(book.ask(), &ask);
        assert!(Book::new(BookSide::EMPTY, ask).is_ok());
        assert_eq!(Book::new(BookSide::EMPTY, BookSide::EMPTY), Ok(Book::EMPTY));

        // Each side passed as the other is ordered the wrong way.
        assert_eq!(
            Book::new(ask, BookSide::EMPTY),
            Err(BookSideError::UnorderedLevels)
        );
        assert_eq!(
            Book::new(BookSide::EMPTY, bid),
            Err(BookSideError::UnorderedLevels)
        );
        let repeated = side(&[level("100", 1), level("100", 2)]);
        assert_eq!(
            Book::new(repeated, BookSide::EMPTY),
            Err(BookSideError::UnorderedLevels)
        );
        let locked = side(&[level("100", 5)]);
        assert_eq!(Book::new(bid, locked), Err(BookSideError::Crossed));
    }
}
