use std::{hint::black_box, time::Instant};

use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{
    BookLevel, BookSide, BookSideKind, CANONICAL_EVENT_VERSION, CompleteBookSnapshot, DomainEvent,
    EVENT_SCHEMA_VERSION, EventFingerprint, EventPayload, InstrumentId, MarketAnnotations,
    MarketId, MatchTime, Observation, ObservedTrade, Price, Quantity, QuantityUnit, QuoteSnapshot,
    SourceFormatId, Symbol, TradeObservationKind, TradingDate, TwseQuoteAnnotations, Volume,
};
use replay_engine::{EventStream, ORDERING_RULE_VERSION, ReplayCore};

const ITERATIONS: usize = 250_000;
const ROUNDS: usize = 5;
const REPLAY_EVENTS: usize = 50_000;

fn fixture_event() -> DomainEvent {
    let unit = QuantityUnit::TradingUnit;
    let bids = [
        ("100.00", 73),
        ("99.50", 131),
        ("99.00", 211),
        ("98.50", 307),
        ("98.00", 401),
    ]
    .into_iter()
    .map(|(price, quantity)| {
        BookLevel::new(
            Price::parse(price).unwrap(),
            Quantity::new(quantity, unit).unwrap(),
        )
    })
    .collect();
    let asks = [
        ("100.50", 79),
        ("101.00", 137),
        ("101.50", 223),
        ("102.00", 313),
        ("102.50", 409),
    ]
    .into_iter()
    .map(|(price, quantity)| {
        BookLevel::new(
            Price::parse(price).unwrap(),
            Quantity::new(quantity, unit).unwrap(),
        )
    })
    .collect();
    let book = CompleteBookSnapshot::new(
        BookSide::new(BookSideKind::Bid, bids).unwrap(),
        BookSide::new(BookSideKind::Ask, asks).unwrap(),
    )
    .unwrap();
    let quote = QuoteSnapshot::new(
        book,
        Observation::Set(ObservedTrade::new(
            Price::parse("100.50").unwrap(),
            Quantity::new(17, unit).unwrap(),
            TradeObservationKind::Regular,
        )),
        Observation::Set(Volume::new(12_345, unit)),
        MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0, 0)),
    )
    .unwrap();
    DomainEvent::new(
        InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
        TradingDate::parse("2026-08-12").unwrap(),
        SourceFormatId::new("STOCK_REALTIME").unwrap(),
        MatchTime::parse("2026-08-12T09:00:00.125+08:00").unwrap(),
        Some(42),
        EventPayload::QuoteSnapshot(quote),
    )
}

// Models the old replay path: a merge ordering key, a second key in apply_ordered,
// then another canonical frame for the event-stream checksum.
fn repeated_encode_baseline(event: &DomainEvent) -> (EventFingerprint, EventFingerprint, Vec<u8>) {
    let merge_fingerprint = event.fingerprint().unwrap();
    let apply_fingerprint = event.fingerprint().unwrap();
    let canonical = event.to_canonical_bytes().unwrap();
    let retained_for_order_check = canonical.clone();
    black_box(retained_for_order_check);
    (merge_fingerprint, apply_fingerprint, canonical)
}

// Models the optimized replay path: one canonical frame supplies both the ordering fingerprint
// and checksum bytes, followed by the retained prior frame used for collision checking.
fn single_encode_path(event: &DomainEvent) -> (EventFingerprint, EventFingerprint, Vec<u8>) {
    let canonical = event.to_canonical_bytes().unwrap();
    let fingerprint = EventFingerprint::hash_canonical_bytes(&canonical);
    let retained_for_order_check = canonical.clone();
    black_box(retained_for_order_check);
    (fingerprint, fingerprint, canonical)
}

fn measure(
    label: &str,
    operation: fn(&DomainEvent) -> (EventFingerprint, EventFingerprint, Vec<u8>),
    event: &DomainEvent,
) -> u128 {
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(operation(black_box(event)));
    }
    let elapsed = started.elapsed().as_nanos();
    println!(
        "{label}: {ITERATIONS} iterations, {elapsed} ns, {:.0} events/s",
        ITERATIONS as f64 * 1_000_000_000.0 / elapsed as f64,
    );
    elapsed
}

struct ReplaySlice<'a> {
    events: std::slice::Iter<'a, DomainEvent>,
    repeat_pre_change_encoding: bool,
}

impl EventStream for ReplaySlice<'_> {
    type Error = std::io::Error;

    fn next_event(&mut self) -> Result<Option<DomainEvent>, Self::Error> {
        let Some(event) = self.events.next() else {
            return Ok(None);
        };
        if self.repeat_pre_change_encoding {
            // The old merge generated a fingerprint, then application generated another key and
            // checksum frame. Current application performs the latter two operations together.
            black_box(event.fingerprint().unwrap());
            black_box(event.to_canonical_bytes().unwrap());
        }
        Ok(Some(event.clone()))
    }
}

