//! The corporate actions of a real marketdata tree. Ignored by default because it
//! needs the data:
//!
//! ```text
//! OSMIUM_MD_ROOT=/srv/md/v1 cargo test --release --locked -p osmium-ledger --test marketdata -- --ignored --nocapture
//! ```

use std::collections::{BTreeMap, HashMap};

use osmium_ledger::{
    Calendar, Config, DayFacts, DayTrade, Ledger, LedgerError, MissingTerms, SecurityClass,
    SecurityFacts,
};
use osmium_md::{CorporateActions, DEFAULT_MD_ROOT, MdRoot, TradingCalendar};
use osmium_types::{
    Board, CorporateAction, DailyQuote, Date, Fill, FillBasis, Money, NewOrder, OrderEvent,
    OrderEventKind, OrderId, OrderPrice, Price, Shares, Side, Symbol, TimeInForce, Timestamp,
    TradeKind, Venue,
};

const SHARES: i64 = 10_000;

fn facts(reference: Price) -> SecurityFacts {
    SecurityFacts {
        class: SecurityClass::Stock,
        ref_price: Some(reference),
        limit_up: None,
        limit_down: None,
        day_trade: DayTrade::default(),
        disposition: false,
        repeated_disposition: false,
        margin_eligible: false,
        short_eligible: false,
        short_suspended: false,
        short_last_cover_day: false,
        short_price_floor: false,
        non_10_par: None,
        shares_per_lot: Some(1_000),
    }
}

fn quotes(symbol: Symbol, close: Price) -> HashMap<Symbol, DailyQuote> {
    HashMap::from([(
        symbol,
        DailyQuote {
            symbol,
            close: Some(close),
            close_bid: None,
            close_ask: None,
            ref_price: None,
            limit_down: None,
        },
    )])
}

/// Holds 10,000 shares bought at the close before the ex-date through `action` and
/// returns the equity at that close and at the ex-date's reference price.
fn hold_through(
    calendar: &Calendar,
    config: Config,
    before: Date,
    action: &CorporateAction,
) -> Result<(Money, Money), LedgerError> {
    let symbol = action.symbol;
    let mut ledger = Ledger::new(config, calendar.clone());
    ledger.begin_day(DayFacts {
        date: before,
        securities: HashMap::from([(symbol, facts(action.before_price))]),
        actions: Vec::new(),
    })?;
    let order = NewOrder {
        symbol,
        board: Board::Regular,
        side: Side::Buy,
        kind: TradeKind::Cash,
        price: OrderPrice::Limit(action.before_price),
        time_in_force: TimeInForce::Rod,
        qty: Shares::new(SHARES),
    };
    ledger.check_new(OrderId(1), &order).unwrap();
    ledger.on_event(&OrderEvent {
        order: OrderId(1),
        time: Timestamp::from_micros(0),
        kind: OrderEventKind::Filled(Fill {
            price: action.before_price,
            qty: Shares::new(SHARES),
            basis: FillBasis::Aggressive,
            trigger: None,
        }),
    });
    let bought = ledger.end_day(&quotes(symbol, action.before_price))?.equity;
    ledger.begin_day(DayFacts {
        date: action.ex_date,
        securities: HashMap::from([(symbol, facts(action.ref_price))]),
        actions: vec![*action],
    })?;
    let after = ledger.end_day(&quotes(symbol, action.ref_price))?.equity;
    Ok((bought, after))
}

#[test]
#[ignore = "needs a marketdata tree"]
fn real_corporate_actions_keep_the_value_of_a_position() {
    let root =
        MdRoot::new(std::env::var("OSMIUM_MD_ROOT").unwrap_or_else(|_| DEFAULT_MD_ROOT.to_owned()));
    let trading: Vec<Date> = TradingCalendar::load(&root)
        .unwrap()
        .trading_days(Venue::Twse)
        .collect();
    let last = *trading.last().unwrap();
    let extra = (1..=4).map(|days| Date::from_epoch_days(last.epoch_days() + days).unwrap());
    let calendar = Calendar::new(trading.iter().copied(), extra);
    let actions = CorporateActions::load(&root, trading[1]).unwrap();

    for (name, missing_terms) in [
        ("exact terms only", MissingTerms::Error),
        ("approximate terms", MissingTerms::Approximate),
    ] {
        // Per kind: applied, needing terms, and the largest change of value in parts
        // per ten thousand of the position.
        let mut applied: BTreeMap<&str, usize> = BTreeMap::new();
        let mut missing: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        let mut worst: BTreeMap<&str, (i64, String)> = BTreeMap::new();
        let mut outliers: Vec<String> = Vec::new();
        let mut total = 0_usize;
        for action in actions.rows() {
            let Ok(index) = trading.binary_search(&action.ex_date) else {
                continue;
            };
            if index == 0 {
                continue;
            }
            let mut config = Config::new(Money::new(1_000_000_000_000));
            config.corporate_actions.missing_terms = missing_terms;
            let kind = action.kind.as_str();
            match hold_through(&calendar, config, trading[index - 1], action) {
                Ok((before, after)) => {
                    *applied.entry(kind).or_default() += 1;
                    let position = action.before_price.units() * SHARES / Price::UNITS_PER_YUAN;
                    let change = (after - before).get() * 10_000 / position.max(1);
                    total += 1;
                    if change.abs() > 100 {
                        outliers.push(format!(
                            "{kind} {} {} {} -> {}: {change}",
                            action.symbol, action.ex_date, action.before_price, action.ref_price
                        ));
                    }
                    let entry = worst.entry(kind).or_insert((0, String::new()));
                    if change.abs() > entry.0.abs() {
                        *entry = (change, format!("{} {}", action.symbol, action.ex_date));
                    }
                }
                Err(LedgerError::MissingActionTerms { missing: what, .. }) => {
                    *missing.entry((kind, what)).or_default() += 1;
                }
                Err(error) => panic!("{} on {}: {error}", action.symbol, action.ex_date),
            }
        }
        println!("{name}:");
        println!("  applied: {applied:?}");
        println!("  needing terms: {missing:?}");
        println!("  largest change of value (1/10000 of the position): {worst:?}");
        println!(
            "  changing the value by more than 1 %: {} of {total}",
            outliers.len()
        );
        for outlier in outliers.iter().take(12) {
            println!("    {outlier}");
        }
        // The reference price is what makes the position worth the same on both sides
        // of the ex-date, so the accounting must leave the value where it was. The
        // rows that do not are known: capital reductions whose reference price
        // disagrees with their ratio and refund (2323 on 2022-10-24 and 2026-10-12,
        // 3356 on 2024-11-11), and a dividend stated in another currency than the
        // price (00774C).
        assert!(outliers.len() * 1_000 <= total, "{name}");
    }
}
