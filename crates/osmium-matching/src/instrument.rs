use osmium_book::Method;
use osmium_types::{Currency, Instrument, InstrumentType, Price, Venue};

/// The instrument classes the simulator supports, each with its own tick table
/// (`docs/reference/matching.md` section 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstrumentClass {
    /// Common and preferred shares and depositary receipts.
    Stock,
    /// Exchange-traded funds and notes and real-estate trusts.
    Etf,
    Warrant,
    /// Convertible and exchangeable corporate bonds.
    ConvertibleBond,
}

const YUAN: i64 = Price::UNITS_PER_YUAN;

/// `(upper bound of the price range, tick)` in price units; the last range is open.
type TickTable = &'static [(i64, i64)];

const STOCK_TICKS: TickTable = &[
    (10 * YUAN, YUAN / 100),
    (50 * YUAN, YUAN / 20),
    (100 * YUAN, YUAN / 10),
    (500 * YUAN, YUAN / 2),
    (1_000 * YUAN, YUAN),
    (i64::MAX, 5 * YUAN),
];
const ETF_TICKS: TickTable = &[(50 * YUAN, YUAN / 100), (i64::MAX, YUAN / 20)];
const WARRANT_TICKS: TickTable = &[
    (5 * YUAN, YUAN / 100),
    (10 * YUAN, YUAN / 20),
    (50 * YUAN, YUAN / 10),
    (100 * YUAN, YUAN / 2),
    (500 * YUAN, YUAN),
    (i64::MAX, 5 * YUAN),
];
const BOND_TICKS: TickTable = &[
    (150 * YUAN, YUAN / 20),
    (1_000 * YUAN, YUAN),
    (i64::MAX, 5 * YUAN),
];

impl InstrumentClass {
    /// The class of an instrument, or `None` when the simulator does not support it.
    /// `OTHER` is a mixed marketdata type and is split by the form of the code.
    #[must_use]
    pub fn of(instrument: &Instrument) -> Option<Self> {
        match instrument.instrument_type {
            InstrumentType::Stock | InstrumentType::Tdr => Some(Self::Stock),
            InstrumentType::Etf | InstrumentType::Etn | InstrumentType::Reit => Some(Self::Etf),
            InstrumentType::WarrantCall | InstrumentType::WarrantPut => Some(Self::Warrant),
            InstrumentType::Other => {
                Self::of_other(instrument.venue, instrument.symbol.as_str().as_bytes())
            }
        }
    }

    fn of_other(venue: Venue, code: &[u8]) -> Option<Self> {
        let digits = |part: &[u8]| part.iter().all(u8::is_ascii_digit);
        let all_digits = digits(code);
        // Preferred shares: four digits and a suffix that starts with a letter.
        if code.len() > 4
            && digits(&code[..4])
            && code[4].is_ascii_alphabetic()
            && code[4..].iter().all(u8::is_ascii_alphanumeric)
        {
            return Some(Self::Stock);
        }
        if all_digits && code.len() == 6 {
            if venue == Venue::Twse && code.starts_with(b"91") {
                return Some(Self::Stock);
            }
            if code.starts_with(b"00") {
                return Some(Self::Etf);
            }
        }
        // Exchange-traded notes: `02`, digits, and a letter at the end.
        if code.len() == 6
            && code.starts_with(b"02")
            && digits(&code[..5])
            && code[5].is_ascii_alphabetic()
        {
            return Some(Self::Etf);
        }
        if venue == Venue::Tpex && all_digits && matches!(code.len(), 5 | 6) {
            return Some(Self::ConvertibleBond);
        }
        None
    }

    const fn ticks(self) -> TickTable {
        match self {
            Self::Stock => STOCK_TICKS,
            Self::Etf => ETF_TICKS,
            Self::Warrant => WARRANT_TICKS,
            Self::ConvertibleBond => BOND_TICKS,
        }
    }

    /// The tick, in price units, of the range `price` itself lies in.
    #[must_use]
    pub fn tick(self, price: Price) -> i64 {
        self.ticks()
            .iter()
            .find(|&&(below, _)| price.units() < below)
            .map(|&(_, tick)| tick)
            .expect("the last range is open")
    }

