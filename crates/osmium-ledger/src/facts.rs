//! What the ledger knows about each instrument on one trading day, derived from
//! marketdata by the rules of `docs/reference/accounting.md`.

use std::collections::HashMap;

use osmium_matching::InstrumentClass;
use osmium_types::{
    CorporateAction, CreditQuota, DailyQuote, Date, DayTradeFlag, Instrument, InstrumentType,
    MarketEvent, MarketEventKind, Price, Symbol,
};

use crate::{
    amount::Rate,
    calendar::add_days,
    config::{Config, Eligibility},
};

/// The classes the fee, tax and credit rules tell apart (sections 6, 8.1 and 9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecurityClass {
    Stock,
    Preferred,
    Tdr,
    Etf,
    /// An ETF whose code ends in `B`.
    BondEtf,
    Etn,
    Reit,
    Warrant,
    ConvertibleBond,
}

impl SecurityClass {
    /// The class of an instrument, or `None` when the simulator does not support it.
    /// It refines the classes of `docs/reference/matching.md` section 5.3.
    #[must_use]
    pub fn of(instrument: &Instrument) -> Option<Self> {
        let code = instrument.symbol.as_str().as_bytes();
        Some(match InstrumentClass::of(instrument)? {
            InstrumentClass::Warrant => Self::Warrant,
            InstrumentClass::ConvertibleBond => Self::ConvertibleBond,
            InstrumentClass::Stock => match instrument.instrument_type {
                InstrumentType::Stock => Self::Stock,
                InstrumentType::Tdr => Self::Tdr,
                _ if code.iter().all(u8::is_ascii_digit) => Self::Tdr,
                _ => Self::Preferred,
            },
            InstrumentClass::Etf => match instrument.instrument_type {
                InstrumentType::Etn => Self::Etn,
                InstrumentType::Reit => Self::Reit,
                InstrumentType::Other if code.starts_with(b"02") => Self::Etn,
                _ if code.ends_with(b"B") => Self::BondEtf,
                _ => Self::Etf,
            },
        })
    }

    /// Stocks, depositary receipts and funds; not notes, trusts, warrants or bonds.
    const fn is_share_or_fund(self) -> bool {
        matches!(
            self,
            Self::Stock | Self::Preferred | Self::Tdr | Self::Etf | Self::BondEtf
        )
    }

    /// The classes that may be bought on margin and sold short at all (section 9.1).
    const fn may_have_credit(self) -> bool {
        matches!(self, Self::Stock | Self::Tdr | Self::Etf | Self::BondEtf)
    }

    /// Funds, notes and trusts are exempt from the short-sale price restriction.
    const fn is_fund_like(self) -> bool {
        matches!(self, Self::Etf | Self::BondEtf | Self::Etn | Self::Reit)
    }
}

/// Which same-day round trips may be paired as day trades (section 8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DayTrade {
    pub buy_first: bool,
    pub sell_first: bool,
    /// The answer is an approximation: the exchange's flag is not known for the day.
    pub approx: bool,
}

/// What the ledger needs from the previous trading day of one instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PreviousDay {
    pub close: Option<Price>,
    pub limit_down: Option<Price>,
    pub ref_price: Option<Price>,
    pub margin_limit: Option<i64>,
    pub short_limit: Option<i64>,
}

impl PreviousDay {
    /// The previous day as marketdata `daily` has it, without quotas.
    #[must_use]
    pub const fn of(quote: &DailyQuote) -> Self {
        Self {
            close: quote.close,
            limit_down: quote.limit_down,
            ref_price: quote.ref_price,
            margin_limit: None,
            short_limit: None,
        }
    }

    pub fn set_quota(&mut self, quota: &CreditQuota) {
        self.margin_limit = quota.margin_limit;
        self.short_limit = quota.short_limit;
    }
}

/// One instrument on one trading day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityFacts {
    pub class: SecurityClass,
    pub ref_price: Option<Price>,
    pub limit_up: Option<Price>,
    pub limit_down: Option<Price>,
    pub day_trade: DayTrade,
    /// In a disposition period.
    pub disposition: bool,
    pub margin_eligible: bool,
    pub short_eligible: bool,
    /// Short selling is suspended today.
    pub short_suspended: bool,
    /// Today is the last day to cover before a suspension (section 10.4).
    pub short_last_cover_day: bool,
    /// Short sales may not be priced below the reference price (section 10.2).
    pub short_price_floor: bool,
    /// `None` when marketdata does not say whether the par value is 10 yuan.
    pub non_10_par: Option<bool>,
    /// `None` when marketdata does not have the size of a board lot for the day.
    pub shares_per_lot: Option<u32>,
}

