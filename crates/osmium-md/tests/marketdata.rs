//! Real-data checks. They need a readable marketdata tree and are ignored by default:
//! `OSMIUM_MD_ROOT=/srv/md/v1 cargo test -p osmium-md --release -- --ignored`.

use std::time::Instant;

use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{EventPayload, MarketSignal, Observation, Symbol, TradingDate};
use osmium_md::{DEFAULT_MD_ROOT, Feed, FeedDay, MdRoot};
use replay_engine::{EventStream, ReplayCore, order_events};

fn root() -> MdRoot {
    MdRoot::new(std::env::var("OSMIUM_MD_ROOT").unwrap_or_else(|_| DEFAULT_MD_ROOT.to_owned()))
}

fn replay_day(date: &str, feed: Feed, symbols: &[&str]) {
    let date = TradingDate::parse(date).unwrap();
    let symbols: Vec<Symbol> = symbols.iter().map(|s| Symbol::new(s).unwrap()).collect();
    let started = Instant::now();
    let mut day = FeedDay::load(&root(), date, feed, &symbols).unwrap();
    let loaded = started.elapsed();

    let mut events = Vec::new();
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for symbol in &symbols {
        let mut stream = day.take_stream(symbol).unwrap();
        let mut previous_seq = 0;
        while let Some(event) = stream.next_event().unwrap() {
            let seq = event.source_sequence().unwrap();
            assert!(seq > previous_seq);
            previous_seq = seq;
            assert!(event.observed_time() >= event.match_time());
            let signal = match event.payload().market_signal() {
                Observation::Set(MarketSignal::Continuous) => "continuous",
                Observation::Set(MarketSignal::AuctionCollecting(_)) => "collecting",
                Observation::Set(MarketSignal::AuctionUncross(_)) => "uncross",
                _ => "other",
            };
            let kind = match event.payload() {
                EventPayload::QuoteSnapshot(_) => "quote",
                EventPayload::BookSnapshot(_) => "book",
                EventPayload::TradeBatch(_) => "trade",
                EventPayload::IndicativeAuction(_) => "indicative",
                EventPayload::MarketStatus(_) => "status",
            };
            *kinds.entry(format!("{kind}/{signal}")).or_default() += 1;
            events.push(event);
        }
    }

    let ordered = order_events(events).unwrap();
    let mut core = ReplayCore::new(
        symbols
            .iter()
            .map(|symbol| {
                MarketState::new(
                    market_types::InstrumentId::new(feed.market(), *symbol),
                    date,
                )
            })
            .collect(),
        MarketStateReducer::new(feed.state_profile()),
        ReducerContext::new(
            date,
            SessionSegmentId::new("regular").unwrap(),
            SegmentBoundaryPolicy::Carry,
            1,
        ),
    )
    .unwrap();
    let count = ordered.len();
    core.replay(ordered).unwrap();
    eprintln!(
        "{date} {feed} {} symbols: {count} events, load {loaded:?}, total {:?}; {kinds:?}",
        symbols.len(),
        started.elapsed()
    );
}

#[test]
#[ignore = "needs a marketdata tree (OSMIUM_MD_ROOT, default /srv/md/v1)"]
fn twse_f6_replays_through_the_reducer() {
    replay_day("2026-10-15", Feed::TwseF6, &["2330", "006203", "2317"]);
}

#[test]
#[ignore = "needs a marketdata tree (OSMIUM_MD_ROOT, default /srv/md/v1)"]
fn odd_lot_and_warrant_feeds_replay_through_the_reducer() {
    replay_day("2026-10-15", Feed::TwseF23, &["2330", "00402A"]);
    replay_day("2026-10-15", Feed::TpexF6, &["6488", "8069"]);
    replay_day("2026-10-15", Feed::TwseF17, &["066186", "063344"]);
}

/// Maps and replays a spread of symbols for every F6／F23 feed present on each date.
///
/// Dates come from `OSMIUM_MD_SWEEP_DATES` (comma-separated); every `OSMIUM_MD_SWEEP_STRIDE`-th
/// instrument (default 25) is selected.
#[test]
#[ignore = "needs a marketdata tree (OSMIUM_MD_ROOT, default /srv/md/v1)"]
fn sweep_dates_map_and_replay() {
    let dates = std::env::var("OSMIUM_MD_SWEEP_DATES").unwrap_or_else(|_| {
        "2022-03-15,2023-06-14,2024-05-20,2025-04-07,2025-11-12,2026-10-15".to_owned()
    });
    let stride: usize = std::env::var("OSMIUM_MD_SWEEP_STRIDE")
        .ok()
        .map_or(25, |value| value.parse().unwrap());
    for date in dates.split(',') {
        let trading_date = TradingDate::parse(date).unwrap();
        let manifest = osmium_md::DayManifest::load(&root(), trading_date).unwrap();
        // Warrants are absent from `instruments` on many dates, so sampling it would select
        // no F17 symbols; F17 is covered by the fixed-symbol test above.
        for feed in [Feed::TwseF6, Feed::TpexF6, Feed::TwseF23, Feed::TpexF23] {
            if manifest
                .entry(osmium_md::Table::Books, feed.part())
                .is_err()
            {
                eprintln!("{date} {feed}: absent");
                continue;
            }
            let instruments =
                osmium_md::Instruments::load(&root(), trading_date, feed, &manifest).unwrap();
            let symbols: Vec<String> = instruments
                .iter()
                .step_by(stride)
                .map(|record| record.symbol.as_str().to_owned())
                .collect();
            let symbols: Vec<&str> = symbols.iter().map(String::as_str).collect();
            replay_day(date, feed, &symbols);
        }
    }
}
