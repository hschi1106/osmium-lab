use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::PathBuf,
};

use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{InstrumentId, MarketId, MatchTime, Symbol, TradingDate};
use replay_engine::{ReplayCore, order_events};
use twse_normalizer::{NormalizerConfig, TwseNormalizer};

fn instrument() -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new("SYNTH-TWSE-EQ").unwrap())
}

fn date() -> TradingDate {
    TradingDate::parse("2026-07-20").unwrap()
}

fn normalize_fixture() -> Vec<market_types::DomainEvent> {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/teralion/twse/SYNTH-TWSE-EQ/2026-07-20/regular-quotes");
    let mut shards = fs::read_dir(&fixture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect::<Vec<_>>();
    shards.sort();
    let lines = shards.into_iter().flat_map(|path| {
        BufReader::new(File::open(path).unwrap())
            .lines()
            .map(Result::unwrap)
    });
    TwseNormalizer::new(
        NormalizerConfig::new(
            instrument(),
            date(),
            MatchTime::parse("2026-07-20T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-20T13:35:00+08:00").unwrap(),
        )
        .unwrap(),
    )
    .normalize_json_lines(lines)
    .unwrap()
    .into_events()
}

fn core() -> ReplayCore {
    ReplayCore::new(
        vec![MarketState::new(instrument(), date())],
        MarketStateReducer::twse_regular(),
        ReducerContext::new(
            date(),
            SessionSegmentId::new("regular").unwrap(),
            SegmentBoundaryPolicy::Carry,
            1,
        ),
    )
    .unwrap()
}

#[test]
fn synthetic_fixture_replay_is_deterministic_and_preserves_event_order() {
    let events = normalize_fixture();
    assert!(!events.is_empty());

    let ordered = order_events(events.clone()).unwrap();
    assert_eq!(ordered.len(), events.len());

    let mut first = core();
    first.replay(events.clone()).unwrap();
    let first = first.complete().unwrap();

    let mut reversed = events;
    reversed.reverse();
    let mut second = core();
    second.replay(reversed).unwrap();
    let second = second.complete().unwrap();

    assert_eq!(first.summary().event_count(), ordered.len() as u64);
    assert_eq!(
        first.summary().event_checksum(),
        second.summary().event_checksum()
    );
    assert_eq!(
        first.summary().final_state_checksum(),
        second.summary().final_state_checksum()
    );
    let state = first.state(&instrument()).unwrap();
    assert_eq!(state.state_version(), ordered.len() as u64);
    assert!(state.cumulative_volume().known().is_some());
    assert!(first.summary().last_match_time().is_some());
}