/// How long a short-sale suspension without an end date is taken to last, in calendar
/// days: suspensions with a known end last four trading days.
const OPEN_ENDED_SUSPENSION_DAYS: u32 = 7;

fn in_force(event: &MarketEvent, date: Date) -> bool {
    let end = event
        .end
        .unwrap_or_else(|| add_days(event.start, OPEN_ENDED_SUSPENSION_DAYS));
    event.start <= date && date <= end
}

impl SecurityFacts {
    /// `events` are the marketdata events of this instrument; `previous` is its
    /// previous trading day, `None` when it has no row there.
    #[must_use]
    pub fn derive(
        date: Date,
        instrument: &Instrument,
        previous: Option<&PreviousDay>,
        events: &[MarketEvent],
        config: &Config,
    ) -> Option<Self> {
        let class = SecurityClass::of(instrument)?;
        let active = |kind: MarketEventKind| {
            events
                .iter()
                .any(|event| event.kind == kind && in_force(event, date))
        };
        let disposition = active(MarketEventKind::Disposition);
        let short_suspended = active(MarketEventKind::ShortSuspended);

        let day_trade = if disposition || !class.is_share_or_fund() {
            DayTrade {
                approx: instrument.derived && class.is_share_or_fund(),
                ..DayTrade::default()
            }
        } else if instrument.derived {
            DayTrade {
                buy_first: true,
                sell_first: !active(MarketEventKind::DayTradeSuspended),
                approx: true,
            }
        } else {
            DayTrade {
                buy_first: instrument.day_trade.is_some(),
                sell_first: instrument.day_trade == Some(DayTradeFlag::Both),
                approx: false,
            }
        };

        let listed =
            config.credit.eligibility == Eligibility::MarketdataApprox && class.may_have_credit();
        let quota = |limit: fn(&PreviousDay) -> Option<i64>| {
            listed && previous.and_then(limit).is_some_and(|limit| limit > 0)
        };
        let margin_eligible = quota(|day| day.margin_limit) && !disposition;
        let short_eligible = quota(|day| day.short_limit);

        Some(Self {
            class,
            ref_price: instrument.ref_price,
            limit_up: instrument.limit_up,
            limit_down: instrument.limit_down,
            day_trade,
            disposition,
            margin_eligible,
            short_eligible,
            short_suspended,
            short_last_cover_day: events
                .iter()
                .any(|event| event.kind == MarketEventKind::ShortSuspended && event.start == date),
            short_price_floor: short_price_floor(date, class, previous),
            non_10_par: instrument.non_10_par,
            shares_per_lot: instrument.shares_per_lot,
        })
    }
}

/// Section 10.2. An instrument whose previous close or limit is not known is treated
/// as restricted.
fn short_price_floor(date: Date, class: SecurityClass, previous: Option<&PreviousDay>) -> bool {
    if class.is_fund_like() {
        return false;
    }
    let Some(previous) = previous else {
        return false;
    };
    let Some(close) = previous.close else {
        // It did not trade, so it did not close at the limit.
        return false;
    };
    let at_limit_down = previous.limit_down.is_none_or(|limit| close <= limit);
    let (from, until) = (
        Date::from_ymd(2022, 10, 21).expect("a valid date"),
        Date::from_ymd(2023, 2, 23).expect("a valid date"),
    );
    let temporary = from <= date
        && date <= until
        && matches!(
            class,
            SecurityClass::Stock | SecurityClass::Preferred | SecurityClass::Tdr
        )
        && previous.ref_price.is_none_or(|reference| {
            i128::from(close.units()) * 1_000 <= i128::from(reference.units()) * 965
        });
    at_limit_down || temporary
}

