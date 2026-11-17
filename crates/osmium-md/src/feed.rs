use std::fmt;

use market_state::{CumulativeVolumePolicy, MarketStateProfile};
use market_types::{MarketId, QuantityUnit, SourceFormatId};

/// A marketdata exchange feed: one `part` of the `trades`／`books` tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feed {
    /// TWSE regular-board stocks and ETFs (Format 6).
    TwseF6,
    /// TPEx regular-board stocks and ETFs (Format 6).
    TpexF6,
    /// TWSE warrants (Format 17).
    TwseF17,
    /// TPEx warrants (Format 17).
    TpexF17,
    /// TWSE intraday odd lots (Format 23).
    TwseF23,
    /// TPEx intraday odd lots (Format 23).
    TpexF23,
}

/// Board on which a feed's book is maintained. Regular and odd-lot books are independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Board {
    /// Quantities are trading units (lots, 張).
    Regular,
    /// Quantities are shares.
    OddLot,
}

impl Feed {
    pub const ALL: [Self; 6] = [
        Self::TwseF6,
        Self::TpexF6,
        Self::TwseF17,
        Self::TpexF17,
        Self::TwseF23,
        Self::TpexF23,
    ];

    /// The marketdata `part` name, also used as the event `source_format`.
    #[must_use]
    pub const fn part(self) -> &'static str {
        match self {
            Self::TwseF6 => "TWSE_F6",
            Self::TpexF6 => "TPEX_F6",
            Self::TwseF17 => "TWSE_F17",
            Self::TpexF17 => "TPEX_F17",
            Self::TwseF23 => "TWSE_F23",
            Self::TpexF23 => "TPEX_F23",
        }
    }

    #[must_use]
    pub fn from_part(part: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|feed| feed.part() == part)
    }

    #[must_use]
    pub const fn market(self) -> MarketId {
        match self {
            Self::TwseF6 | Self::TwseF17 | Self::TwseF23 => MarketId::Twse,
            Self::TpexF6 | Self::TpexF17 | Self::TpexF23 => MarketId::Tpex,
        }
    }

    /// The marketdata `venue` value and `instruments` part for this feed.
    #[must_use]
    pub const fn venue(self) -> &'static str {
        match self.market() {
            MarketId::Twse => "TWSE",
            _ => "TPEX",
        }
    }

    #[must_use]
    pub const fn board(self) -> Board {
        match self {
            Self::TwseF23 | Self::TpexF23 => Board::OddLot,
            _ => Board::Regular,
        }
    }

    #[must_use]
    pub const fn quantity_unit(self) -> QuantityUnit {
        match self.board() {
            Board::Regular => QuantityUnit::TradingUnit,
            Board::OddLot => QuantityUnit::Share,
        }
    }

    #[must_use]
    pub fn source_format(self) -> SourceFormatId {
        SourceFormatId::new(self.part()).expect("feed part names are short and non-empty")
    }

    /// Market-state profile accepting every event kind this feed produces.
    #[must_use]
    pub fn state_profile(self) -> MarketStateProfile {
        MarketStateProfile::accepting_all(
            self.market(),
            &[self.source_format()],
            CumulativeVolumePolicy::NonDecreasingWithinSegment {
                unit: self.quantity_unit(),
            },
        )
        .expect("feed market-state profile is valid")
    }
}

impl fmt::Display for Feed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.part())
    }
}

#[cfg(test)]
mod tests {
    use super::{Board, Feed};
    use market_types::{MarketId, QuantityUnit};

    #[test]
    fn part_names_roundtrip_and_odd_lots_use_shares() {
        for feed in Feed::ALL {
            assert_eq!(Feed::from_part(feed.part()), Some(feed));
            assert_eq!(feed.source_format().as_str(), feed.part());
        }
        assert_eq!(Feed::from_part("TAIFEX"), None);
        assert_eq!(Feed::TpexF23.market(), MarketId::Tpex);
        assert_eq!(Feed::TpexF23.board(), Board::OddLot);
        assert_eq!(Feed::TpexF23.quantity_unit(), QuantityUnit::Share);
        assert_eq!(Feed::TwseF17.quantity_unit(), QuantityUnit::TradingUnit);
    }
}