    /// Whether `price` is positive and on the tick grid.
    #[must_use]
    pub fn is_on_tick(self, price: Price) -> bool {
        price.units() > 0 && price.units() % self.tick(price) == 0
    }
}

/// The daily price limits of an instrument (`docs/reference/matching.md` section 5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriceLimits {
    Range {
        down: Price,
        up: Price,
    },
    /// The instrument trades without price limits today.
    Unlimited,
    /// marketdata has no limit price for the instrument today.
    Unknown,
}

impl PriceLimits {
    /// A limit-up at or above this and a limit-down at or below [`Self::NO_LIMIT_DOWN`]
    /// is how the exchange broadcasts "no price limit".
    const NO_LIMIT_UP: Price = Price::from_units(9_995 * YUAN);
    const NO_LIMIT_DOWN: Price = Price::from_units(YUAN / 100);

    #[must_use]
    pub fn of(instrument: &Instrument) -> Self {
        match (instrument.limit_down, instrument.limit_up) {
            (Some(down), Some(up)) if up >= Self::NO_LIMIT_UP && down <= Self::NO_LIMIT_DOWN => {
                Self::Unlimited
            }
            (Some(down), Some(up)) => Self::Range { down, up },
            _ => Self::Unknown,
        }
    }
}

/// What the matching rules need to know about one book on one trading day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookSpec {
    pub class: InstrumentClass,
    pub method: Method,
    /// Shares per regular-board lot.
    pub shares_per_lot: u32,
    pub limits: PriceLimits,
    /// The reference price of the day (開盤競價基準).
    pub ref_price: Option<Price>,
}

impl BookSpec {
    /// The spec of a book of `instrument`, or `None` when the simulator does not
    /// support the instrument: a foreign-currency instrument or an unknown class.
    ///
    /// `shares_per_lot` is passed separately because marketdata lacks it on some days
    /// and the caller decides what to assume then.
    #[must_use]
    pub fn new(instrument: &Instrument, method: Method, shares_per_lot: u32) -> Option<Self> {
        if instrument.currency != Currency::Twd {
            return None;
        }
        Some(Self {
            class: InstrumentClass::of(instrument)?,
            method,
            shares_per_lot,
            limits: PriceLimits::of(instrument),
            ref_price: instrument.ref_price,
        })
    }
}

#[cfg(test)]
mod tests {
    use osmium_book::Method;
    use osmium_types::{Currency, Instrument, InstrumentType, Price, Symbol, Venue};

    use super::{BookSpec, InstrumentClass, PriceLimits};

    fn price(text: &str) -> Price {
        text.parse().unwrap()
    }

    fn instrument(venue: Venue, code: &str, kind: InstrumentType) -> Instrument {
        Instrument {
            venue,
            symbol: Symbol::new(code).unwrap(),
            instrument_type: kind,
            shares_per_lot: Some(1_000),
            ref_price: Some(price("100")),
            limit_up: Some(price("110")),
            limit_down: Some(price("90")),
            match_interval_s: Some(0),
            currency: Currency::Twd,
            derived: false,
        }
    }

    #[test]
    fn the_marketdata_type_decides_the_class_except_for_other() {
        use InstrumentClass::{Etf, Stock, Warrant};
        for (kind, expected) in [
            (InstrumentType::Stock, Stock),
            (InstrumentType::Tdr, Stock),
            (InstrumentType::Etf, Etf),
            (InstrumentType::Etn, Etf),
            (InstrumentType::Reit, Etf),
            (InstrumentType::WarrantCall, Warrant),
            (InstrumentType::WarrantPut, Warrant),
        ] {
            assert_eq!(
                InstrumentClass::of(&instrument(Venue::Twse, "2330", kind)),
                Some(expected)
            );
        }
    }