fn replay_core(instrument: &InstrumentId, date: TradingDate) -> ReplayCore {
    ReplayCore::new(
        vec![MarketState::new(instrument.clone(), date)],
        MarketStateReducer::twse_regular(),
        ReducerContext::new(
            date,
            SessionSegmentId::new("regular").unwrap(),
            SegmentBoundaryPolicy::Carry,
            1,
        ),
    )
    .unwrap()
}

fn replay_measure(
    events: &[DomainEvent],
    repeat_pre_change_encoding: bool,
) -> (u128, [u8; 32], [u8; 32]) {
    let instrument = events[0].instrument().clone();
    let date = events[0].trading_date();
    let mut core = replay_core(&instrument, date);
    let mut stream = ReplaySlice {
        events: events.iter(),
        repeat_pre_change_encoding,
    };
    let started = Instant::now();
    core.replay_stream(&mut stream).unwrap();
    let completed = core.complete().unwrap();
    let elapsed = started.elapsed().as_nanos();
    let checksum = *completed.summary().event_checksum().as_bytes();
    let state_checksum = *completed.summary().final_state_checksum().as_bytes();
    let seconds = elapsed as f64 / 1_000_000_000.0;
    println!(
        "{} replay: {} events, {elapsed} ns, {:.0} events/s",
        if repeat_pre_change_encoding {
            "repeat-encode baseline"
        } else {
            "single-encode"
        },
        events.len(),
        events.len() as f64 / seconds,
    );
    (elapsed, checksum, state_checksum)
}

fn fixed_replay_events(count: usize) -> Vec<DomainEvent> {
    let template = fixture_event();
    let base = template.match_time().as_unix_microseconds();
    (0..count)
        .map(|sequence| {
            DomainEvent::new(
                template.instrument().clone(),
                template.trading_date(),
                template.source_format().clone(),
                MatchTime::from_unix_microseconds(base + sequence as i64),
                Some(sequence as u64),
                template.payload().clone(),
            )
        })
        .collect()
}

fn main() {
    let event = fixture_event();
    println!(
        "replay-engine {}, event schema {}, canonical event {}, ordering rule {}, target {}/{}",
        env!("CARGO_PKG_VERSION"),
        EVENT_SCHEMA_VERSION,
        CANONICAL_EVENT_VERSION,
        ORDERING_RULE_VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    let replay_events = fixed_replay_events(REPLAY_EVENTS);
    let mut baseline_replay_times = Vec::with_capacity(ROUNDS);
    let mut optimized_replay_times = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        println!("replay round {}", round + 1);
        let (baseline, optimized) = if round % 2 == 0 {
            let baseline = replay_measure(&replay_events, true);
            let optimized = replay_measure(&replay_events, false);
            (baseline, optimized)
        } else {
            let optimized = replay_measure(&replay_events, false);
            let baseline = replay_measure(&replay_events, true);
            (baseline, optimized)
        };
        assert_eq!(
            baseline.1, optimized.1,
            "replay checksum must be unchanged by encoding reuse"
        );
        assert_eq!(
            baseline.2, optimized.2,
            "final state checksum must be unchanged by encoding reuse"
        );
        baseline_replay_times.push(baseline.0);
        optimized_replay_times.push(optimized.0);
    }
    baseline_replay_times.sort_unstable();
    optimized_replay_times.sort_unstable();
    let baseline_replay_median = baseline_replay_times[ROUNDS / 2];
    let optimized_replay_median = optimized_replay_times[ROUNDS / 2];
    println!(
        "replay medians: baseline {baseline_replay_median} ns ({:.0} events/s), single-encode {optimized_replay_median} ns ({:.0} events/s)",
        REPLAY_EVENTS as f64 * 1_000_000_000.0 / baseline_replay_median as f64,
        REPLAY_EVENTS as f64 * 1_000_000_000.0 / optimized_replay_median as f64,
    );
    println!(
        "replay pipeline speedup: {:.2}x (fixed synthetic event stream; checksum equivalent)",
        baseline_replay_median as f64 / optimized_replay_median as f64,
    );
    let baseline = repeated_encode_baseline(&event);
    let optimized = single_encode_path(&event);
    assert_eq!(
        baseline, optimized,
        "optimization must preserve event identity"
    );

    let mut baseline_times = Vec::with_capacity(ROUNDS);
    let mut optimized_times = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        println!("round {}", round + 1);
        if round % 2 == 0 {
            baseline_times.push(measure(
                "repeat-encode baseline",
                repeated_encode_baseline,
                &event,
            ));
            optimized_times.push(measure("single-encode", single_encode_path, &event));
        } else {
            optimized_times.push(measure("single-encode", single_encode_path, &event));
            baseline_times.push(measure(
                "repeat-encode baseline",
                repeated_encode_baseline,
                &event,
            ));
        }
    }
    baseline_times.sort_unstable();
    optimized_times.sort_unstable();
    let baseline_median = baseline_times[ROUNDS / 2];
    let optimized_median = optimized_times[ROUNDS / 2];
    println!(
        "median speedup: {:.2}x (synthetic fixed five-level TWSE quote; {ITERATIONS} iterations/round)",
        baseline_median as f64 / optimized_median as f64,
    );
}