/// The lowest short-sale margin ratio in force on a trade date (section 10.1).
#[must_use]
pub fn short_margin_ratio(date: Date) -> Rate {
    const TABLE: &[((u32, u32, u32), i64)] = &[
        ((2022, 10, 3), 100),
        ((2022, 10, 12), 120),
        ((2023, 2, 24), 90),
        ((2025, 4, 7), 130),
        ((2025, 5, 26), 90),
        ((2026, 11, 9), 130),
    ];
    let percent = TABLE
        .iter()
        .rev()
        .find(|&&((year, month, day), _)| {
            date >= Date::from_ymd(year, month, day).expect("a valid date")
        })
        .map_or(90, |&(_, percent)| percent);
    Rate::new(percent, 100)
}

/// Every supported instrument of one trading day, and the corporate actions that take
/// effect on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayFacts {
    pub date: Date,
    pub securities: HashMap<Symbol, SecurityFacts>,
    /// The corporate actions whose ex-date is `date`.
    pub actions: Vec<CorporateAction>,
}

impl DayFacts {
    /// `previous` holds the previous trading day of each instrument; `events` and
    /// `actions` may hold rows of any date and instrument.
    #[must_use]
    pub fn build<'a>(
        date: Date,
        instruments: impl IntoIterator<Item = &'a Instrument>,
        previous: &HashMap<Symbol, PreviousDay>,
        events: &[MarketEvent],
        actions: &[CorporateAction],
        config: &Config,
    ) -> Self {
        let mut by_symbol: HashMap<Symbol, Vec<MarketEvent>> = HashMap::new();
        for event in events.iter().filter(|event| event.start <= date) {
            by_symbol.entry(event.symbol).or_default().push(*event);
        }
        let securities = instruments
            .into_iter()
            .filter_map(|instrument| {
                let events = by_symbol
                    .get(&instrument.symbol)
                    .map_or(&[][..], Vec::as_slice);
                SecurityFacts::derive(
                    date,
                    instrument,
                    previous.get(&instrument.symbol),
                    events,
                    config,
                )
                .map(|facts| (instrument.symbol, facts))
            })
            .collect();
        let mut actions: Vec<CorporateAction> = actions
            .iter()
            .filter(|action| action.ex_date == date)
            .copied()
            .collect();
        actions.sort_by_key(|action| (action.symbol, action.kind.as_str()));
        Self {
            date,
            securities,
            actions,
        }
    }
}

#[cfg(test)]
mod tests {
    use osmium_types::{
        Currency, Date, DayTradeFlag, Instrument, InstrumentType, MarketEvent, MarketEventKind,
        Money, Price, Symbol, Venue,
    };

    use super::{DayTrade, PreviousDay, SecurityClass, SecurityFacts, short_margin_ratio};
    use crate::{
        amount::Rate,
        config::{Config, Eligibility},
    };

