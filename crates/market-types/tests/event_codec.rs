#[path = "support/event.rs"]
mod support;

use market_types::{
    AuctionEvidence, AuctionObservation, AuctionPurpose, BookLevel, BookSide, BookSideKind,
    CANONICAL_EVENT_VERSION, CompleteBookSnapshot, DomainEvent, EVENT_SCHEMA_VERSION, EventPayload,
    IndicativeAuction, InstrumentId, MarketAnnotations, MarketId, MarketStatusObservation,
    MatchTime, Observation, ObservedTrade, Price, Quantity, QuantityUnit, QuoteSnapshot,
    SourceFormatId, Symbol, TpexQuoteAnnotations, TradeBatch, TradeBatchOrdering,
    TradeObservationKind, TradingDate, VolatilityDirection, Volume,
};
use support::empty_book;

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
        TradingDate::from_epoch_days(0).unwrap(),
        SourceFormatId::new("X").unwrap(),
        MatchTime::from_unix_microseconds(1),
        None,
        EventPayload::QuoteSnapshot(snapshot),
    );

    let mut expected = Vec::new();
    expected.extend_from_slice(b"OSME");
    expected.extend_from_slice(&CANONICAL_EVENT_VERSION.to_be_bytes());
    expected.extend_from_slice(&EVENT_SCHEMA_VERSION.to_be_bytes());
    expected.push(1);
    expected.extend_from_slice(&1_u32.to_be_bytes());
    expected.push(b'A');
    expected.extend_from_slice(&0_i32.to_be_bytes());
    expected.extend_from_slice(&1_u32.to_be_bytes());
    expected.push(b'X');
    expected.extend_from_slice(&1_i64.to_be_bytes());
    expected.push(0);
    expected.push(10);
    expected.extend_from_slice(&[0; 12]);
    expected.push(0);
    expected.push(1);
    expected.push(0);
    expected.extend_from_slice(&0_u64.to_be_bytes());
    expected.push(0);
    expected.push(0);

    let canonical = event.to_canonical_bytes().unwrap();
    assert_eq!(canonical, expected);
    assert_eq!(
        event.fingerprint().unwrap().as_bytes(),
        blake3::hash(&canonical).as_bytes()
    );
}

#[test]
fn canonical_event_rejects_unrenderable_match_time_on_encode_and_decode() {
    let make_event = |match_time| {
        DomainEvent::new(
            InstrumentId::new(MarketId::Twse, Symbol::new("A").unwrap()),
            TradingDate::from_epoch_days(0).unwrap(),
            SourceFormatId::new("X").unwrap(),
            match_time,
            None,
            EventPayload::BookSnapshot(market_types::BookSnapshot::new(
                empty_book(),
                MarketAnnotations::None,
            )),
        )
    };
    let canonical = make_event(MatchTime::from_unix_microseconds(1))
        .to_canonical_bytes()
        .unwrap();

    // Fixed event header, one-byte symbol, date, and one-byte source format precede match_time.
    const MATCH_TIME_OFFSET: usize = 23;
    for micros in [i64::MIN, i64::MAX] {
        assert_eq!(
            make_event(MatchTime::from_unix_microseconds(micros)).to_canonical_bytes(),
            Err(market_types::CanonicalEncodingError::InvalidMatchTime)
        );

        let mut invalid_frame = canonical.clone();
        invalid_frame[MATCH_TIME_OFFSET..MATCH_TIME_OFFSET + 8]
            .copy_from_slice(&micros.to_be_bytes());
        assert_eq!(
            DomainEvent::from_canonical_bytes(&invalid_frame),
            Err(market_types::CanonicalDecodingError::InvalidValue)
        );
    }
}

