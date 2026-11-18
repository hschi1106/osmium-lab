use std::fmt;

/// An exchange whose data marketdata publishes as `venue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Venue {
    Twse,
    Tpex,
}

impl Venue {
    pub const ALL: [Self; 2] = [Self::Twse, Self::Tpex];

    /// The marketdata `venue` value, also the `instruments` part name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Twse => "TWSE",
            Self::Tpex => "TPEX",
        }
    }
}

impl fmt::Display for Venue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A trading board. An instrument has one independent order book per board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Board {
    /// Regular trading; marketdata quantities are lots (張).
    Regular,
    /// Intraday odd lots; marketdata quantities are shares.
    OddLot,
}

impl Board {
    /// The marketdata `board` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Regular => "REG",
            Self::OddLot => "ODD",
        }
    }
}

impl fmt::Display for Board {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A marketdata exchange feed: one `part` of the `books` and `trades` tables.
///
/// The declaration order is the feed rank that breaks ties between feeds in replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feed {
    /// TWSE regular board except warrants (Format 6).
    TwseF6,
    /// TPEx regular board except warrants (Format 6).
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

impl Feed {
    /// Every feed in rank order.
    pub const ALL: [Self; 6] = [
        Self::TwseF6,
        Self::TpexF6,
        Self::TwseF17,
        Self::TpexF17,
        Self::TwseF23,
        Self::TpexF23,
    ];

    /// The marketdata `feed` value and part name.
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
    pub const fn venue(self) -> Venue {
        match self {
            Self::TwseF6 | Self::TwseF17 | Self::TwseF23 => Venue::Twse,
            Self::TpexF6 | Self::TpexF17 | Self::TpexF23 => Venue::Tpex,
        }
    }

    #[must_use]
    pub const fn board(self) -> Board {
        match self {
            Self::TwseF23 | Self::TpexF23 => Board::OddLot,
            _ => Board::Regular,
        }
    }

    /// Position in the cross-feed replay order; lower ranks come first at equal times.
    #[must_use]
    pub const fn rank(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for Feed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.part())
    }
}

#[cfg(test)]
mod tests {
    use super::{Board, Feed, Venue};

    #[test]
    fn feeds_map_to_parts_venues_and_boards() {
        for feed in Feed::ALL {
            assert_eq!(Feed::from_part(feed.part()), Some(feed));
            assert!(feed.part().starts_with(feed.venue().as_str()));
        }
        assert_eq!(Feed::from_part("TAIFEX"), None);
        assert_eq!(Feed::TpexF17.venue(), Venue::Tpex);
        assert_eq!(Feed::TpexF17.board(), Board::Regular);
        assert_eq!(Feed::TwseF23.board(), Board::OddLot);
        assert_eq!(Board::OddLot.as_str(), "ODD");
    }

    #[test]
    fn rank_follows_the_replay_specification() {
        let ranks = Feed::ALL.map(|feed| (feed.part(), feed.rank()));
        assert_eq!(
            ranks,
            [
                ("TWSE_F6", 0),
                ("TPEX_F6", 1),
                ("TWSE_F17", 2),
                ("TPEX_F17", 3),
                ("TWSE_F23", 4),
                ("TPEX_F23", 5),
            ]
        );
        assert!(Feed::TwseF6 < Feed::TpexF23);
    }
}
