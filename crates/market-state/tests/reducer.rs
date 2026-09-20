use market_state::{
    CumulativeVolumePolicy, LastTrade, MarketState, MarketStateProfile, MarketStateReducer,
    ReducerContext, SegmentBoundaryPolicy, SessionSegmentId, SourceFormatRule, StateField,
    StateTransitionError, StateWarningCode, UnavailableReason, canonical_final_state_set,
    final_state_checksum,
};
use market_types::{
    AuctionEvidence, AuctionObservation, AuctionPurpose, BookLevel, BookSide, BookSideKind,
    BookSnapshot, CompleteBookSnapshot, DomainEvent, EventKind, EventPayload, IndicativeAuction,
    InstrumentId, MarketAnnotations, MarketId, MarketSignal, MarketStatusObservation, MatchTime,
    Observation, ObservedTrade, Price, Quantity, QuantityUnit, QuoteSnapshot, SourceFormatId,
    Symbol, TpexQuoteAnnotations, TradeBatch, TradeBatchOrdering, TradeObservationKind,
    TradingDate, TwseQuoteAnnotations, UnknownValue, VolatilityDirection, Volume,
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

fn level(price: &str, quantity: u64) -> BookLevel {
    BookLevel::new(
        Price::parse(price).unwrap(),
        Quantity::new(quantity, QuantityUnit::TradingUnit).unwrap(),
    )
}

fn book(bid_prices: &[&str], ask_prices: &[&str]) -> CompleteBookSnapshot {
    CompleteBookSnapshot::new(
        BookSide::new(
            BookSideKind::Bid,
            bid_prices
                .iter()
                .enumerate()
                .map(|(index, price)| level(price, index as u64 + 1))
                .collect(),
        )
        .unwrap(),
        BookSide::new(
            BookSideKind::Ask,
            ask_prices
                .iter()
                .enumerate()
                .map(|(index, price)| level(price, index as u64 + 1))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn trade(price: &str, quantity: u64, kind: TradeObservationKind) -> ObservedTrade {
    ObservedTrade::new(
        Price::parse(price).unwrap(),
        Quantity::new(quantity, QuantityUnit::TradingUnit).unwrap(),
        kind,
    )
}

fn quote_event(
    micros: i64,
    snapshot_book: CompleteBookSnapshot,
    observed_trade: Observation<ObservedTrade>,
    cumulative_volume: Observation<Volume>,
) -> DomainEvent {
    DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::from_unix_microseconds(micros),
        None,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                snapshot_book,
                observed_trade,
                cumulative_volume,
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0)),
            )
            .unwrap()
            .with_market_signal(Observation::Set(MarketSignal::Continuous)),
        ),
    )
}

fn stability_event(micros: i64) -> DomainEvent {
    DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::from_unix_microseconds(micros),
        None,
        EventPayload::IndicativeAuction(
            IndicativeAuction::new(
                AuctionObservation::volatility_interruption(VolatilityDirection::Up, false, false),
                Observation::Set(Price::parse("105").unwrap()),
                Observation::Set(Quantity::new(20, QuantityUnit::TradingUnit).unwrap()),
                Observation::Set(book(&["104"], &["106"])),
                Observation::Set(Volume::new(999, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0x80, 0x02)),
            )
            .unwrap(),
        ),
    )
}

fn paused_firm_quote_event(micros: i64) -> DomainEvent {
    DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::from_unix_microseconds(micros),
        None,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book(&["102"], &["103"]),
                Observation::NoObservation,
                Observation::Set(Volume::new(11, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0x01)),
            )
            .unwrap()
            .with_market_signal(Observation::Set(MarketSignal::AuctionCollecting(
                AuctionObservation::volatility_interruption(
                    VolatilityDirection::Down,
                    false,
                    false,
                ),
            ))),
        ),
    )
}

fn volatility_pause_status_event(micros: i64, volume: u64) -> DomainEvent {
    DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_REALTIME").unwrap(),
        MatchTime::from_unix_microseconds(micros),
        None,
        EventPayload::MarketStatus(
            MarketStatusObservation::new(
                Observation::Set(Volume::new(volume, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0x02)),
            )
            .with_market_signal(Observation::Set(MarketSignal::AuctionCollecting(
                AuctionObservation::volatility_interruption(
                    VolatilityDirection::Down,
                    false,
                    false,
                ),
            ))),
        ),
    )
}

