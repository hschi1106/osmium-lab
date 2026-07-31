use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
    StateField, StateTransitionError,
};
use market_types::{
    BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent, EventPayload,
    InstrumentId, MarketAnnotations, MarketId, MatchTime, Observation, Price, Quantity,
    QuantityUnit, QuoteSnapshot, SourceFormatId, Symbol, TradeBatch, TradeOrder, TradePrint,
    TradePrintKind, TradingDate, TwseQuoteAnnotations, Volume,
};
use replay_engine::{
    OrderingError, OrderingKey, ReplayClock, ReplayCore, ReplayError, order_events,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap())
}

fn date() -> TradingDate {
    TradingDate::parse("2026-07-27").unwrap()
}

fn context() -> ReducerContext {
    ReducerContext::new(
        date(),
        SessionSegmentId::new("regular").unwrap(),
        SegmentBoundaryPolicy::Carry,
        1,
    )
}

fn level(price: &str) -> BookLevel {
    BookLevel::new(
        Price::parse(price).unwrap(),
        Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
    )
}

fn book() -> CompleteBookSnapshot {
    CompleteBookSnapshot::new(
        BookSide::new(BookSideKind::Bid, vec![level("100")]).unwrap(),
        BookSide::new(BookSideKind::Ask, vec![level("101")]).unwrap(),
    )
    .unwrap()
}

fn trade(price: &str, quantity: u64, kind: TradePrintKind) -> TradePrint {
    TradePrint::new(
        Price::parse(price).unwrap(),
        Quantity::new(quantity, QuantityUnit::TradingUnit).unwrap(),
        kind,
    )
}

fn quote(micros: i64, format: &str, cumulative: u64, sequence: Option<u64>) -> DomainEvent {
    DomainEvent::new(
        instrument(),
        date(),
        SourceFormatId::new(format).unwrap(),
        MatchTime::from_unix_microseconds(micros),
        sequence,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book(),
                Observation::Set(trade("100", 1, TradePrintKind::Regular)),
                Observation::Set(Volume::new(cumulative, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0)),
            )
            .unwrap(),
        ),
    )
}

fn intermediate(micros: i64, cumulative: u64) -> DomainEvent {
    DomainEvent::new(
        instrument(),
        date(),
        SourceFormatId::new("STOCK_REALTIME").unwrap(),
        MatchTime::from_unix_microseconds(micros),
        None,
        EventPayload::TradeBatch(
            TradeBatch::new(
                vec![trade("99.5", 1, TradePrintKind::Intermediate)],
                TradeOrder::SourceOrdered,
                Observation::Set(Volume::new(cumulative, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0)),
            )
            .unwrap(),
        ),
    )
}

fn core() -> ReplayCore {
    ReplayCore::new(
        vec![MarketState::new(instrument(), date())],
        MarketStateReducer::twse_regular(),
        context(),
    )
    .unwrap()
}

#[test]
fn ordering_v2_places_twse_intermediate_before_final_and_retains_duplicates() {
    let intermediate = intermediate(10, 10);
    let final_quote = quote(10, "STOCK_REALTIME", 11, None);
    let snapshot = quote(10, "STOCK_SNAPSHOT", 11, None);
    let duplicate = snapshot.clone();

    let ordered = order_events(vec![
        duplicate,
        final_quote.clone(),
        snapshot.clone(),
        intermediate.clone(),
    ])
    .unwrap();
    assert_eq!(ordered.len(), 4);
    assert_eq!(ordered[0], intermediate);
    assert_eq!(ordered[1], final_quote);
    assert_eq!(ordered[2], snapshot);
    assert_eq!(ordered[3], snapshot);
    assert_eq!(
        OrderingKey::for_event(&ordered[0])
            .unwrap()
            .source_phase_rank(),
        10
    );
    assert_eq!(
        OrderingKey::for_event(&ordered[1])
            .unwrap()
            .source_phase_rank(),
        20
    );
}

#[test]
fn source_sequence_none_sorts_before_some_before_fingerprint() {
    let without = quote(10, "STOCK_SNAPSHOT", 10, None);
    let with = quote(10, "STOCK_SNAPSHOT", 10, Some(0));
    let ordered = order_events(vec![with.clone(), without.clone()]).unwrap();
    assert_eq!(ordered, vec![without, with]);
}

