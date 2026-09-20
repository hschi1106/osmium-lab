use market_state::{
    MarketPhase, MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy,
    SessionSegmentId, StateField,
};
use market_types::{
    AuctionObservation, BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent,
    EventPayload, IndicativeAuction, InstrumentId, MarketAnnotations, MarketId, MarketSignal,
    MatchTime, Observation, Price, Quantity, QuantityUnit, QuoteSnapshot, SourceFormatId, Symbol,
    TradingDate, UnknownValue,
};
use replay_engine::ReplayCore;
use strategy_api::{
    MarketTradingContextEvaluator, MatchingState, NewOrderEntry, OrderBlockReason,
    OrderRestrictionReason, SessionKind, SessionSegment, TradingContext,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap())
}

fn date() -> TradingDate {
    TradingDate::parse("2026-07-27").unwrap()
}

fn segment() -> SessionSegment {
    SessionSegment::new(
        SessionSegmentId::new("regular").unwrap(),
        SessionKind::Regular,
        date(),
        MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap(),
        MatchTime::parse("2026-07-27T13:30:00+08:00").unwrap(),
    )
    .unwrap()
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

fn book() -> CompleteBookSnapshot {
    let quantity = Quantity::new(1, QuantityUnit::TradingUnit).unwrap();
    CompleteBookSnapshot::new(
        BookSide::new(
            BookSideKind::Bid,
            vec![BookLevel::new(Price::parse("100").unwrap(), quantity)],
        )
        .unwrap(),
        BookSide::new(
            BookSideKind::Ask,
            vec![BookLevel::new(Price::parse("101").unwrap(), quantity)],
        )
        .unwrap(),
    )
    .unwrap()
}

fn event(time: &str, payload: EventPayload) -> DomainEvent {
    DomainEvent::new(
        instrument(),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::parse(time).unwrap(),
        None,
        payload,
    )
}

fn quote(time: &str, signal: Observation<MarketSignal>) -> DomainEvent {
    event(
        time,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book(),
                Observation::NoObservation,
                Observation::NoObservation,
                MarketAnnotations::None,
            )
            .unwrap()
            .with_market_signal(signal),
        ),
    )
}

fn auction(time: &str, observation: AuctionObservation) -> DomainEvent {
    event(
        time,
        EventPayload::IndicativeAuction(
            IndicativeAuction::new(
                observation,
                Observation::NoObservation,
                Observation::NoObservation,
                Observation::NoObservation,
                Observation::NoObservation,
                MarketAnnotations::None,
            )
            .unwrap(),
        ),
    )
}

fn evaluate(
    replay: &mut ReplayCore,
    event: DomainEvent,
    segment: &SessionSegment,
) -> TradingContext {
    let commit = replay.apply_ordered(&event).unwrap();
    let state = replay.state(event.instrument()).unwrap().view();
    MarketTradingContextEvaluator::evaluate(&event, commit.occurrence(), state, segment).unwrap()
}

#[test]
fn opening_uncross_uses_call_auction_for_this_event_but_continuous_for_post_state() {
    let segment = segment();
    let mut replay = core();
    let opening = AuctionObservation::opening(false, false);
    let context = evaluate(
        &mut replay,
        quote(
            "2026-07-27T09:00:00+08:00",
            Observation::Set(MarketSignal::AuctionUncross(opening)),
        ),
        &segment,
    );
    assert_eq!(
        context.matching(),
        MatchingState::Enabled(market_types::MatchingMethod::CallAuction)
    );
    assert_eq!(context.new_order_entry(), NewOrderEntry::Allowed);
    assert_eq!(
        replay.state(&instrument()).unwrap().view().phase().known(),
        Some(&MarketPhase::Continuous)
    );

    let next = evaluate(
        &mut replay,
        quote(
            "2026-07-27T09:00:01+08:00",
            Observation::Set(MarketSignal::Continuous),
        ),
        &segment,
    );
    assert_eq!(
        next.matching(),
        MatchingState::Enabled(market_types::MatchingMethod::Continuous)
    );
}