#[test]
fn neutral_auction_quote_updates_signal_without_raw_flag_validation() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let context = context("regular", SegmentBoundaryPolicy::Carry);
    let firm = quote_event(
        1,
        book(&["100"], &["101"]),
        Observation::NoObservation,
        Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
    );
    reducer.apply(&mut state, &firm, &context).unwrap();
    let firm_book = state.book().clone();
    let trial_quote = DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::from_unix_microseconds(2),
        None,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book(&["104"], &["106"]),
                Observation::NoObservation,
                Observation::Set(Volume::new(999, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0x80, 0)),
            )
            .unwrap()
            .with_market_signal(Observation::Set(MarketSignal::AuctionCollecting(
                AuctionObservation::opening(false, false),
            ))),
        ),
    );
    reducer.apply(&mut state, &trial_quote, &context).unwrap();
    assert_eq!(state.state_version(), 2);
    assert_ne!(state.book(), &firm_book);
    assert_eq!(
        state.phase().known(),
        Some(&market_state::MarketPhase::Auction(
            market_state::AuctionState::new(AuctionObservation::opening(false, false)),
        ))
    );
    assert_eq!(state.cumulative_volume().known().unwrap().value(), 999);
}

fn reserved_firm_quote_event(micros: i64) -> DomainEvent {
    DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::from_unix_microseconds(micros),
        None,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book(&["102"], &["103"]),
                Observation::NoObservation,
                Observation::Set(Volume::new(11, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0x01, 0)),
            )
            .unwrap()
            .with_market_signal(Observation::Unknown(UnknownValue::Unsigned(0x0100))),
        ),
    )
}

#[test]
fn tpex_profile_accepts_tpex_annotations_and_replaces_the_book() {
    let instrument = InstrumentId::new(MarketId::Tpex, Symbol::new("6488").unwrap());
    let date = TradingDate::parse("2026-07-27").unwrap();
    let reducer = MarketStateReducer::tpex_regular();
    let context = ReducerContext::new(
        date,
        SessionSegmentId::new("regular").unwrap(),
        SegmentBoundaryPolicy::Carry,
        1,
    );
    let event = DomainEvent::new(
        instrument.clone(),
        date,
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap(),
        None,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book(&["100"], &["101"]),
                Observation::NoObservation,
                Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                MarketAnnotations::TpexQuote(TpexQuoteAnnotations::new(16, 0)),
            )
            .unwrap(),
        ),
    );
    let mut state = MarketState::new(instrument, date);
    reducer.apply(&mut state, &event, &context).unwrap();
    assert_eq!(state.state_version(), 1);
    assert_eq!(
        state.view().best_bid().unwrap().price(),
        Price::parse("100").unwrap()
    );
}

#[test]
fn tpex_warrant_profile_accepts_tpex_warrant_quote_events() {
    let instrument = InstrumentId::new(MarketId::Tpex, Symbol::new("6488").unwrap());
    let date = TradingDate::parse("2026-07-27").unwrap();
    let reducer = MarketStateReducer::tpex_warrant();
    let context = ReducerContext::new(
        date,
        SessionSegmentId::new("regular").unwrap(),
        SegmentBoundaryPolicy::Carry,
        1,
    );
    let event = DomainEvent::new(
        instrument.clone(),
        date,
        SourceFormatId::new("WARRANT_SNAPSHOT").unwrap(),
        MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap(),
        None,
        EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                book(&["100"], &["101"]),
                Observation::NoObservation,
                Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                MarketAnnotations::TpexQuote(TpexQuoteAnnotations::new(16, 0)),
            )
            .unwrap(),
        ),
    );
    let mut state = MarketState::new(instrument, date);
    reducer.apply(&mut state, &event, &context).unwrap();
    assert_eq!(state.state_version(), 1);
    assert_eq!(
        state.view().best_ask().unwrap().price(),
        Price::parse("101").unwrap()
    );
}

