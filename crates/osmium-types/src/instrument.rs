use crate::{Price, Symbol, Venue};

/// The marketdata `instrument_type` of a TWSE／TPEx instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstrumentType {
    Stock,
    Etf,
    Etn,
    Tdr,
    Reit,
    WarrantCall,
    WarrantPut,
    /// A mixed class (preferred shares, some ETFs and ETNs, convertible bonds, ...).
    Other,
}

impl InstrumentType {
    /// The marketdata `instrument_type` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stock => "STOCK",
            Self::Etf => "ETF",
            Self::Etn => "ETN",
            Self::Tdr => "TDR",
            Self::Reit => "REIT",
            Self::WarrantCall => "WARRANT_C",
            Self::WarrantPut => "WARRANT_P",
            Self::Other => "OTHER",
        }
    }

    #[must_use]
    pub fn from_marketdata(value: &str) -> Option<Self> {
        [
            Self::Stock,
            Self::Etf,
            Self::Etn,
            Self::Tdr,
            Self::Reit,
            Self::WarrantCall,
            Self::WarrantPut,
            Self::Other,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == value)
    }
}

/// The currency an instrument trades in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Currency {
    Twd,
    /// Any other currency (marketdata `currency != "TWD"`).
    Foreign,
}

/// One row of the marketdata `instruments` table: an instrument on one trading date.
///
/// A `None` field is one the source did not provide for that date; nothing is inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instrument {
    pub venue: Venue,
    pub symbol: Symbol,
    pub instrument_type: InstrumentType,
    /// Shares per regular-board lot (張).
    pub shares_per_lot: Option<u32>,
    pub ref_price: Option<Price>,
    pub limit_up: Option<Price>,
    pub limit_down: Option<Price>,
    /// Matching cycle in seconds: `0` continuous trading, `> 0` periodic call auction.
    pub match_interval_s: Option<u32>,
    pub currency: Currency,
    /// The row does not come from the exchange's instrument broadcast: part of it was
    /// derived from daily quotes, and `currency` is a default rather than a fact.
    pub derived: bool,
}

#[cfg(test)]
mod tests {
    use super::InstrumentType;

    #[test]
    fn instrument_types_use_marketdata_names() {
        for name in [
            "STOCK",
            "ETF",
            "ETN",
            "TDR",
            "REIT",
            "WARRANT_C",
            "WARRANT_P",
            "OTHER",
        ] {
            assert_eq!(
                InstrumentType::from_marketdata(name).unwrap().as_str(),
                name
            );
        }
        assert_eq!(InstrumentType::from_marketdata("FUTURE"), None);
    }
}
