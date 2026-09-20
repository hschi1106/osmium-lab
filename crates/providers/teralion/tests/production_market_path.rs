use market_state::{
    MarketPhase, MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy,
    SessionSegmentId,
};
use market_types::{
    AuctionEvidence, AuctionObservation, AuctionPurpose, DomainEvent, EventPayload, InstrumentId,
    MarketAnnotations, MarketId, MarketSignal, MarketStatusObservation, MatchTime, MatchingMethod,
    Observation, SourceFormatId, Symbol, TradingDate,
};
use replay_engine::ReplayCore;
use strategy_api::{
    MarketTradingContextEvaluator, NewOrderEntry, OrderRestrictionReason, SessionKind,
    SessionSegment,
};
use teralion_provider::twse::{NormalizerConfig, TwseNormalizer};

fn instrument() -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new("SYNTH-PROD").unwrap())
}

fn date() -> TradingDate {
    TradingDate::parse("2026-07-27").unwrap()
}

fn normalizer() -> TwseNormalizer {
    TwseNormalizer::new(
        NormalizerConfig::new(
            instrument(),
            date(),
            MatchTime::parse("2026-07-27T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-27T13:35:00+08:00").unwrap(),
        )
        .unwrap(),
    )
}

fn quote_line(match_time: &str, status_flags: u8, deal_price: &str, cumulative: u64) -> String {
    let received_at = match_time.replace("+08:00", ".001000+08:00");
    format!(
        r#"{{"asks":[{{"price":101,"quantity":6}}],"bids":[{{"price":99,"quantity":7}}],"cum_volume":{cumulative},"deal":{{"price":{deal_price},"quantity":1}},"format":"STOCK_SNAPSHOT","intermediate_print":false,"limit_flags":0,"market":"twse","match_time":"{match_time}","received_at":"{received_at}","status_flags":{status_flags},"symbol":"SYNTH-PROD","type":"quote"}}"#,
    )
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

fn neutral_periodic_status(match_time: &str, signal: MarketSignal) -> DomainEvent {
    DomainEvent::new(
        instrument(),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::parse(match_time).unwrap(),
        None,
        EventPayload::MarketStatus(
            MarketStatusObservation::new(Observation::NoObservation, MarketAnnotations::None)
                .with_market_signal(Observation::Set(signal)),
        ),
    )
}

#[test]
fn missing_market_signal_replays_unrelated_event_as_unknown() {
    let event = DomainEvent::new(
        instrument(),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::parse("2026-07-27T09:01:00+08:00").unwrap(),
        None,
        EventPayload::MarketStatus(MarketStatusObservation::new(
            Observation::NoObservation,
            MarketAnnotations::None,
        )),
    );
    let mut core = core();
    let commit = core.apply_ordered(&event).unwrap();
    let context = MarketTradingContextEvaluator::evaluate(
        &event,
        commit.occurrence(),
        core.state(&instrument()).unwrap().view(),
        &segment(),
    )
    .unwrap();
    assert_eq!(context.matching(), strategy_api::MatchingState::Unknown);
    assert_eq!(context.new_order_entry(), NewOrderEntry::Unknown);
    assert!(core.state(&instrument()).unwrap().phase().known().is_none());
}

#[test]
fn provider_events_reach_production_reducer_and_context_without_background_lookup() {
    let report = normalizer()
        .normalize_json_lines([
            quote_line("2026-07-27T09:04:00+08:00", 0x80, "100", 1),
            quote_line("2026-07-27T09:04:01+08:00", 0x10, "100.5", 2),
        ])
        .unwrap();
    assert_eq!(report.events().len(), 2);

    let mut core = core();
    let segment = segment();
    let trial = &report.events()[0];
    let trial_commit = core.apply_ordered(trial).unwrap();
    let trial_context = MarketTradingContextEvaluator::evaluate(
        trial,
        trial_commit.occurrence(),
        core.state(&instrument()).unwrap().view(),
        &segment,
    )
    .unwrap();
    assert_eq!(
        trial_context.matching(),
        strategy_api::MatchingState::Enabled(MatchingMethod::CallAuction)
    );
    assert_eq!(
        trial_context.new_order_entry(),
        NewOrderEntry::Restricted(OrderRestrictionReason::AuctionCollecting)
    );
    assert_eq!(
        trial_context.auction().unwrap().purpose(),
        AuctionEvidence::NoObservation
    );
    assert!(matches!(
        core.state(&instrument()).unwrap().phase().known(),
        Some(MarketPhase::Auction(auction))
            if auction.purpose() == AuctionEvidence::NoObservation
    ));

    let firm = &report.events()[1];
    core.apply_ordered(firm).unwrap();
    assert_eq!(
        core.state(&instrument()).unwrap().phase().known(),
        Some(&MarketPhase::Continuous)
    );

    let disposal = AuctionObservation::periodic(false, true);
    let collecting = neutral_periodic_status(
        "2026-07-27T09:05:00+08:00",
        MarketSignal::AuctionCollecting(disposal),
    );
    let collecting_commit = core.apply_ordered(&collecting).unwrap();
    let result = neutral_periodic_status(
        "2026-07-27T09:05:01+08:00",
        MarketSignal::AuctionUncross(disposal),
    );
    let result_commit = core.apply_ordered(&result).unwrap();
    let result_context = MarketTradingContextEvaluator::evaluate(
        &result,
        result_commit.occurrence(),
        core.state(&instrument()).unwrap().view(),
        &segment,
    )
    .unwrap();
    assert_eq!(
        result_context.matching(),
        strategy_api::MatchingState::Enabled(MatchingMethod::CallAuction)
    );
    assert_eq!(
        result_context.new_order_entry(),
        NewOrderEntry::Restricted(OrderRestrictionReason::AuctionCollecting)
    );
    assert_eq!(
        collecting_commit.transition().new_version(),
        result_commit.transition().previous_version()
    );
    assert!(matches!(
        core.state(&instrument()).unwrap().phase().known(),
        Some(MarketPhase::Auction(auction))
            if auction.purpose() == AuctionEvidence::Known(AuctionPurpose::Periodic)
                && auction.disposal() == AuctionEvidence::Known(true)
    ));
}
