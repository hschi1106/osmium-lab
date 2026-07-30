use market_types::{
    BookError, BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent, EventError,
    EventPayload, InstantTrend, InstrumentId, LimitPosition, MarketAnnotations, MarketId,
    MatchTime, MatchingMethod, Observation, Price, Quantity, QuantityUnit, QuoteSnapshot,
    SourceFormatId, Symbol, TradeBatch, TradeOrder, TradePrint, TradePrintKind, TradingDate,
    TwseQuoteAnnotations, Volume,
};

fn level(price: &str, quantity: u64, unit: QuantityUnit) -> BookLevel {
    BookLevel::new(
        Price::parse(price).unwrap(),
        Quantity::new(quantity, unit).unwrap(),
    )
}

fn empty_book() -> CompleteBookSnapshot {
    CompleteBookSnapshot::new(
        BookSide::new(BookSideKind::Bid, vec![]).unwrap(),
        BookSide::new(BookSideKind::Ask, vec![]).unwrap(),
    )
    .unwrap()
}

#[test]
fn book_side_enforces_contiguity_price_order_and_units() {
    let bids = BookSide::new(
        BookSideKind::Bid,
        vec![
            level("100", 2, QuantityUnit::TradingUnit),
            level("99.5", 3, QuantityUnit::TradingUnit),
        ],
    )
    .unwrap();
    assert_eq!(bids.levels().count(), 2);

    let invalid_order = BookSide::new(
        BookSideKind::Bid,
        vec![
            level("100", 2, QuantityUnit::TradingUnit),
            level("100", 3, QuantityUnit::TradingUnit),
        ],
    );
    assert!(matches!(
        invalid_order,
        Err(BookError::PriceOrder {
            side: BookSideKind::Bid,
            index: 1
        })
    ));

    let invalid_units = BookSide::new(
        BookSideKind::Ask,
        vec![
            level("100", 2, QuantityUnit::TradingUnit),
            level("101", 3, QuantityUnit::Share),
        ],
    );
    assert!(matches!(
        invalid_units,
        Err(BookError::UnitMismatch { index: 1, .. })
    ));

    let non_contiguous = BookSide::from_slots(
        BookSideKind::Ask,
        [
            Some(level("100", 2, QuantityUnit::TradingUnit)),
            None,
            Some(level("101", 3, QuantityUnit::TradingUnit)),
            None,
            None,
        ],
    );
    assert!(matches!(
        non_contiguous,
        Err(BookError::NonContiguous { index: 2, .. })
    ));
}

#[test]
fn quote_and_trade_batch_reject_mixed_quantity_units() {
    let trade = TradePrint::new(
        Price::parse("100").unwrap(),
        Quantity::new(1, QuantityUnit::Share).unwrap(),
        TradePrintKind::Regular,
    );
    let quote = QuoteSnapshot::new(
        CompleteBookSnapshot::new(
            BookSide::new(
                BookSideKind::Bid,
                vec![level("99", 1, QuantityUnit::TradingUnit)],
            )
            .unwrap(),
            BookSide::new(
                BookSideKind::Ask,
                vec![level("101", 1, QuantityUnit::TradingUnit)],
            )
            .unwrap(),
        )
        .unwrap(),
        Observation::Set(trade),
        Observation::Set(Volume::new(1, QuantityUnit::Share)),
        MarketAnnotations::None,
    );
    assert!(matches!(
        quote,
        Err(EventError::QuantityUnitMismatch { .. })
    ));

    let batch = TradeBatch::new(
        vec![trade],
        TradeOrder::Unspecified,
        Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
        MarketAnnotations::None,
    );
    assert!(matches!(
        batch,
        Err(EventError::QuantityUnitMismatch { .. })
    ));
}

#[test]
fn twse_annotation_typed_views_preserve_raw_semantics() {
    let annotations = TwseQuoteAnnotations::new(0xF8, 0b01_10_00_10);
    assert_eq!(annotations.status_flags_raw(), 0xF8);
    assert!(annotations.status().trial());
    assert!(annotations.status().delayed_open());
    assert!(annotations.status().delayed_close());
    assert_eq!(
        annotations.status().matching_method(),
        MatchingMethod::Continuous
    );
    assert!(annotations.status().opening_marker());
    assert!(!annotations.status().closing_marker());
    assert_eq!(annotations.status().reserved_bits(), 0);

    assert_eq!(annotations.limits().trade(), LimitPosition::LowerLimit);
    assert_eq!(annotations.limits().best_bid(), LimitPosition::UpperLimit);
    assert_eq!(annotations.limits().best_ask(), LimitPosition::Normal);
    assert_eq!(
        annotations.limits().instant_trend(),
        InstantTrend::VolatilityInterruptionUp
    );
}

#[test]
fn canonical_quote_frame_has_the_documented_field_order() {
    let snapshot = QuoteSnapshot::new(
        empty_book(),
        Observation::NoObservation,
        Observation::Set(Volume::new(0, QuantityUnit::SourceUnit)),
        MarketAnnotations::None,
    )
    .unwrap();
    let event = DomainEvent::new(
        InstrumentId::new(MarketId::Twse, Symbol::new("A").unwrap()),
        TradingDate::from_epoch_days(0),
        SourceFormatId::new("X").unwrap(),
        MatchTime::from_unix_microseconds(1),
        None,
        EventPayload::QuoteSnapshot(snapshot),
    );

    let mut expected = Vec::new();
    expected.extend_from_slice(b"OSME");
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.extend_from_slice(&1_u16.to_be_bytes());
    expected.push(1);
    expected.extend_from_slice(&1_u32.to_be_bytes());
    expected.push(b'A');
    expected.extend_from_slice(&0_i32.to_be_bytes());
    expected.extend_from_slice(&1_u32.to_be_bytes());
    expected.push(b'X');
    expected.extend_from_slice(&1_i64.to_be_bytes());
    expected.push(0);
    expected.push(10);
    expected.extend_from_slice(&[0; 10]);
    expected.push(0);
    expected.push(1);
    expected.push(0);
    expected.extend_from_slice(&0_u64.to_be_bytes());
    expected.push(0);

    let canonical = event.to_canonical_bytes().unwrap();
    assert_eq!(canonical, expected);
    assert_eq!(
        event.fingerprint().unwrap().as_bytes(),
        blake3::hash(&canonical).as_bytes()
    );
}

#[test]
fn canonical_event_changes_for_distinct_observation_semantics() {
    let make_event = |trade| {
        let snapshot = QuoteSnapshot::new(
            empty_book(),
            trade,
            Observation::Set(Volume::new(0, QuantityUnit::SourceUnit)),
            MarketAnnotations::None,
        )
        .unwrap();
        DomainEvent::new(
            InstrumentId::new(MarketId::Twse, Symbol::new("A").unwrap()),
            TradingDate::from_epoch_days(0),
            SourceFormatId::new("X").unwrap(),
            MatchTime::from_unix_microseconds(1),
            None,
            EventPayload::QuoteSnapshot(snapshot),
        )
        .to_canonical_bytes()
        .unwrap()
    };

    assert_ne!(
        make_event(Observation::NoObservation),
        make_event(Observation::Clear)
    );
}
