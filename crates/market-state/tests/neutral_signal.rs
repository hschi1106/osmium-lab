use market_state::{
    AuctionState, MarketPhase, MarketState, MarketStateReducer, ReducerContext,
    SegmentBoundaryPolicy, SessionSegmentId, StateField, UnavailableReason,
};
use market_types::{
    AuctionEvidence, AuctionObservation, AuctionPurpose, BookLevel, BookSide, BookSideKind,
    CompleteBookSnapshot, DomainEvent, EventPayload, IndicativeAuction, InstrumentId,
    MarketAnnotations, MarketId, MarketSignal, MatchTime, Observation, Price, Quantity,
    QuantityUnit, QuoteSnapshot, SourceFormatId, Symbol, TradingDate, UnknownValue,
    VolatilityDirection,
};

fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new(symbol).unwrap())
}

fn date() -> TradingDate {
    TradingDate::parse("2026-07-27").unwrap()
}

fn context(segment: &str, policy: SegmentBoundaryPolicy) -> ReducerContext {
    ReducerContext::new(date(), SessionSegmentId::new(segment).unwrap(), policy, 1)
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

fn event(micros: i64, payload: EventPayload) -> DomainEvent {
    DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::from_unix_microseconds(micros),
        None,
        payload,
    )
}

fn auction(micros: i64, observation: AuctionObservation) -> DomainEvent {
    event(
        micros,
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

fn quote(
    micros: i64,
    book_value: CompleteBookSnapshot,
    signal: Observation<MarketSignal>,
) -> DomainEvent {
    event(
        micros,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book_value,
                Observation::NoObservation,
                Observation::NoObservation,
                MarketAnnotations::None,
            )
            .unwrap()
            .with_market_signal(signal),
        ),
    )
}

fn apply(
    reducer: &MarketStateReducer,
    state: &mut MarketState,
    event: &DomainEvent,
    segment: &str,
    policy: SegmentBoundaryPolicy,
) {
    reducer
        .apply(state, event, &context(segment, policy))
        .unwrap();
}