    fn day(text: &str) -> Date {
        text.parse().unwrap()
    }

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
            day_trade: Some(DayTradeFlag::Both),
            non_10_par: Some(false),
        }
    }

    fn stock() -> Instrument {
        instrument(Venue::Twse, "2330", InstrumentType::Stock)
    }

    fn event(kind: MarketEventKind, start: &str, end: Option<&str>) -> MarketEvent {
        MarketEvent {
            symbol: Symbol::new("2330").unwrap(),
            kind,
            start: day(start),
            end: end.map(day),
        }
    }

    fn listed() -> PreviousDay {
        PreviousDay {
            close: Some(price("100")),
            limit_down: Some(price("91")),
            ref_price: Some(price("101")),
            margin_limit: Some(1_000_000),
            short_limit: Some(1_000_000),
        }
    }

    fn facts(
        date: &str,
        instrument: &Instrument,
        previous: Option<&PreviousDay>,
        events: &[MarketEvent],
    ) -> SecurityFacts {
        SecurityFacts::derive(
            day(date),
            instrument,
            previous,
            events,
            &Config::new(Money::ZERO),
        )
        .unwrap()
    }

    #[test]
    fn classes_refine_the_matching_classes() {
        use InstrumentType::{Etf, Etn, Other, Reit, Stock, Tdr, WarrantCall};
        for (venue, code, kind, expected) in [
            (Venue::Twse, "2330", Stock, SecurityClass::Stock),
            (Venue::Twse, "2881A", Other, SecurityClass::Preferred),
            (Venue::Twse, "9103", Tdr, SecurityClass::Tdr),
            (Venue::Twse, "910322", Other, SecurityClass::Tdr),
            (Venue::Twse, "0050", Etf, SecurityClass::Etf),
            (Venue::Twse, "00679B", Etf, SecurityClass::BondEtf),
            (Venue::Twse, "006208", Other, SecurityClass::Etf),
            (Venue::Twse, "020020", Etn, SecurityClass::Etn),
            (Venue::Twse, "02001L", Other, SecurityClass::Etn),
            (Venue::Twse, "01001T", Reit, SecurityClass::Reit),
            (Venue::Twse, "030001", WarrantCall, SecurityClass::Warrant),
            (Venue::Tpex, "12345", Other, SecurityClass::ConvertibleBond),
        ] {
            assert_eq!(
                SecurityClass::of(&instrument(venue, code, kind)),
                Some(expected),
                "{code}"
            );
        }
        assert_eq!(
            SecurityClass::of(&instrument(Venue::Twse, "XYZ", Other)),
            None
        );
    }

    #[test]
    fn the_exchange_flag_decides_day_trading() {
        let mut instrument = stock();
        let both = facts("2026-10-08", &instrument, None, &[]).day_trade;
        assert_eq!(
            (both.buy_first, both.sell_first, both.approx),
            (true, true, false)
        );
        instrument.day_trade = Some(DayTradeFlag::BuyFirst);
        let buy = facts("2026-10-08", &instrument, None, &[]).day_trade;
        assert_eq!((buy.buy_first, buy.sell_first), (true, false));
        instrument.day_trade = None;
        assert_eq!(
            facts("2026-10-08", &instrument, None, &[]).day_trade,
            DayTrade::default()
        );
    }

    #[test]
    fn without_the_flag_day_trading_is_an_approximation() {
        let mut instrument = stock();
        instrument.derived = true;
        instrument.day_trade = None;
        let open = facts("2023-06-01", &instrument, None, &[]).day_trade;
        assert_eq!(
            (open.buy_first, open.sell_first, open.approx),
            (true, true, true)
        );
        let suspended = [event(
            MarketEventKind::DayTradeSuspended,
            "2023-05-30",
            Some("2023-06-02"),
        )];
        let limited = facts("2023-06-01", &instrument, None, &suspended).day_trade;
        assert_eq!((limited.buy_first, limited.sell_first), (true, false));
        // The suspension is over.
        assert!(
            facts("2023-06-05", &instrument, None, &suspended)
                .day_trade
                .sell_first
        );
    }

    #[test]
    fn warrants_notes_trusts_and_bonds_are_never_day_traded() {
        for (venue, code, kind) in [
            (Venue::Twse, "030001", InstrumentType::WarrantCall),
            (Venue::Twse, "020020", InstrumentType::Etn),
            (Venue::Twse, "01001T", InstrumentType::Reit),
            (Venue::Tpex, "12345", InstrumentType::Other),
        ] {
            let mut instrument = instrument(venue, code, kind);
            instrument.derived = true;
            let day_trade = facts("2023-06-01", &instrument, None, &[]).day_trade;
            assert!(!day_trade.buy_first && !day_trade.sell_first, "{code}");
        }
    }

    #[test]
    fn a_disposition_stops_day_trading_and_margin_buying() {
        let disposition = [event(
            MarketEventKind::Disposition,
            "2026-10-06",
            Some("2026-10-19"),
        )];
        let during = facts("2026-10-08", &stock(), Some(&listed()), &disposition);
        assert!(during.disposition);
        assert_eq!(during.day_trade, DayTrade::default());
        assert!(!during.margin_eligible);
        assert!(during.short_eligible);
        let after = facts("2026-10-20", &stock(), Some(&listed()), &disposition);
        assert!(!after.disposition && after.margin_eligible && after.day_trade.buy_first);
    }

    #[test]
    fn credit_follows_the_quotas_of_the_previous_day() {
        let none = facts("2026-10-08", &stock(), None, &[]);
        assert!(!none.margin_eligible && !none.short_eligible);
        let listed_facts = facts("2026-10-08", &stock(), Some(&listed()), &[]);
        assert!(listed_facts.margin_eligible && listed_facts.short_eligible);
        let stopped = PreviousDay {
            margin_limit: Some(0),
            short_limit: None,
            ..listed()
        };
        let stopped = facts("2026-10-08", &stock(), Some(&stopped), &[]);
        assert!(!stopped.margin_eligible && !stopped.short_eligible);
        // Preferred shares and notes never have credit.
        let preferred = instrument(Venue::Twse, "2881A", InstrumentType::Other);
        assert!(!facts("2026-10-08", &preferred, Some(&listed()), &[]).margin_eligible);

        let mut config = Config::new(Money::ZERO);
        config.credit.eligibility = Eligibility::None;
        let off = SecurityFacts::derive(day("2026-10-08"), &stock(), Some(&listed()), &[], &config)
            .unwrap();
        assert!(!off.margin_eligible && !off.short_eligible);
    }

    #[test]
    fn a_short_suspension_starts_on_the_last_cover_day() {
        let suspension = [event(
            MarketEventKind::ShortSuspended,
            "2026-06-05",
            Some("2026-06-10"),
        )];
        let first = facts("2026-06-05", &stock(), Some(&listed()), &suspension);
        assert!(first.short_suspended && first.short_last_cover_day);
        let middle = facts("2026-06-09", &stock(), Some(&listed()), &suspension);
        assert!(middle.short_suspended && !middle.short_last_cover_day);
        let after = facts("2026-06-11", &stock(), Some(&listed()), &suspension);
        assert!(!after.short_suspended);
        // Without an end date it is taken to last seven calendar days.
        let open = [event(MarketEventKind::ShortSuspended, "2026-06-05", None)];
        assert!(facts("2026-06-12", &stock(), Some(&listed()), &open).short_suspended);
        assert!(!facts("2026-06-15", &stock(), Some(&listed()), &open).short_suspended);
    }

    #[test]
    fn short_sales_have_a_price_floor_after_a_limit_down_close() {
        let floor = |date: &str, instrument: &Instrument, previous: &PreviousDay| {
            facts(date, instrument, Some(previous), &[]).short_price_floor
        };
        assert!(!floor("2026-10-08", &stock(), &listed()));
        let limit_down = PreviousDay {
            close: Some(price("91")),
            ..listed()
        };
        assert!(floor("2026-10-08", &stock(), &limit_down));
        // Funds are exempt.
        let fund = instrument(Venue::Twse, "0050", InstrumentType::Etf);
        assert!(!floor("2026-10-08", &fund, &limit_down));
        // No trade on the previous day: it did not close at the limit.
        let no_trade = PreviousDay {
            close: None,
            ..listed()
        };
        assert!(!floor("2026-10-08", &stock(), &no_trade));
        // An unknown limit is treated as restricted.
        let unknown = PreviousDay {
            limit_down: None,
            ..listed()
        };
        assert!(floor("2026-10-08", &stock(), &unknown));
    }

    #[test]
    fn a_fall_of_three_and_a_half_percent_restricted_shorts_in_late_2022() {
        // 97.4 / 101 is a fall of 3.56 %; 97.5 / 101 is 3.47 %.
        let fell = PreviousDay {
            close: Some(price("97.4")),
            ..listed()
        };
        let held = PreviousDay {
            close: Some(price("97.5")),
            ..listed()
        };
        let floor = |date: &str, previous: &PreviousDay| {
            facts(date, &stock(), Some(previous), &[]).short_price_floor
        };
        assert!(floor("2022-10-21", &fell));
        assert!(floor("2023-02-23", &fell));
        assert!(!floor("2022-10-21", &held));
        assert!(!floor("2022-10-20", &fell));
        assert!(!floor("2023-02-24", &fell));
    }

    #[test]
    fn the_short_margin_ratio_follows_the_trade_date() {
        for (date, percent) in [
            ("2022-01-03", 90),
            ("2022-10-03", 100),
            ("2022-10-11", 100),
            ("2022-10-12", 120),
            ("2023-02-23", 120),
            ("2023-02-24", 90),
            ("2025-04-07", 130),
            ("2025-05-23", 130),
            ("2025-05-26", 90),
            ("2026-11-06", 90),
            ("2026-11-09", 130),
        ] {
            assert_eq!(
                short_margin_ratio(day(date)),
                Rate::new(percent, 100),
                "{date}"
            );
        }
    }
}