#[test]
fn initial_state_preserves_unavailable_semantics() {
    let state = MarketState::new(instrument("2330"), date());
    assert_eq!(state.state_version(), 0);
    assert_eq!(state.current_segment_id(), None);
    assert!(matches!(
        state.book(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
    assert!(matches!(
        state.recent_trade(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
    assert!(matches!(
        state.cumulative_volume(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
    assert!(state.last_event().is_none());
}

#[test]
fn quote_replaces_book_atomically_and_no_observation_preserves_trade_origin() {
    let reducer = MarketStateReducer::twse_regular();
    let context = context("regular", SegmentBoundaryPolicy::Carry);
    let mut state = MarketState::new(instrument("2330"), date());
    let first = quote_event(
        1,
        book(&["100", "99"], &["101", "102"]),
        Observation::Set(trade("100", 2, TradeObservationKind::Regular)),
        Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
    );
    let first_receipt = reducer.apply(&mut state, &first, &context).unwrap();

    assert_eq!(state.state_version(), 1);
    assert_eq!(first_receipt.previous_version(), 0);
    assert_eq!(first_receipt.new_version(), 1);
    let first_trade_origin = state.recent_trade().observed_at().unwrap().clone();
    assert_eq!(
        state.view().best_bid().unwrap().price(),
        Price::parse("100").unwrap()
    );
    assert!(matches!(state.view().last_trade(), LastTrade::Known(_)));

    let second = quote_event(
        2,
        book(&["98"], &["103"]),
        Observation::NoObservation,
        Observation::Set(Volume::new(11, QuantityUnit::TradingUnit)),
    );
    reducer.apply(&mut state, &second, &context).unwrap();

    assert_eq!(state.state_version(), 2);
    assert_eq!(state.book().known().unwrap().bids().levels().count(), 1);
    assert_eq!(
        state.view().best_bid().unwrap().price(),
        Price::parse("98").unwrap()
    );
    assert_eq!(
        state.recent_trade().observed_at(),
        Some(&first_trade_origin)
    );
    assert_eq!(state.cumulative_volume().known().unwrap().value(), 11);
}

#[test]
fn trade_batch_preserves_book_and_increments_version_once() {
    let reducer = MarketStateReducer::twse_regular();
    let context = context("regular", SegmentBoundaryPolicy::Carry);
    let mut state = MarketState::new(instrument("2330"), date());
    let quote = quote_event(
        1,
        book(&["100"], &["101"]),
        Observation::NoObservation,
        Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
    );
    reducer.apply(&mut state, &quote, &context).unwrap();
    let prior_book = state.book().clone();

    let batch = TradeBatch::new(
        vec![trade("100.5", 2, TradeObservationKind::Intermediate)],
        TradeBatchOrdering::SourceSequencePreserved,
        Observation::Set(Volume::new(12, QuantityUnit::TradingUnit)),
        MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0)),
    )
    .unwrap();
    let event = DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_REALTIME").unwrap(),
        MatchTime::from_unix_microseconds(2),
        None,
        EventPayload::TradeBatch(batch),
    );
    reducer.apply(&mut state, &event, &context).unwrap();

    assert_eq!(state.state_version(), 2);
    assert_eq!(state.book(), &prior_book);
    let LastTrade::Known(last_trade) = state.view().last_trade() else {
        panic!("source-ordered batch must expose its last trade")
    };
    assert_eq!(last_trade.price(), Price::parse("100.5").unwrap());
}

#[test]
fn status_only_pause_preserves_firm_book_and_trade() {
    let reducer = MarketStateReducer::twse_regular();
    let context = context("regular", SegmentBoundaryPolicy::Carry);
    let mut state = MarketState::new(instrument("2330"), date());
    let quote = quote_event(
        1,
        book(&["100"], &["101"]),
        Observation::Set(trade("100", 2, TradeObservationKind::Regular)),
        Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
    );
    reducer.apply(&mut state, &quote, &context).unwrap();
    let prior_book = state.book().clone();
    let prior_trade = state.recent_trade().clone();

    let receipt = reducer
        .apply(&mut state, &volatility_pause_status_event(2, 10), &context)
        .unwrap();

    assert_eq!(state.book(), &prior_book);
    assert_eq!(state.recent_trade(), &prior_trade);
    assert_eq!(state.cumulative_volume().known().unwrap().value(), 10);
    assert!(
        !receipt
            .changed_fields()
            .contains(&market_state::ChangedField::Book)
    );
    assert!(
        !receipt
            .changed_fields()
            .contains(&market_state::ChangedField::RecentTrade)
    );
    assert!(
        receipt
            .changed_fields()
            .contains(&market_state::ChangedField::MarketSignal)
    );
}

#[test]
fn clear_unknown_and_volume_regression_have_explicit_atomic_behavior() {
    let reducer = MarketStateReducer::twse_regular();
    let context = context("regular", SegmentBoundaryPolicy::Carry);
    let mut state = MarketState::new(instrument("2330"), date());
    let first = quote_event(
        1,
        book(&["100"], &["101"]),
        Observation::Set(trade("100", 1, TradeObservationKind::Regular)),
        Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
    );
    reducer.apply(&mut state, &first, &context).unwrap();

    let unknown = quote_event(
        2,
        book(&["100"], &["101"]),
        Observation::Unknown(UnknownValue::Unsigned(7)),
        Observation::Clear,
    );
    let receipt = reducer.apply(&mut state, &unknown, &context).unwrap();
    assert_eq!(
        receipt.warning_codes(),
        &[StateWarningCode::UnknownRecentTrade]
    );
    assert!(matches!(
        state.recent_trade(),
        StateField::Unknown {
            raw: UnknownValue::Unsigned(7),
            ..
        }
    ));
    assert!(matches!(
        state.cumulative_volume(),
        StateField::Unavailable(UnavailableReason::Cleared { .. })
    ));

    let restored = quote_event(
        3,
        book(&["100"], &["101"]),
        Observation::NoObservation,
        Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
    );
    reducer.apply(&mut state, &restored, &context).unwrap();
    let before_error = state.clone();
    let regression = quote_event(
        4,
        book(&["99"], &["102"]),
        Observation::Set(trade("99", 1, TradeObservationKind::Regular)),
        Observation::Set(Volume::new(9, QuantityUnit::TradingUnit)),
    );
    let error = reducer
        .apply(&mut state, &regression, &context)
        .unwrap_err();
    assert!(matches!(
        error,
        StateTransitionError::CumulativeVolumeRegression {
            previous: 10,
            next: 9
        }
    ));
    assert_eq!(state, before_error);
}

#[test]
fn segment_reset_is_committed_with_the_first_new_segment_event() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    reducer
        .apply(
            &mut state,
            &quote_event(
                1,
                book(&["100"], &["101"]),
                Observation::Set(trade("100", 1, TradeObservationKind::Regular)),
                Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
            ),
            &context("first", SegmentBoundaryPolicy::ResetObservableFields),
        )
        .unwrap();

    reducer
        .apply(
            &mut state,
            &quote_event(
                2,
                book(&["90"], &["91"]),
                Observation::NoObservation,
                Observation::NoObservation,
            ),
            &context("second", SegmentBoundaryPolicy::ResetObservableFields),
        )
        .unwrap();
    assert_eq!(state.current_segment_id().unwrap().as_str(), "second");
    assert!(state.book().known().is_some());
    assert!(matches!(
        state.recent_trade(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
    assert!(matches!(
        state.cumulative_volume(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
}

#[test]
fn book_snapshot_preserves_trade_and_volume_under_an_explicit_profile() {
    let profile = MarketStateProfile::new(
        MarketId::Twse,
        vec![
            SourceFormatRule::new(
                SourceFormatId::new("BOOK").unwrap(),
                vec![EventKind::BookSnapshot],
            )
            .unwrap(),
        ],
        CumulativeVolumePolicy::Unconstrained,
        1,
    )
    .unwrap();
    let reducer = MarketStateReducer::new(profile);
    let mut state = MarketState::new(instrument("2330"), date());
    let event = DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("BOOK").unwrap(),
        MatchTime::from_unix_microseconds(1),
        None,
        EventPayload::BookSnapshot(BookSnapshot::new(
            book(&["100"], &["101"]),
            MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0, 0)),
        )),
    );
    reducer
        .apply(
            &mut state,
            &event,
            &context("regular", SegmentBoundaryPolicy::Carry),
        )
        .unwrap();
    assert!(state.book().known().is_some());
    assert!(matches!(
        state.recent_trade(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
    assert!(matches!(
        state.cumulative_volume(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
}

#[test]
fn taifex_opening_indicative_is_timeline_without_trade_or_volume_state() {
    let instrument = InstrumentId::new(MarketId::Taifex, Symbol::new("TXFH6").unwrap());
    let trading_date = TradingDate::parse("2026-07-20").unwrap();
    let event = DomainEvent::new(
        instrument.clone(),
        trading_date,
        SourceFormatId::new("I022").unwrap(),
        MatchTime::parse("2026-07-20T08:40:00+08:00").unwrap(),
        None,
        EventPayload::IndicativeAuction(
            IndicativeAuction::new(
                AuctionObservation::opening(false, false),
                Observation::Set(Price::parse("43500").unwrap()),
                Observation::Set(Quantity::new(3, QuantityUnit::Contract).unwrap()),
                Observation::NoObservation,
                Observation::NoObservation,
                MarketAnnotations::None,
            )
            .unwrap(),
        ),
    );
    let reducer = MarketStateReducer::taifex_futures();
    let mut state = MarketState::new(instrument, trading_date);
    let context = ReducerContext::new(
        trading_date,
        SessionSegmentId::new("regular").unwrap(),
        SegmentBoundaryPolicy::Carry,
        1,
    );

    let receipt = reducer.apply(&mut state, &event, &context).unwrap();

    assert_eq!(receipt.new_version(), 1);
    assert!(matches!(
        state.book(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
    assert!(matches!(
        state.recent_trade(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
    assert!(matches!(
        state.cumulative_volume(),
        StateField::Unavailable(UnavailableReason::Initial)
    ));
    assert!(
        matches!(state.indicative_auction(), StateField::Known { value, .. }
        if value.observation() == AuctionObservation::opening(false, false))
    );
    assert!(state.last_event().is_some());
}

#[test]
fn intraday_stability_updates_only_indicative_state() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let context = context("regular", SegmentBoundaryPolicy::Carry);
    let firm = quote_event(
        1,
        book(&["100"], &["101"]),
        Observation::Set(trade("100.5", 2, TradeObservationKind::Regular)),
        Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
    );
    reducer.apply(&mut state, &firm, &context).unwrap();
    let firm_book = state.book().clone();
    let firm_trade = state.recent_trade().clone();
    let firm_volume = state.cumulative_volume().clone();

    let trial = DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::from_unix_microseconds(2),
        None,
        EventPayload::IndicativeAuction(
            IndicativeAuction::new(
                AuctionObservation::volatility_interruption(VolatilityDirection::Up, false, false),
                Observation::Set(Price::parse("105").unwrap()),
                Observation::Set(Quantity::new(20, QuantityUnit::TradingUnit).unwrap()),
                Observation::Set(book(&["104"], &["106"])),
                Observation::Set(Volume::new(999, QuantityUnit::TradingUnit)),
                MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0x80, 0x02)),
            )
            .unwrap(),
        ),
    );
    let receipt = reducer.apply(&mut state, &trial, &context).unwrap();

    assert_eq!(state.book(), &firm_book);
    assert_eq!(state.recent_trade(), &firm_trade);
    assert_eq!(state.cumulative_volume(), &firm_volume);
    assert!(
        matches!(state.indicative_auction(), StateField::Known { value, .. }
        if value.observation().purpose()
                == AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
            && value.observation().direction() == AuctionEvidence::Known(VolatilityDirection::Up))
    );
    assert_eq!(
        receipt.changed_fields(),
        &[
            market_state::ChangedField::IndicativeAuction,
            market_state::ChangedField::MarketSignal,
            market_state::ChangedField::Phase,
            market_state::ChangedField::LastEvent,
        ]
    );
}

#[test]
fn continuous_resume_clears_indicative_state_without_mutating_firm_state() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let context = context("regular", SegmentBoundaryPolicy::Carry);
    let initial_firm = quote_event(
        1,
        book(&["100"], &["101"]),
        Observation::Set(trade("100", 2, TradeObservationKind::Regular)),
        Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
    );
    reducer.apply(&mut state, &initial_firm, &context).unwrap();
    reducer
        .apply(&mut state, &stability_event(2), &context)
        .unwrap();

    // A non-trial event that still carries the verified interruption trend is
    // not evidence that continuous matching has resumed.
    reducer
        .apply(&mut state, &paused_firm_quote_event(3), &context)
        .unwrap();
    assert!(matches!(
        state.indicative_auction(),
        StateField::Known { value, .. }
            if value.observation().purpose()
                == AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
    ));
    reducer
        .apply(&mut state, &reserved_firm_quote_event(4), &context)
        .unwrap();
    assert!(matches!(
        state.indicative_auction(),
        StateField::Known { value, .. }
            if value.observation().purpose()
                == AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
    ));

    let resumed = quote_event(
        5,
        book(&["103"], &["104"]),
        Observation::NoObservation,
        Observation::Set(Volume::new(12, QuantityUnit::TradingUnit)),
    );
    let receipt = reducer.apply(&mut state, &resumed, &context).unwrap();

    assert!(matches!(
        state.indicative_auction(),
        StateField::Unavailable(UnavailableReason::Cleared { cleared_at })
            if cleared_at.match_time() == MatchTime::from_unix_microseconds(5)
    ));
    assert_eq!(
        state.view().best_bid().unwrap().price(),
        Price::parse("103").unwrap()
    );
    assert_eq!(state.cumulative_volume().known().unwrap().value(), 12);
    assert!(
        receipt
            .changed_fields()
            .contains(&market_state::ChangedField::IndicativeAuction)
    );
}

#[test]
fn market_status_does_not_clear_indicative_state_even_with_normal_flags() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let context = context("regular", SegmentBoundaryPolicy::Carry);
    reducer
        .apply(
            &mut state,
            &quote_event(
                1,
                book(&["100"], &["101"]),
                Observation::NoObservation,
                Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
            ),
            &context,
        )
        .unwrap();
    reducer
        .apply(&mut state, &stability_event(2), &context)
        .unwrap();
    let status = DomainEvent::new(
        instrument("2330"),
        date(),
        SourceFormatId::new("STOCK_REALTIME").unwrap(),
        MatchTime::from_unix_microseconds(3),
        None,
        EventPayload::MarketStatus(MarketStatusObservation::new(
            Observation::Set(Volume::new(11, QuantityUnit::TradingUnit)),
            MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(16, 0)),
        )),
    );

    reducer.apply(&mut state, &status, &context).unwrap();

    assert!(matches!(
        state.indicative_auction(),
        StateField::Known { value, .. }
            if value.observation().purpose()
                == AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
    ));
    assert_eq!(state.cumulative_volume().known().unwrap().value(), 11);
}

#[test]
fn carry_session_boundary_clears_indicative_even_if_matching_stays_paused() {
    let reducer = MarketStateReducer::twse_regular();
    let mut state = MarketState::new(instrument("2330"), date());
    let regular = context("regular", SegmentBoundaryPolicy::Carry);
    let next_segment = context("next", SegmentBoundaryPolicy::Carry);
    reducer
        .apply(
            &mut state,
            &quote_event(
                1,
                book(&["100"], &["101"]),
                Observation::NoObservation,
                Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
            ),
            &regular,
        )
        .unwrap();
    reducer
        .apply(&mut state, &stability_event(2), &regular)
        .unwrap();

    let boundary_event = paused_firm_quote_event(3);
    let receipt = reducer
        .apply(&mut state, &boundary_event, &next_segment)
        .unwrap();

    assert!(matches!(
        state.indicative_auction(),
        StateField::Unavailable(UnavailableReason::Cleared { cleared_at })
            if cleared_at.match_time() == MatchTime::from_unix_microseconds(3)
    ));
    assert_eq!(
        state.view().best_bid().unwrap().price(),
        Price::parse("102").unwrap()
    );
    assert!(
        receipt
            .changed_fields()
            .contains(&market_state::ChangedField::IndicativeAuction)
    );
}

#[test]
fn tpex_normal_matching_event_clears_prior_indicative_observation() {
    let instrument = InstrumentId::new(MarketId::Tpex, Symbol::new("6488").unwrap());
    let reducer = MarketStateReducer::tpex_regular();
    let context = ReducerContext::new(
        date(),
        SessionSegmentId::new("regular").unwrap(),
        SegmentBoundaryPolicy::Carry,
        1,
    );
    let quote = |micros, status_flags, limit_flags| {
        DomainEvent::new(
            instrument.clone(),
            date(),
            SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
            MatchTime::from_unix_microseconds(micros),
            None,
            EventPayload::QuoteSnapshot(
                QuoteSnapshot::new(
                    book(&["100"], &["101"]),
                    Observation::NoObservation,
                    Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
                    MarketAnnotations::TpexQuote(TpexQuoteAnnotations::new(
                        status_flags,
                        limit_flags,
                    )),
                )
                .unwrap()
                .with_market_signal(Observation::Set(MarketSignal::Continuous)),
            ),
        )
    };
    let mut state = MarketState::new(instrument.clone(), date());
    reducer
        .apply(&mut state, &quote(1, 16, 0), &context)
        .unwrap();
    let trial = DomainEvent::new(
        instrument.clone(),
        date(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::from_unix_microseconds(2),
        None,
        EventPayload::IndicativeAuction(
            IndicativeAuction::new(
                AuctionObservation::volatility_interruption(
                    VolatilityDirection::Down,
                    false,
                    false,
                ),
                Observation::Set(Price::parse("95").unwrap()),
                Observation::Set(Quantity::new(5, QuantityUnit::TradingUnit).unwrap()),
                Observation::NoObservation,
                Observation::Set(Volume::new(10, QuantityUnit::TradingUnit)),
                MarketAnnotations::TpexQuote(TpexQuoteAnnotations::new(0x80, 0x01)),
            )
            .unwrap(),
        ),
    );
    reducer.apply(&mut state, &trial, &context).unwrap();
    reducer
        .apply(&mut state, &quote(3, 16, 0), &context)
        .unwrap();

    assert!(matches!(
        state.indicative_auction(),
        StateField::Unavailable(UnavailableReason::Cleared { cleared_at })
            if cleared_at.match_time() == MatchTime::from_unix_microseconds(3)
    ));
}

#[test]
fn canonical_state_set_uses_instrument_order_not_input_order() {
    let left = MarketState::new(instrument("1101"), date());
    let right = MarketState::new(instrument("2330"), date());
    let forward = canonical_final_state_set([&left, &right]).unwrap();
    let reverse = canonical_final_state_set([&right, &left]).unwrap();
    assert_eq!(forward, reverse);
    assert_eq!(
        final_state_checksum([&left, &right]).unwrap(),
        final_state_checksum([&right, &left]).unwrap()
    );
    assert!(left.to_canonical_bytes().unwrap().starts_with(b"OSMS"));
}

#[test]
fn initial_market_state_canonical_frame_matches_the_documented_layout() {
    let state = MarketState::new(instrument("2330"), date());
    let mut expected = Vec::new();
    expected.extend_from_slice(b"OSMS");
    expected.extend_from_slice(&market_state::CANONICAL_MARKET_STATE_VERSION.to_be_bytes());
    expected.extend_from_slice(&market_state::MARKET_STATE_VERSION.to_be_bytes());
    expected.push(MarketId::Twse.discriminant());
    expected.extend_from_slice(&4_u32.to_be_bytes());
    expected.extend_from_slice(b"2330");
    expected.extend_from_slice(&date().to_canonical_bytes());
    expected.push(0);
    expected.extend_from_slice(&0_u64.to_be_bytes());
    expected.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    expected.push(0);

    let canonical = state.to_canonical_bytes().unwrap();
    assert_eq!(canonical, expected);
    assert_eq!(
        state.fingerprint().unwrap().as_bytes(),
        blake3::hash(&canonical).as_bytes()
    );
}