    #[test]
    fn other_is_split_by_the_form_of_the_code() {
        use InstrumentClass::{ConvertibleBond, Etf, Stock};
        for (venue, code, expected) in [
            (Venue::Twse, "2881A", Some(Stock)),
            (Venue::Twse, "2887Z1", Some(Stock)),
            (Venue::Tpex, "5871A", Some(Stock)),
            (Venue::Twse, "911622", Some(Stock)),
            (Venue::Tpex, "911622", Some(ConvertibleBond)),
            (Venue::Twse, "009823", Some(Etf)),
            (Venue::Tpex, "006201", Some(Etf)),
            (Venue::Twse, "02001L", Some(Etf)),
            (Venue::Tpex, "22283", Some(ConvertibleBond)),
            (Venue::Tpex, "368401", Some(ConvertibleBond)),
            (Venue::Twse, "22283", None),
            (Venue::Twse, "020030", None),
            (Venue::Tpex, "1234", None),
            (Venue::Twse, "00636K", None),
            (Venue::Tpex, "ABCD", None),
        ] {
            assert_eq!(
                InstrumentClass::of(&instrument(venue, code, InstrumentType::Other)),
                expected,
                "{venue} {code}"
            );
        }
    }

    #[test]
    fn each_class_has_its_tick_table() {
        use InstrumentClass::{ConvertibleBond, Etf, Stock, Warrant};
        for (class, at, tick) in [
            (Stock, "9.99", "0.01"),
            (Stock, "10", "0.05"),
            (Stock, "49.95", "0.05"),
            (Stock, "50", "0.1"),
            (Stock, "100", "0.5"),
            (Stock, "500", "1"),
            (Stock, "1000", "5"),
            (Stock, "5000", "5"),
            (Etf, "49.99", "0.01"),
            (Etf, "50", "0.05"),
            (Warrant, "4.99", "0.01"),
            (Warrant, "5", "0.05"),
            (Warrant, "10", "0.1"),
            (Warrant, "50", "0.5"),
            (Warrant, "100", "1"),
            (Warrant, "500", "5"),
            (ConvertibleBond, "149.95", "0.05"),
            (ConvertibleBond, "150", "1"),
            (ConvertibleBond, "1000", "5"),
        ] {
            assert_eq!(class.tick(price(at)), price(tick).units(), "{class:?} {at}");
        }
    }

    #[test]
    fn a_price_is_valid_on_the_grid_of_its_own_range() {
        let stock = InstrumentClass::Stock;
        assert!(stock.is_on_tick(price("9.99")));
        assert!(stock.is_on_tick(price("10.05")));
        assert!(!stock.is_on_tick(price("10.01")));
        assert!(stock.is_on_tick(price("50")));
        assert!(!stock.is_on_tick(price("50.05")));
        assert!(stock.is_on_tick(price("1005")));
        assert!(!stock.is_on_tick(price("1001")));
        assert!(!stock.is_on_tick(Price::from_units(0)));
        assert!(!stock.is_on_tick(Price::from_units(-100)));
    }

    #[test]
    fn limits_are_a_range_unlimited_or_unknown() {
        let mut stock = instrument(Venue::Twse, "2330", InstrumentType::Stock);
        assert_eq!(
            PriceLimits::of(&stock),
            PriceLimits::Range {
                down: price("90"),
                up: price("110")
            }
        );
        stock.limit_up = Some(price("9995"));
        stock.limit_down = Some(price("0.01"));
        assert_eq!(PriceLimits::of(&stock), PriceLimits::Unlimited);
        stock.limit_up = Some(price("9999.95"));
        assert_eq!(PriceLimits::of(&stock), PriceLimits::Unlimited);
        stock.limit_down = Some(price("0.02"));
        assert!(matches!(PriceLimits::of(&stock), PriceLimits::Range { .. }));
        stock.limit_down = None;
        assert_eq!(PriceLimits::of(&stock), PriceLimits::Unknown);
        stock.limit_down = Some(price("90"));
        stock.limit_up = None;
        assert_eq!(PriceLimits::of(&stock), PriceLimits::Unknown);
    }

    #[test]
    fn foreign_currency_and_unknown_classes_are_unsupported() {
        let stock = instrument(Venue::Twse, "2330", InstrumentType::Stock);
        let spec = BookSpec::new(&stock, Method::Continuous, 1_000).unwrap();
        assert_eq!(spec.class, InstrumentClass::Stock);
        assert_eq!(spec.ref_price, Some(price("100")));

        let mut foreign = stock;
        foreign.currency = Currency::Foreign;
        assert_eq!(BookSpec::new(&foreign, Method::Continuous, 1_000), None);
        let unknown = instrument(Venue::Twse, "22283", InstrumentType::Other);
        assert_eq!(BookSpec::new(&unknown, Method::Continuous, 1_000), None);
    }
}
