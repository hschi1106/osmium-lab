use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{
    AuctionObservation, AuctionPurpose, BookLevel, BookSide, BookSideKind, CompleteBookSnapshot,
    DomainEvent, EventPayload, IndicativeAuction, InstrumentId, MarketAnnotations, MarketId,
    MarketSignal, MatchTime, Observation, Price, Quantity, QuantityUnit, QuoteSnapshot,
    SourceFormatId, Symbol, TradingDate, TwseQuoteAnnotations, VolatilityDirection, Volume,
};
use replay_engine::ReplayCore;
use strategy_api::{
    BinaryIdentity, CanonicalParamsChecksum, ExampleStrategy, IndicatorValue, MatchingState,
    NewOrderEntry, OrderBlockReason, OrderIntent, OrderRestrictionReason, OrderSide, OrderType,
    SessionKind, SessionPhase, SessionSegment, Strategy, StrategyDeclaration, StrategyEventContext,
    StrategyExecutionError, StrategyIdentity, StrategyOutputRecord, StrategyOutputSink,
    StrategyRunErrorCategory, run_strategy,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap())
}

fn date() -> TradingDate {
    TradingDate::parse("2026-07-27").unwrap()
}

fn segment_id() -> SessionSegmentId {
    SessionSegmentId::new("regular").unwrap()
}