#[test]
fn canonical_event_decoder_rejects_trading_dates_outside_four_digit_years() {
    let event = DomainEvent::new(
        InstrumentId::new(MarketId::Twse, Symbol::new("A").unwrap()),
        TradingDate::from_epoch_days(0).unwrap(),
        SourceFormatId::new("X").unwrap(),
        MatchTime::from_unix_microseconds(1),
        None,
        EventPayload::BookSnapshot(market_types::BookSnapshot::new(
            empty_book(),
            MarketAnnotations::None,
        )),
    );
    let canonical = event.to_canonical_bytes().unwrap();

    // Fixed event header, one-byte symbol, then the four-byte epoch-day value.
    const TRADING_DATE_OFFSET: usize = 14;
    for epoch_days in [
        TradingDate::MIN_EPOCH_DAYS - 1,
        TradingDate::MAX_EPOCH_DAYS + 1,
        i32::MIN,
        i32::MAX,
    ] {
        let mut invalid_frame = canonical.clone();
        invalid_frame[TRADING_DATE_OFFSET..TRADING_DATE_OFFSET + 4]
            .copy_from_slice(&epoch_days.to_be_bytes());
        assert_eq!(
            DomainEvent::from_canonical_bytes(&invalid_frame),
            Err(market_types::CanonicalDecodingError::InvalidValue),
            "epoch days: {epoch_days}"
        );
    }
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
            TradingDate::from_epoch_days(0).unwrap(),
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

#[test]
fn auction_evidence_roundtrips_without_collapsing_missing_or_unknown_fields() {
    let observation = AuctionObservation::from_parts(
        AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption),
        AuctionEvidence::NoObservation,
        AuctionEvidence::Unknown,
        AuctionEvidence::Known(VolatilityDirection::Up),
    )
    .unwrap();
    let auction = IndicativeAuction::new(
        observation,
        Observation::NoObservation,
        Observation::NoObservation,
        Observation::NoObservation,
        Observation::NoObservation,
        MarketAnnotations::None,
    )
    .unwrap();
    let event = DomainEvent::new(
        InstrumentId::new(MarketId::Twse, Symbol::new("A").unwrap()),
        TradingDate::from_epoch_days(0).unwrap(),
        SourceFormatId::new("X").unwrap(),
        MatchTime::from_unix_microseconds(1),
        None,
        EventPayload::IndicativeAuction(auction),
    );

    let decoded = DomainEvent::from_canonical_bytes(&event.to_canonical_bytes().unwrap()).unwrap();
    assert_eq!(decoded, event);
    let EventPayload::IndicativeAuction(decoded_auction) = decoded.payload() else {
        panic!("expected indicative auction")
    };
    assert_eq!(
        decoded_auction.observation().delayed(),
        AuctionEvidence::NoObservation
    );
    assert_eq!(
        decoded_auction.observation().disposal(),
        AuctionEvidence::Unknown
    );
    assert!(
        AuctionObservation::from_parts(
            AuctionEvidence::Known(AuctionPurpose::Opening),
            AuctionEvidence::NoObservation,
            AuctionEvidence::NoObservation,
            AuctionEvidence::Known(VolatilityDirection::Up),
        )
        .is_none()
    );
}

#[test]
fn market_order_quantity_roundtrips_separately_from_priced_levels() {
    let market_quantity = Quantity::new(7, QuantityUnit::TradingUnit).unwrap();
    let book = CompleteBookSnapshot::new(
        BookSide::with_market_order_quantity(
            BookSideKind::Bid,
            Some(market_quantity),
            vec![BookLevel::new(
                Price::parse("100").unwrap(),
                Quantity::new(2, QuantityUnit::TradingUnit).unwrap(),
            )],
        )
        .unwrap(),
        BookSide::new(BookSideKind::Ask, vec![]).unwrap(),
    )
    .unwrap();
    let event = DomainEvent::new(
        InstrumentId::new(MarketId::Twse, Symbol::new("A").unwrap()),
        TradingDate::from_epoch_days(0).unwrap(),
        SourceFormatId::new("X").unwrap(),
        MatchTime::from_unix_microseconds(1),
        None,
        EventPayload::BookSnapshot(market_types::BookSnapshot::new(
            book,
            MarketAnnotations::None,
        )),
    );
    let decoded = DomainEvent::from_canonical_bytes(&event.to_canonical_bytes().unwrap()).unwrap();
    assert_eq!(decoded, event);
    let EventPayload::BookSnapshot(snapshot) = decoded.payload() else {
        panic!("expected book snapshot")
    };
    assert_eq!(
        snapshot.book().bids().market_order_quantity(),
        Some(market_quantity)
    );
    assert_eq!(snapshot.book().bids().levels().count(), 1);
}

#[test]
fn tpex_quote_annotations_roundtrip_as_market_specific_event_data() {
    let annotations = TpexQuoteAnnotations::new(0x84, 0b00_00_00_01);
    let snapshot = QuoteSnapshot::new(
        empty_book(),
        Observation::NoObservation,
        Observation::Set(Volume::new(0, QuantityUnit::TradingUnit)),
        MarketAnnotations::TpexQuote(annotations),
    )
    .unwrap();
    let event = DomainEvent::new(
        InstrumentId::new(MarketId::Tpex, Symbol::new("6488").unwrap()),
        TradingDate::parse("2026-07-20").unwrap(),
        SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
        MatchTime::parse("2026-07-20T13:30:00+08:00").unwrap(),
        None,
        EventPayload::QuoteSnapshot(snapshot),
    );
    let canonical = event.to_canonical_bytes().unwrap();
    assert_eq!(
        DomainEvent::from_canonical_bytes(&canonical).unwrap(),
        event
    );
    let EventPayload::QuoteSnapshot(snapshot) = event.payload() else {
        unreachable!()
    };
    let MarketAnnotations::TpexQuote(decoded) = snapshot.annotations() else {
        panic!("TPEx event must preserve the TPEx-specific annotation variant")
    };
    assert_eq!(decoded, &annotations);
}