#[test]
fn unknown_auction_uncross_keeps_call_auction_but_does_not_allow_entry() {
    let segment = segment();
    let mut replay = core();
    let context = evaluate(
        &mut replay,
        quote(
            "2026-07-27T09:00:00+08:00",
            Observation::Set(MarketSignal::AuctionUncross(
                AuctionObservation::unclassified(),
            )),
        ),
        &segment,
    );
    assert_eq!(
        context.matching(),
        MatchingState::Enabled(market_types::MatchingMethod::CallAuction)
    );
    assert_eq!(context.new_order_entry(), NewOrderEntry::Unknown);
    assert!(matches!(
        replay.state(&instrument()).unwrap().phase(),
        StateField::Unknown { .. }
    ));
}

#[test]
fn closing_result_blocks_new_orders_then_closed_state_remains_closed() {
    let segment = segment();
    let mut replay = core();
    let closing = AuctionObservation::closing(true, true);
    let result = evaluate(
        &mut replay,
        quote(
            "2026-07-27T13:29:59+08:00",
            Observation::Set(MarketSignal::AuctionUncross(closing)),
        ),
        &segment,
    );
    assert_eq!(
        result.matching(),
        MatchingState::Enabled(market_types::MatchingMethod::CallAuction)
    );
    assert_eq!(
        result.new_order_entry(),
        NewOrderEntry::Blocked(OrderBlockReason::ClosingResult)
    );

    let closed = evaluate(
        &mut replay,
        event(
            "2026-07-27T13:30:00+08:00",
            EventPayload::MarketStatus(
                market_types::MarketStatusObservation::new(
                    Observation::NoObservation,
                    MarketAnnotations::None,
                )
                .with_market_signal(Observation::Set(MarketSignal::Closed)),
            ),
        ),
        &segment,
    );
    assert_eq!(closed.matching(), MatchingState::Closed);
    assert_eq!(
        closed.new_order_entry(),
        NewOrderEntry::Blocked(OrderBlockReason::ClosingResult)
    );
}

#[test]
fn periodic_and_volatility_interruption_share_call_auction_without_old_reason_taxonomy() {
    let segment = segment();
    let mut replay = core();
    let periodic = AuctionObservation::periodic(true, true);
    let periodic_context = evaluate(
        &mut replay,
        auction("2026-07-27T10:00:00+08:00", periodic),
        &segment,
    );
    assert_eq!(
        periodic_context.matching(),
        MatchingState::Enabled(market_types::MatchingMethod::CallAuction)
    );
    assert_eq!(
        periodic_context.new_order_entry(),
        NewOrderEntry::Restricted(OrderRestrictionReason::AuctionCollecting)
    );
    assert_eq!(periodic_context.auction(), Some(periodic));

    let interruption = AuctionObservation::volatility_interruption(
        market_types::VolatilityDirection::Up,
        false,
        false,
    );
    let interruption_context = evaluate(
        &mut replay,
        auction("2026-07-27T10:00:01+08:00", interruption),
        &segment,
    );
    assert_eq!(
        interruption_context.matching(),
        MatchingState::Enabled(market_types::MatchingMethod::CallAuction)
    );
    assert_eq!(
        interruption_context.new_order_entry(),
        NewOrderEntry::Restricted(OrderRestrictionReason::AuctionCollecting)
    );
}

#[test]
fn no_observation_falls_back_to_state_but_unknown_does_not_fall_back_to_continuous() {
    let segment = segment();
    let mut replay = core();
    let opening = AuctionObservation::opening(false, false);
    let _ = evaluate(
        &mut replay,
        auction("2026-07-27T11:00:00+08:00", opening),
        &segment,
    );
    let carried = evaluate(
        &mut replay,
        quote("2026-07-27T11:00:01+08:00", Observation::NoObservation),
        &segment,
    );
    assert_eq!(
        carried.matching(),
        MatchingState::Enabled(market_types::MatchingMethod::CallAuction)
    );

    let unknown = evaluate(
        &mut replay,
        quote(
            "2026-07-27T11:00:02+08:00",
            Observation::Unknown(UnknownValue::Unsigned(9)),
        ),
        &segment,
    );
    assert_eq!(unknown.matching(), MatchingState::Unknown);
    assert_eq!(unknown.new_order_entry(), NewOrderEntry::Unknown);
}
