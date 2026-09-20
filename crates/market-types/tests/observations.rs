use market_types::{
    BookSide, BookSideKind, CompleteBookSnapshot, EventError, MarketAnnotations, Observation,
    ObservedTrade, Price, Quantity, QuantityUnit, QuoteSnapshot, TradeBatch, TradeBatchOrdering,
    TradeObservationKind, Volume,
};

#[test]
fn observed_trade_renames_preserve_canonical_discriminants() {
    assert_eq!(TradeObservationKind::Regular.discriminant(), 0);
    assert_eq!(TradeObservationKind::Intermediate.discriminant(), 1);
    assert_eq!(TradeBatchOrdering::Unspecified.discriminant(), 0);
    assert_eq!(
        TradeBatchOrdering::SourceSequencePreserved.discriminant(),
        1
    );
}

#[test]
fn quote_and_trade_batch_reject_mixed_quantity_units() {
    let trade = ObservedTrade::new(
        Price::parse("100").unwrap(),
        Quantity::new(1, QuantityUnit::Share).unwrap(),
        TradeObservationKind::Regular,
    );
    let quote = QuoteSnapshot::new(
        market_types::CompleteBookSnapshot::new(
            market_types::BookSide::new(
                market_types::BookSideKind::Bid,
                vec![market_types::BookLevel::new(
                    Price::parse("99").unwrap(),
                    Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
                )],
            )
            .unwrap(),
            market_types::BookSide::new(
                market_types::BookSideKind::Ask,
                vec![market_types::BookLevel::new(
                    Price::parse("101").unwrap(),
                    Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
                )],
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
        TradeBatchOrdering::Unspecified,
        Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
        MarketAnnotations::None,
    );
    assert!(matches!(
        batch,
        Err(EventError::QuantityUnitMismatch { .. })
    ));
}

#[test]
fn generic_firm_payloads_do_not_assume_continuous_without_provider_evidence() {
    let book = CompleteBookSnapshot::new(
        BookSide::new(BookSideKind::Bid, Vec::new()).unwrap(),
        BookSide::new(BookSideKind::Ask, Vec::new()).unwrap(),
    )
    .unwrap();
    let quote = QuoteSnapshot::new(
        book.clone(),
        Observation::NoObservation,
        Observation::NoObservation,
        MarketAnnotations::None,
    )
    .unwrap();
    assert_eq!(quote.market_signal(), &Observation::NoObservation);

    let snapshot = market_types::BookSnapshot::new(book, MarketAnnotations::None);
    assert_eq!(snapshot.market_signal(), &Observation::NoObservation);

    let trade = ObservedTrade::new(
        Price::parse("100").unwrap(),
        Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
        TradeObservationKind::Regular,
    );
    let batch = TradeBatch::new(
        vec![trade],
        TradeBatchOrdering::Unspecified,
        Observation::NoObservation,
        MarketAnnotations::None,
    )
    .unwrap();
    assert_eq!(batch.market_signal(), &Observation::NoObservation);
}