#[test]
fn canonical_trade_batch_rejects_count_larger_than_the_frame() {
    let trade = ObservedTrade::new(
        Price::parse("100").unwrap(),
        Quantity::new(1, QuantityUnit::Contract).unwrap(),
        TradeObservationKind::Regular,
    );
    let batch = TradeBatch::new(
        vec![trade],
        TradeBatchOrdering::SourceSequencePreserved,
        Observation::NoObservation,
        MarketAnnotations::None,
    )
    .unwrap();
    let event = DomainEvent::new(
        InstrumentId::new(MarketId::Taifex, Symbol::new("A").unwrap()),
        TradingDate::from_epoch_days(0).unwrap(),
        SourceFormatId::new("X").unwrap(),
        MatchTime::from_unix_microseconds(1),
        None,
        EventPayload::TradeBatch(batch),
    );
    let canonical = event.to_canonical_bytes().unwrap();
    let count_offset = 33;

    let mut oversized = canonical.clone();
    oversized[count_offset..count_offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        DomainEvent::from_canonical_bytes(&oversized),
        Err(market_types::CanonicalDecodingError::LengthOutOfBounds)
    );

    let mut truncated_vector = canonical;
    truncated_vector[count_offset..count_offset + 4].copy_from_slice(&2_u32.to_be_bytes());
    assert_eq!(
        DomainEvent::from_canonical_bytes(&truncated_vector),
        Err(market_types::CanonicalDecodingError::LengthOutOfBounds)
    );
}

#[test]
fn canonical_decoder_rejects_trailing_bytes_and_survives_frame_mutations() {
    let trade = ObservedTrade::new(
        Price::parse("100").unwrap(),
        Quantity::new(1, QuantityUnit::Contract).unwrap(),
        TradeObservationKind::Regular,
    );
    let encode = |payload| {
        DomainEvent::new(
            InstrumentId::new(MarketId::Taifex, Symbol::new("A").unwrap()),
            TradingDate::from_epoch_days(0).unwrap(),
            SourceFormatId::new("X").unwrap(),
            MatchTime::from_unix_microseconds(1),
            None,
            payload,
        )
        .to_canonical_bytes()
        .unwrap()
    };
    let frames = [
        encode(EventPayload::QuoteSnapshot(
            QuoteSnapshot::new(
                empty_book(),
                Observation::NoObservation,
                Observation::NoObservation,
                MarketAnnotations::None,
            )
            .unwrap(),
        )),
        encode(EventPayload::BookSnapshot(market_types::BookSnapshot::new(
            empty_book(),
            MarketAnnotations::None,
        ))),
        encode(EventPayload::TradeBatch(
            TradeBatch::new(
                vec![trade],
                TradeBatchOrdering::Unspecified,
                Observation::NoObservation,
                MarketAnnotations::None,
            )
            .unwrap(),
        )),
        encode(EventPayload::IndicativeAuction(
            IndicativeAuction::new(
                AuctionObservation::opening(false, false),
                Observation::NoObservation,
                Observation::NoObservation,
                Observation::NoObservation,
                Observation::NoObservation,
                MarketAnnotations::None,
            )
            .unwrap(),
        )),
        encode(EventPayload::MarketStatus(MarketStatusObservation::new(
            Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
            MarketAnnotations::TpexQuote(TpexQuoteAnnotations::new(0x10, 0x02)),
        ))),
    ];

    for frame in frames {
        let mut with_trailing_byte = frame.clone();
        with_trailing_byte.push(0);
        assert_eq!(
            DomainEvent::from_canonical_bytes(&with_trailing_byte),
            Err(market_types::CanonicalDecodingError::TrailingBytes)
        );

        for end in 0..frame.len() {
            let result =
                std::panic::catch_unwind(|| DomainEvent::from_canonical_bytes(&frame[..end]));
            assert!(
                result.is_ok(),
                "decoder panicked on a truncated frame at {end}"
            );
            assert!(result.unwrap().is_err(), "truncated frame decoded at {end}");
        }

        for index in 0..frame.len() {
            for bit in 0..u8::BITS {
                let mut mutated = frame.clone();
                mutated[index] ^= 1 << bit;
                let result =
                    std::panic::catch_unwind(|| DomainEvent::from_canonical_bytes(&mutated));
                assert!(
                    result.is_ok(),
                    "decoder panicked at byte {index}, bit {bit}"
                );
                if let Ok(event) = result.unwrap() {
                    assert_eq!(
                        event.to_canonical_bytes().unwrap(),
                        mutated,
                        "decoder accepted a non-canonical frame at byte {index}, bit {bit}"
                    );
                }
            }
        }
    }
}