fn segment() -> SessionSegment {
    SessionSegment::new(
        segment_id(),
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
        ReducerContext::new(date(), segment_id(), SegmentBoundaryPolicy::Carry, 1),
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

fn quote(time: &str, cumulative: u64, status: u8, limits: u8) -> DomainEvent {
    let annotations = TwseQuoteAnnotations::new(status, limits);
    let match_time = MatchTime::parse(time).unwrap();
    let payload = if annotations.status().trial() {
        let observation = if annotations.status().opening_marker() || match_time < segment().open()
        {
            AuctionObservation::opening(annotations.status().delayed_open(), false)
        } else if annotations.status().closing_marker() {
            AuctionObservation::closing(annotations.status().delayed_close(), false)
        } else {
            AuctionObservation::periodic(false, false)
        };
        EventPayload::IndicativeAuction(
            IndicativeAuction::new(
                observation,
                Observation::NoObservation,
                Observation::NoObservation,
                Observation::Set(book()),
                Observation::Set(Volume::new(cumulative, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(annotations),
            )
            .unwrap(),
        )
    } else {
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book(),
                Observation::NoObservation,
                Observation::Set(Volume::new(cumulative, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(annotations),
            )
            .unwrap()
            .with_market_signal(Observation::Set(
                if annotations.limits().instant_trend()
                    == market_types::InstantTrend::VolatilityInterruptionDown
                {
                    MarketSignal::AuctionCollecting(AuctionObservation::volatility_interruption(
                        VolatilityDirection::Down,
                        false,
                        false,
                    ))
                } else if annotations.limits().instant_trend()
                    == market_types::InstantTrend::VolatilityInterruptionUp
                {
                    MarketSignal::AuctionCollecting(AuctionObservation::volatility_interruption(
                        VolatilityDirection::Up,
                        false,
                        false,
                    ))
                } else if annotations.status().closing_marker() {
                    MarketSignal::AuctionUncross(AuctionObservation::closing(false, false))
                } else {
                    MarketSignal::Continuous
                },
            )),
        )
    };
    DomainEvent::new(
        instrument(),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        match_time,
        None,
        payload,
    )
}

fn delayed_open_auction() -> DomainEvent {
    quote("2026-07-27T08:59:59+08:00", 0, 0xc0, 0)
}

fn binary_identity() -> BinaryIdentity {
    BinaryIdentity::new("test-source-blake3", [7_u8; 32]).unwrap()
}

fn identity(id: &str) -> StrategyIdentity {
    StrategyIdentity::new(id, "1", binary_identity()).unwrap()
}

#[derive(Debug)]
struct ContextObserver {
    identity: StrategyIdentity,
    expected_versions: Vec<u64>,
    phases: Vec<SessionPhase>,
    matching: Vec<MatchingState>,
    order_entry: Vec<NewOrderEntry>,
}

impl ContextObserver {
    fn new() -> Self {
        Self {
            identity: identity("test.context-observer"),
            expected_versions: Vec::new(),
            phases: Vec::new(),
            matching: Vec::new(),
            order_entry: Vec::new(),
        }
    }
}

impl Strategy for ContextObserver {
    fn identity(&self) -> &StrategyIdentity {
        &self.identity
    }

    fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
        CanonicalParamsChecksum::for_empty_params()
    }

    fn declaration(&self) -> StrategyDeclaration {
        StrategyDeclaration::new([instrument()], [SessionKind::Regular]).unwrap()
    }

    fn on_event(
        &mut self,
        context: StrategyEventContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        assert_eq!(
            context.occurrence().instrument_state_version(),
            context.market_state().state_version()
        );
        assert_eq!(
            context.occurrence().event_fingerprint().as_bytes(),
            context.trading().event_fingerprint()
        );
        assert_eq!(
            context.event().match_time(),
            context.market_state().last_match_time().unwrap()
        );
        self.expected_versions
            .push(context.market_state().state_version());
        self.phases.push(context.session().phase());
        self.matching.push(context.trading().matching());
        self.order_entry.push(context.trading().new_order_entry());
        output.emit_indicator(
            "seen_version",
            IndicatorValue::Unsigned(context.market_state().state_version()),
        )?;
        if context.trading().auction().is_some_and(|auction| {
            auction.purpose() == AuctionPurpose::Opening && auction.delayed()
        }) {
            output.emit_indicator("delayed_open_seen", IndicatorValue::Bool(true))?;
        }
        Ok(())
    }
}

#[test]
fn callback_observes_committed_post_event_state_and_context() {
    let events = vec![
        quote("2026-07-27T08:55:00+08:00", 0, 0x80, 0),
        quote("2026-07-27T09:00:00+08:00", 1, 0x10, 0),
        quote("2026-07-27T13:30:00.000001+08:00", 1, 0x04, 0),
    ];
    let completed = run_strategy(core(), ContextObserver::new(), &segment(), events).unwrap();

    assert_eq!(completed.callback_count(), 3);
    assert_eq!(completed.replay().summary().event_count(), 3);
    assert_eq!(completed.strategy_output().records().len(), 3);
    assert!(matches!(
        completed.strategy_output().records()[0],
        StrategyOutputRecord::EventIndicator {
            instrument_state_version: 1,
            ..
        }
    ));
}

#[test]
fn strategy_callback_can_observe_explicit_delayed_open_signal() {
    let completed = run_strategy(
        core(),
        ContextObserver::new(),
        &segment(),
        vec![delayed_open_auction()],
    )
    .unwrap();

    assert_eq!(completed.callback_count(), 1);
    assert!(completed.strategy_output().records().iter().any(|record| {
        matches!(
            record,
            StrategyOutputRecord::EventIndicator {
                indicator_name,
                value: IndicatorValue::Bool(true),
                ..
            } if indicator_name.as_ref() == "delayed_open_seen"
        )
    }));
}

#[test]
fn boxed_strategy_runs_through_the_generic_lifecycle() {
    let strategy: Box<dyn Strategy> = Box::new(ContextObserver::new());
    let completed = run_strategy(
        core(),
        strategy,
        &segment(),
        vec![quote("2026-07-27T09:00:00+08:00", 1, 0x10, 0)],
    )
    .unwrap();
    assert_eq!(completed.callback_count(), 1);
}

#[test]
fn example_strategy_declares_only_2330_and_emits_in_fixed_order() {
    let strategy = ExampleStrategy::new(binary_identity(), instrument()).unwrap();
    assert_eq!(strategy.declaration().universe(), &[instrument()]);
    assert_eq!(strategy.declaration().sessions(), &[SessionKind::Regular]);

    let completed = run_strategy(
        core(),
        strategy,
        &segment(),
        vec![
            quote("2026-07-27T09:00:00+08:00", 10, 0x10, 0),
            quote("2026-07-27T09:00:01+08:00", 11, 0x10, 0),
        ],
    )
    .unwrap();
    assert_eq!(completed.callback_count(), 2);
    assert_eq!(completed.strategy_output().records().len(), 4);
    assert!(matches!(
        &completed.strategy_output().records()[0],
        StrategyOutputRecord::EventIndicator {
            output_sequence: 1,
            indicator_name,
            value: IndicatorValue::Unsigned(1),
            ..
        } if indicator_name.as_ref() == "state_version"
    ));
    assert!(matches!(
        &completed.strategy_output().records()[1],
        StrategyOutputRecord::EventIndicator {
            output_sequence: 2,
            indicator_name,
            value: IndicatorValue::Unsigned(10),
            ..
        } if indicator_name.as_ref() == "cum_volume"
    ));
}

#[test]
fn deterministic_output_is_independent_of_input_order() {
    let events = vec![
        quote("2026-07-27T09:00:00+08:00", 10, 0x10, 0),
        quote("2026-07-27T09:00:01+08:00", 11, 0x10, 0),
    ];
    let first = run_strategy(
        core(),
        ExampleStrategy::new(binary_identity(), instrument()).unwrap(),
        &segment(),
        events.clone(),
    )
    .unwrap();
    let second = run_strategy(
        core(),
        ExampleStrategy::new(binary_identity(), instrument()).unwrap(),
        &segment(),
        events.into_iter().rev().collect(),
    )
    .unwrap();
    assert_eq!(
        first.strategy_output().to_canonical_bytes().unwrap(),
        second.strategy_output().to_canonical_bytes().unwrap()
    );
    assert_eq!(
        first.strategy_output().checksum().unwrap(),
        second.strategy_output().checksum().unwrap()
    );
}

#[derive(Debug)]
struct FailingStrategy {
    identity: StrategyIdentity,
    mode: FailureMode,
}

#[derive(Debug, Clone, Copy)]
enum FailureMode {
    Error,
    Panic,
    Capability,
}

impl FailingStrategy {
    fn new(mode: FailureMode) -> Self {
        Self {
            identity: identity("test.failing"),
            mode,
        }
    }
}

impl Strategy for FailingStrategy {
    fn identity(&self) -> &StrategyIdentity {
        &self.identity
    }

    fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
        CanonicalParamsChecksum::for_empty_params()
    }

    fn declaration(&self) -> StrategyDeclaration {
        StrategyDeclaration::new([instrument()], [SessionKind::Regular]).unwrap()
    }

    fn on_event(
        &mut self,
        _context: StrategyEventContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        output.emit_indicator("discard_me", IndicatorValue::Bool(true))?;
        match self.mode {
            FailureMode::Error => Err(StrategyExecutionError::new("expected callback error")),
            FailureMode::Panic => panic!("expected test panic"),
            FailureMode::Capability => {
                output.emit_order_intent(OrderIntent::new(
                    instrument(),
                    OrderSide::Buy,
                    Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
                    OrderType::Market,
                ))?;
                Ok(())
            }
        }
    }
}

#[test]
fn failed_callback_discards_current_batch_after_core_commit() {
    let error = run_strategy(
        core(),
        FailingStrategy::new(FailureMode::Error),
        &segment(),
        vec![quote("2026-07-27T09:00:00+08:00", 1, 0x10, 0)],
    )
    .unwrap_err();
    assert_eq!(
        error.failure().category(),
        StrategyRunErrorCategory::Callback
    );
    assert_eq!(error.failure().processed_event_count(), 1);
    assert_eq!(error.failure().committed_output_count(), 0);
    let occurrence = error.failure().occurrence().unwrap();
    assert_eq!(occurrence.run_event_ordinal(), 1);
    assert_eq!(occurrence.instrument_state_version(), 1);
}

#[test]
fn panic_and_unavailable_capability_have_stable_categories() {
    let panic = run_strategy(
        core(),
        FailingStrategy::new(FailureMode::Panic),
        &segment(),
        vec![quote("2026-07-27T09:00:00+08:00", 1, 0x10, 0)],
    )
    .unwrap_err();
    assert_eq!(
        panic.failure().category(),
        StrategyRunErrorCategory::StrategyPanic
    );

    let capability = run_strategy(
        core(),
        FailingStrategy::new(FailureMode::Capability),
        &segment(),
        vec![quote("2026-07-27T09:00:00+08:00", 1, 0x10, 0)],
    )
    .unwrap_err();
    assert_eq!(
        capability.failure().category(),
        StrategyRunErrorCategory::CapabilityUnavailable
    );
}

#[test]
fn session_phase_and_twse_indicative_rules_are_explicit() {
    let cases = [
        (
            quote("2026-07-27T08:59:59+08:00", 0, 0x80, 0),
            SessionPhase::WarmUp,
            MatchingState::Enabled(market_types::MatchingMethod::CallAuction),
            NewOrderEntry::Restricted(OrderRestrictionReason::PreOpenLimitOrdersOnly),
        ),
        (
            quote("2026-07-27T09:00:00+08:00", 1, 0x10, 0),
            SessionPhase::Active,
            MatchingState::Enabled(market_types::MatchingMethod::Continuous),
            NewOrderEntry::Allowed,
        ),
        (
            quote("2026-07-27T09:01:00+08:00", 2, 0x10, 1),
            SessionPhase::Active,
            MatchingState::Enabled(market_types::MatchingMethod::CallAuction),
            NewOrderEntry::Restricted(OrderRestrictionReason::AuctionCollecting),
        ),
        (
            quote("2026-07-27T13:30:00.000001+08:00", 2, 0x04, 0),
            SessionPhase::CoolDown,
            MatchingState::Enabled(market_types::MatchingMethod::CallAuction),
            NewOrderEntry::Blocked(OrderBlockReason::CoolDown),
        ),
    ];
    let mut replay = core();
    for (event, phase, matching, order_entry) in cases {
        let commit = replay.apply_ordered(&event).unwrap();
        let state = replay.state(&instrument()).unwrap().view();
        let context = strategy_api::MarketTradingContextEvaluator::evaluate(
            &event,
            commit.occurrence(),
            state,
            &segment(),
        )
        .unwrap();
        assert_eq!(context.session().phase(), phase);
        assert_eq!(context.matching(), matching);
        assert_eq!(context.new_order_entry(), order_entry);
    }
}