#[test]
fn opening_and_delayed_opening_uncross_end_in_continuous_phase() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());

    apply(
        &reducer,
        &mut state,
        &auction(1, AuctionObservation::opening(false, false)),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    apply(
        &reducer,
        &mut state,
        &auction(2, AuctionObservation::opening(true, false)),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert_eq!(
        state.phase().known(),
        Some(&MarketPhase::Auction(AuctionState::new(
            AuctionObservation::opening(true, false),
        )))
    );

    let result = quote(
        3,
        book("100", "101"),
        Observation::Set(MarketSignal::AuctionUncross(AuctionObservation::opening(
            true, false,
        ))),
    );
    apply(
        &reducer,
        &mut state,
        &result,
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert_eq!(state.phase().known(), Some(&MarketPhase::Continuous));
    assert_eq!(
        state.market_signal().known(),
        Some(&MarketSignal::AuctionUncross(AuctionObservation::opening(
            true, false,
        )))
    );
}

#[test]
fn closing_uncross_closes_without_erasing_the_last_firm_book() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let firm = quote(
        1,
        book("100", "101"),
        Observation::Set(MarketSignal::Continuous),
    );
    apply(
        &reducer,
        &mut state,
        &firm,
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    let firm_book = state.book().clone();
    apply(
        &reducer,
        &mut state,
        &auction(2, AuctionObservation::closing(true, true)),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    let result = quote(
        3,
        book("90", "91"),
        Observation::Set(MarketSignal::AuctionUncross(AuctionObservation::closing(
            true, true,
        ))),
    );
    apply(
        &reducer,
        &mut state,
        &result,
        "regular",
        SegmentBoundaryPolicy::Carry,
    );

    assert_eq!(state.phase().known(), Some(&MarketPhase::Closed));
    assert_eq!(state.book().known().unwrap(), &book("90", "91"));
    assert_ne!(state.book(), &firm_book);
    assert_eq!(
        state.market_signal().known(),
        Some(&MarketSignal::AuctionUncross(AuctionObservation::closing(
            true, true,
        )))
    );
}

#[test]
fn periodic_uncross_keeps_the_next_periodic_auction_and_repeated_delay_has_no_counter() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let observation = AuctionObservation::periodic(true, true);
    for micros in [1, 2] {
        apply(
            &reducer,
            &mut state,
            &auction(micros, observation),
            "regular",
            SegmentBoundaryPolicy::Carry,
        );
    }
    assert_eq!(
        state.phase().known(),
        Some(&MarketPhase::Auction(AuctionState::new(observation)))
    );

    let result = quote(
        3,
        book("100", "101"),
        Observation::Set(MarketSignal::AuctionUncross(observation)),
    );
    apply(
        &reducer,
        &mut state,
        &result,
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert_eq!(
        state.phase().known(),
        Some(&MarketPhase::Auction(AuctionState::new(observation)))
    );
    let Some(MarketPhase::Auction(auction)) = state.phase().known() else {
        panic!("periodic uncross must retain the next auction phase")
    };
    assert_eq!(auction.delayed(), AuctionEvidence::Known(true));
    assert_eq!(auction.disposal(), AuctionEvidence::Known(true));
}

#[test]
fn auction_no_observation_carries_same_round_fields_but_unknown_invalidates_only_that_field() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let interruption = AuctionObservation::volatility_interruption_with_evidence(
        AuctionEvidence::Known(VolatilityDirection::Down),
        AuctionEvidence::Known(true),
        AuctionEvidence::NoObservation,
    );
    apply(
        &reducer,
        &mut state,
        &quote(
            1,
            book("100", "101"),
            Observation::Set(MarketSignal::AuctionCollecting(interruption)),
        ),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );

    apply(
        &reducer,
        &mut state,
        &quote(
            2,
            book("100", "101"),
            Observation::Set(MarketSignal::AuctionCollecting(
                AuctionObservation::unclassified(),
            )),
        ),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    let Some(MarketPhase::Auction(auction)) = state.phase().known() else {
        panic!("partial trial must retain the auction phase")
    };
    assert_eq!(
        auction.purpose(),
        AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
    );
    assert_eq!(auction.delayed(), AuctionEvidence::Known(true));
    assert_eq!(
        auction.direction(),
        AuctionEvidence::Known(VolatilityDirection::Down)
    );

    apply(
        &reducer,
        &mut state,
        &quote(
            3,
            book("100", "101"),
            Observation::Set(MarketSignal::AuctionCollecting(
                AuctionObservation::unknown_purpose(),
            )),
        ),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    let Some(MarketPhase::Auction(auction)) = state.phase().known() else {
        panic!("unknown purpose still identifies an auction collection event")
    };
    assert_eq!(auction.purpose(), AuctionEvidence::Unknown);
    assert_eq!(auction.delayed(), AuctionEvidence::Known(true));
}

#[test]
fn uncross_without_known_purpose_does_not_infer_a_post_state() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    apply(
        &reducer,
        &mut state,
        &quote(
            1,
            book("100", "101"),
            Observation::Set(MarketSignal::AuctionUncross(
                AuctionObservation::unclassified(),
            )),
        ),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert!(matches!(state.phase(), StateField::Unknown { .. }));
}

#[test]
fn indicative_payloads_merge_partial_observations_before_state_publication() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let interruption = AuctionObservation::volatility_interruption_with_evidence(
        AuctionEvidence::Known(VolatilityDirection::Up),
        AuctionEvidence::NoObservation,
        AuctionEvidence::NoObservation,
    );
    apply(
        &reducer,
        &mut state,
        &auction(1, interruption),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    apply(
        &reducer,
        &mut state,
        &auction(2, AuctionObservation::unclassified()),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );

    let StateField::Known { value, .. } = state.indicative_auction() else {
        panic!("indicative payload must be known")
    };
    assert_eq!(
        value.observation().purpose(),
        AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
    );
    assert_eq!(
        value.observation().direction(),
        AuctionEvidence::Known(VolatilityDirection::Up)
    );
}

#[test]
fn volatility_interruption_result_returns_to_continuous_without_using_a_trial_book() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let firm = quote(
        1,
        book("100", "101"),
        Observation::Set(MarketSignal::Continuous),
    );
    apply(
        &reducer,
        &mut state,
        &firm,
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    let firm_book = state.book().clone();
    let interruption =
        AuctionObservation::volatility_interruption(VolatilityDirection::Down, false, false);
    apply(
        &reducer,
        &mut state,
        &auction(2, interruption),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert_eq!(
        state.phase().known(),
        Some(&MarketPhase::Auction(AuctionState::new(interruption)))
    );
    apply(
        &reducer,
        &mut state,
        &quote(
            3,
            book("90", "91"),
            Observation::Set(MarketSignal::AuctionUncross(interruption)),
        ),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert_eq!(state.phase().known(), Some(&MarketPhase::Continuous));
    assert_ne!(state.book(), &firm_book);
}

#[test]
fn no_observation_keeps_phase_but_unknown_signal_is_not_continuous() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let observation = AuctionObservation::opening(false, false);
    apply(
        &reducer,
        &mut state,
        &auction(1, observation),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    apply(
        &reducer,
        &mut state,
        &quote(2, book("100", "101"), Observation::NoObservation),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert_eq!(
        state.phase().known(),
        Some(&MarketPhase::Auction(AuctionState::new(observation)))
    );

    apply(
        &reducer,
        &mut state,
        &quote(
            3,
            book("100", "101"),
            Observation::Unknown(UnknownValue::Unsigned(7)),
        ),
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert!(matches!(state.phase(), StateField::Unknown { .. }));
    assert!(matches!(
        state.market_signal(),
        StateField::Unknown {
            raw: UnknownValue::Unsigned(7),
            ..
        }
    ));
}

#[test]
fn session_reset_clears_auction_context_and_states_are_instrument_local() {
    let reducer = MarketStateReducer::twse_regular();
    let mut first = MarketState::new(instrument("2330"), date());
    let second = MarketState::new(instrument("2317"), date());
    let opening = auction(1, AuctionObservation::opening(false, false));
    apply(
        &reducer,
        &mut first,
        &opening,
        "regular",
        SegmentBoundaryPolicy::Carry,
    );
    assert!(second.phase().known().is_none());
    assert!(matches!(
        second.phase(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));

    let resumed = quote(
        2,
        book("100", "101"),
        Observation::Set(MarketSignal::Continuous),
    );
    apply(
        &reducer,
        &mut first,
        &resumed,
        "next",
        SegmentBoundaryPolicy::ResetObservableFields,
    );
    assert_eq!(first.phase().known(), Some(&MarketPhase::Continuous));
    assert!(matches!(
        first.indicative_auction(),
        StateField::Unavailable(UnavailableReason::Initial)
            | StateField::Unavailable(UnavailableReason::Cleared { .. })
    ));
}

#[test]
fn equivalent_neutral_event_sequences_have_the_same_final_state_checksum() {
    let reducer = MarketStateReducer::twse_regular();
    let events = [
        quote(
            1,
            book("100", "101"),
            Observation::Set(MarketSignal::Continuous),
        ),
        auction(2, AuctionObservation::periodic(false, false)),
        quote(
            3,
            book("101", "102"),
            Observation::Set(MarketSignal::AuctionUncross(AuctionObservation::periodic(
                false, false,
            ))),
        ),
    ];
    let mut left = MarketState::new(instrument("2330"), date());
    let mut right = MarketState::new(instrument("2330"), date());
    for event in &events {
        apply(
            &reducer,
            &mut left,
            event,
            "regular",
            SegmentBoundaryPolicy::Carry,
        );
        apply(
            &reducer,
            &mut right,
            event,
            "regular",
            SegmentBoundaryPolicy::Carry,
        );
    }
    assert_eq!(left.fingerprint().unwrap(), right.fingerprint().unwrap());
}