#[test]
fn invalid_twse_realtime_trade_shape_is_rejected() {
    let event = DomainEvent::new(
        instrument(),
        date(),
        SourceFormatId::new("STOCK_REALTIME").unwrap(),
        MatchTime::from_unix_microseconds(10),
        None,
        EventPayload::TradeBatch(
            TradeBatch::new(
                vec![trade("100", 1, TradePrintKind::Regular)],
                TradeOrder::SourceOrdered,
                Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0)),
            )
            .unwrap(),
        ),
    );
    assert!(matches!(
        order_events(vec![event]),
        Err(OrderingError::InvalidTwseRealtimeShape)
    ));
}

#[test]
fn replay_clock_state_and_checksum_commit_once_per_event() {
    let mut core = core();
    let first = quote(10, "STOCK_SNAPSHOT", 10, None);
    let commit = core.apply_ordered(&first).unwrap();
    assert_eq!(commit.occurrence().run_event_ordinal(), 1);
    assert_eq!(commit.occurrence().instrument_state_version(), 1);
    assert_eq!(
        core.clock(),
        ReplayClock::At {
            match_time: MatchTime::from_unix_microseconds(10),
            event_ordinal: 1
        }
    );

    let duplicate = core.apply_ordered(&first).unwrap();
    assert_eq!(duplicate.occurrence().run_event_ordinal(), 2);
    assert_eq!(duplicate.occurrence().instrument_state_version(), 2);
    assert_eq!(core.state(&instrument()).unwrap().state_version(), 2);
}

#[test]
fn precommit_ordering_and_state_errors_leave_current_event_uncommitted() {
    let mut core = core();
    core.apply_ordered(&quote(10, "STOCK_SNAPSHOT", 10, None))
        .unwrap();
    let before_clock = core.clock();
    let before_state = core.state(&instrument()).unwrap().clone();
    let before_checksum = core.processed_prefix_checksum();

    let ordering_error = core
        .apply_ordered(&quote(9, "STOCK_SNAPSHOT", 11, None))
        .unwrap_err();
    assert!(matches!(
        ordering_error,
        ReplayError::GlobalOrderingRegression
    ));
    assert_eq!(core.clock(), before_clock);
    assert_eq!(core.state(&instrument()), Some(&before_state));
    assert_eq!(core.processed_prefix_checksum(), before_checksum);

    let state_error = core
        .apply_ordered(&quote(11, "STOCK_SNAPSHOT", 9, None))
        .unwrap_err();
    assert!(matches!(
        state_error,
        ReplayError::StateTransition(StateTransitionError::CumulativeVolumeRegression {
            previous: 10,
            next: 9
        })
    ));
    assert_eq!(core.clock(), before_clock);
    assert_eq!(core.state(&instrument()), Some(&before_state));
    assert_eq!(core.processed_prefix_checksum(), before_checksum);
}

#[test]
fn shuffled_inputs_produce_identical_checksums_and_final_state() {
    let events = vec![
        quote(11, "STOCK_SNAPSHOT", 11, None),
        quote(10, "STOCK_SNAPSHOT", 10, None),
        quote(12, "STOCK_SNAPSHOT", 12, None),
    ];
    let mut reversed = events.clone();
    reversed.reverse();

    let mut first = core();
    first.replay(events).unwrap();
    let first = first.complete().unwrap();
    let mut second = core();
    second.replay(reversed).unwrap();
    let second = second.complete().unwrap();

    assert_eq!(
        first.summary().event_checksum(),
        second.summary().event_checksum()
    );
    assert_eq!(
        first.summary().final_state_checksum(),
        second.summary().final_state_checksum()
    );
    assert_eq!(first.summary().event_count(), 3);
    assert_eq!(
        first
            .state(&instrument())
            .unwrap()
            .cumulative_volume()
            .known()
            .unwrap()
            .value(),
        12
    );
}

#[test]
fn empty_selected_event_sequence_has_a_framed_checksum_and_initial_state() {
    let completed = core().complete().unwrap();
    assert_eq!(completed.summary().event_count(), 0);
    assert_eq!(completed.summary().first_match_time(), None);
    assert_eq!(completed.summary().last_match_time(), None);
    assert_eq!(
        completed.state(&instrument()).unwrap().book(),
        &StateField::initial()
    );

    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"OSRS");
    canonical.extend_from_slice(&1_u16.to_be_bytes());
    canonical.extend_from_slice(&1_u16.to_be_bytes());
    canonical.extend_from_slice(&1_u16.to_be_bytes());
    canonical.extend_from_slice(&2_u16.to_be_bytes());
    canonical.push(0);
    canonical.extend_from_slice(&0_u64.to_be_bytes());
    assert_eq!(
        completed.summary().event_checksum().as_bytes(),
        blake3::hash(&canonical).as_bytes()
    );
}
