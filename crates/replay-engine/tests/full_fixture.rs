use market_state::{
    LastTrade, MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy,
    SessionSegmentId,
};
use market_types::{
    BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent, EventPayload,
    InstrumentId, MarketAnnotations, MarketId, MatchTime, Observation, ObservedTrade, Price,
    Quantity, QuantityUnit, QuoteSnapshot, SourceFormatId, Symbol, TradeObservationKind,
    TradingDate, TwseQuoteAnnotations, Volume,
};
use replay_engine::{ReplayCore, order_events};

fn instrument() -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new("SYNTH-CORE").unwrap())
}

fn date() -> TradingDate {
    TradingDate::parse("2026-07-20").unwrap()
}

fn book(bid: &str, ask: &str) -> CompleteBookSnapshot {
    let level = |price: &str| {
        BookLevel::new(
            Price::parse(price).unwrap(),
            Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
        )
    };
    CompleteBookSnapshot::new(
        BookSide::new(BookSideKind::Bid, vec![level(bid)]).unwrap(),
        BookSide::new(BookSideKind::Ask, vec![level(ask)]).unwrap(),
    )
    .unwrap()
}

fn quote(
    micros: i64,
    source_format: &str,
    snapshot: CompleteBookSnapshot,
    trade: Observation<ObservedTrade>,
    volume: u64,
    sequence: u64,
) -> DomainEvent {
    DomainEvent::new(
        instrument(),
        date(),
        SourceFormatId::new(source_format).unwrap(),
        MatchTime::from_unix_microseconds(micros),
        Some(sequence),
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                snapshot,
                trade,
                Observation::Set(Volume::new(volume, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0)),
            )
            .unwrap(),
        ),
    )
}

fn neutral_events() -> Vec<DomainEvent> {
    vec![
        quote(
            1,
            "STOCK_SNAPSHOT",
            book("100", "101"),
            Observation::Set(ObservedTrade::new(
                Price::parse("101").unwrap(),
                Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
                TradeObservationKind::Regular,
            )),
            1,
            1,
        ),
        quote(
            2,
            "STOCK_REALTIME",
            book("101", "102"),
            Observation::NoObservation,
            2,
            2,
        ),
    ]
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
fn neutral_domain_events_replay_is_deterministic_and_preserves_event_order() {
    let events = neutral_events();
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
    assert_eq!(state.cumulative_volume().known().unwrap().value(), 2);
    let LastTrade::Known(last_trade) = state.view().last_trade() else {
        panic!("neutral quote event should expose its observed trade")
    };
    assert_eq!(last_trade.price(), Price::parse("101").unwrap());
    assert!(first.summary().last_match_time().is_some());
}
