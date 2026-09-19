use market_types::{
    EventPayload, IndicativeAuctionKind, InstantTrend, InstrumentId, MarketAnnotations, MarketId,
    MatchTime, Observation, QuantityUnit, Symbol, TradeObservationKind, TradingDate,
};
use teralion_provider::twse::{
    KnownSkipReason, NormalizationErrorKind, NormalizerConfig, RealtimeGroupError, TwseNormalizer,
    WarningKind,
};

fn normalizer() -> TwseNormalizer {
    TwseNormalizer::new(
        NormalizerConfig::new(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            TradingDate::parse("2026-07-27").unwrap(),
            MatchTime::parse("2026-07-27T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-27T13:35:00+08:00").unwrap(),
        )
        .unwrap(),
    )
}

fn quote(
    format: &str,
    match_time: &str,
    intermediate: bool,
    book: (&str, &str),
    deal: &str,
    cumulative: u64,
    flags: (u8, u8),
) -> String {
    let (bids, asks) = book;
    let (status, limit) = flags;
    format!(
        r#"{{"type":"quote","market":"twse","format":"{format}","symbol":"2330","match_time":"{match_time}","received_at":"2026-07-27T09:00:00.000001+08:00","bids":{bids},"asks":{asks},"deal":{deal},"cum_volume":{cumulative},"limit_flags":{limit},"status_flags":{status},"intermediate_print":{intermediate}}}"#
    )
}

fn complete_book() -> (&'static str, &'static str) {
    (
        r#"[{"price":100.000000000000000000,"quantity":2},{"price":99.5,"quantity":3}]"#,
        r#"[{"price":100.5,"quantity":4},{"price":101,"quantity":5}]"#,
    )
}

#[test]
fn snapshot_maps_exact_numeric_lexemes_and_null_deal() {
    let (bids, asks) = complete_book();
    let line = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T09:00:00+08:00",
        false,
        (bids, asks),
        "null",
        0,
        (16, 0),
    );
    let report = normalizer().normalize_json_lines([line]).unwrap();

    assert_eq!(report.input_records(), 1);
    assert!(report.warnings().is_empty());
    let EventPayload::QuoteSnapshot(snapshot) = report.events()[0].payload() else {
        panic!("expected quote snapshot")
    };
    assert_eq!(
        snapshot
            .book()
            .bids()
            .levels()
            .next()
            .unwrap()
            .price()
            .atoms(),
        100_000_000_000_000_000_000
    );
    assert_eq!(
        snapshot
            .book()
            .bids()
            .levels()
            .next()
            .unwrap()
            .displayed_quantity()
            .unit(),
        QuantityUnit::TradingUnit
    );
    assert_eq!(snapshot.trade(), &Observation::NoObservation);
    assert_eq!(snapshot.cumulative_volume().as_set().unwrap().value(), 0);
}

#[test]
fn market_order_book_quantity_is_not_treated_as_zero_price() {
    let line = quote(
        "STOCK_REALTIME",
        "2026-07-27T09:00:01+08:00",
        false,
        (
            r#"[{"price":0,"quantity":7},{"price":100,"quantity":2}]"#,
            r#"[{"price":101,"quantity":3}]"#,
        ),
        "null",
        0,
        (16, 0),
    );
    let report = normalizer().normalize_json_lines([line]).unwrap();
    let EventPayload::QuoteSnapshot(snapshot) = report.events()[0].payload() else {
        panic!("expected quote snapshot")
    };
    assert_eq!(
        snapshot
            .book()
            .bids()
            .market_order_quantity()
            .unwrap()
            .value(),
        7
    );
    assert_eq!(
        snapshot
            .book()
            .bids()
            .levels()
            .next()
            .unwrap()
            .price()
            .atoms(),
        100_000_000_000_000_000_000
    );
}

#[test]
fn zero_price_market_order_must_be_the_first_book_entry() {
    let line = quote(
        "STOCK_REALTIME",
        "2026-07-27T09:00:01+08:00",
        false,
        (
            r#"[{"price":100,"quantity":2},{"price":0,"quantity":7}]"#,
            "[]",
        ),
        "null",
        0,
        (16, 0),
    );
    assert!(matches!(
        normalizer()
            .normalize_json_lines([line])
            .unwrap_err()
            .kind(),
        NormalizationErrorKind::InvalidPayload(
            "zero-price market-order level must be the first book entry"
        )
    ));
}

#[test]
fn trial_quotes_become_opening_and_closing_auction_events() {
    let (bids, asks) = complete_book();
    let opening = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T08:55:00+08:00",
        false,
        (bids, asks),
        r#"{"price":100,"quantity":2}"#,
        0,
        (128, 0),
    );
    let closing = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T13:25:00+08:00",
        false,
        (bids, asks),
        r#"{"price":101,"quantity":3}"#,
        0,
        (128, 0),
    );
    let report = normalizer()
        .normalize_json_lines([opening, closing])
        .unwrap();
    let EventPayload::IndicativeAuction(opening) = report.events()[0].payload() else {
        panic!("expected opening indicative auction")
    };
    let EventPayload::IndicativeAuction(closing) = report.events()[1].payload() else {
        panic!("expected closing indicative auction")
    };
    assert_eq!(opening.kind(), IndicativeAuctionKind::Opening);
    assert_eq!(closing.kind(), IndicativeAuctionKind::Closing);
}

#[test]
fn delayed_open_trial_is_typed_as_opening_and_keeps_delayed_flag() {
    let (bids, asks) = complete_book();
    for (match_time, status) in [
        ("2026-07-27T08:59:58+08:00", 0xc8),
        ("2026-07-27T09:01:00+08:00", 0xc0),
    ] {
        let report = normalizer()
            .normalize_json_lines([quote(
                "STOCK_REALTIME",
                match_time,
                false,
                (bids, asks),
                r#"{"price":100,"quantity":2}"#,
                10,
                (status, 0),
            )])
            .unwrap();
        let EventPayload::IndicativeAuction(auction) = report.events()[0].payload() else {
            panic!("delayed opening trial must not become a firm quote")
        };
        assert_eq!(auction.kind(), IndicativeAuctionKind::Opening);
        let market_types::MarketAnnotations::TwseQuote(annotations) = auction.annotations() else {
            panic!("TWSE source annotations must be preserved")
        };
        assert!(annotations.status().delayed_open());
    }
}

#[test]
fn conflicting_delayed_open_and_delayed_close_flags_are_rejected() {
    let (bids, asks) = complete_book();
    let error = normalizer()
        .normalize_json_lines([quote(
            "STOCK_REALTIME",
            "2026-07-27T09:01:00+08:00",
            false,
            (bids, asks),
            r#"{"price":100,"quantity":2}"#,
            10,
            (0xe0, 0),
        )])
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::InvalidPayload(_)
    ));
}

#[test]
fn intraday_unmarked_trial_becomes_an_isolated_indicative_observation() {
    let (bids, asks) = complete_book();
    let trial = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T09:04:43+08:00",
        false,
        (bids, asks),
        r#"{"price":100,"quantity":2}"#,
        10,
        (128, 0),
    );
    let report = normalizer().normalize_json_lines([trial]).unwrap();
    let EventPayload::IndicativeAuction(auction) = report.events()[0].payload() else {
        panic!("expected an unclassified indicative auction")
    };
    let market_types::MarketAnnotations::TwseQuote(annotations) = auction.annotations() else {
        panic!("expected TWSE annotations")
    };
    assert!(annotations.status().trial());
    assert_eq!(auction.kind(), IndicativeAuctionKind::IntradayUnclassified);
    assert_eq!(auction.quantity().as_set().unwrap().value(), 2);
}

#[test]
fn intraday_trial_keeps_direction_annotation_without_claiming_trial_semantics() {
    let (bids, asks) = complete_book();
    let down = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T09:04:43+08:00",
        false,
        (bids, asks),
        r#"{"price":100,"quantity":2}"#,
        10,
        (128, 1),
    );
    let (bids, asks) = complete_book();
    let up = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T09:04:48+08:00",
        false,
        (bids, asks),
        r#"{"price":101,"quantity":3}"#,
        10,
        (128, 2),
    );
    let report = normalizer().normalize_json_lines([down, up]).unwrap();
    let kinds = report
        .events()
        .iter()
        .map(|event| match event.payload() {
            EventPayload::IndicativeAuction(auction) => auction.kind(),
            _ => panic!("trial must be indicative"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            IndicativeAuctionKind::IntradayUnclassified,
            IndicativeAuctionKind::IntradayUnclassified,
        ]
    );
    let trends = report
        .events()
        .iter()
        .map(|event| {
            let EventPayload::IndicativeAuction(auction) = event.payload() else {
                unreachable!("trial must be indicative")
            };
            let MarketAnnotations::TwseQuote(annotations) = auction.annotations() else {
                unreachable!("TWSE source annotations must be preserved")
            };
            annotations.limits().instant_trend()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        trends,
        vec![
            InstantTrend::VolatilityInterruptionDown,
            InstantTrend::VolatilityInterruptionUp,
        ]
    );
}

#[test]
fn non_trial_volatility_pause_without_book_becomes_status_observation() {
    let pause = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T09:04:43+08:00",
        false,
        ("[]", "[]"),
        "null",
        10,
        (0x10, 0x01),
    );
    let report = normalizer().normalize_json_lines([pause]).unwrap();
    let EventPayload::MarketStatus(status) = report.events()[0].payload() else {
        panic!("non-trial pause without a disclosed book is a status observation")
    };
    assert_eq!(status.cumulative_volume().as_set().unwrap().value(), 10);
    let MarketAnnotations::TwseQuote(annotations) = status.annotations() else {
        panic!("expected TWSE source annotations")
    };
    assert_eq!(
        annotations.limits().instant_trend(),
        InstantTrend::VolatilityInterruptionDown
    );
}

#[test]
fn zero_quantity_volatility_pause_is_a_status_observation() {
    let pause = quote(
        "STOCK_REALTIME",
        "2026-07-27T09:04:43+08:00",
        false,
        ("[]", "[]"),
        r#"{"price":100,"quantity":0}"#,
        10,
        (0x10, 0x02),
    );
    let report = normalizer().normalize_json_lines([pause]).unwrap();
    let EventPayload::MarketStatus(status) = report.events()[0].payload() else {
        panic!("a non-trial volatility pause must become a status observation")
    };

    assert_eq!(status.cumulative_volume().as_set().unwrap().value(), 10);
    let MarketAnnotations::TwseQuote(annotations) = status.annotations() else {
        panic!("expected TWSE source annotations")
    };
    assert_eq!(
        annotations.limits().instant_trend(),
        InstantTrend::VolatilityInterruptionUp
    );
}

#[test]
fn zero_quantity_deal_is_rejected_without_pause_flags_or_with_a_book() {
    let invalid_cases = [
        quote(
            "STOCK_REALTIME",
            "2026-07-27T09:04:43+08:00",
            false,
            ("[]", "[]"),
            r#"{"price":100,"quantity":0}"#,
            10,
            (0x10, 0),
        ),
        quote(
            "STOCK_REALTIME",
            "2026-07-27T09:04:43+08:00",
            false,
            complete_book(),
            r#"{"price":100,"quantity":0}"#,
            10,
            (0x10, 0x02),
        ),
    ];

    for line in invalid_cases {
        let error = normalizer().normalize_json_lines([line]).unwrap_err();
        assert!(matches!(
            error.kind(),
            NormalizationErrorKind::InvalidPayload(_)
        ));
    }
}

#[test]
fn realtime_pair_is_grouped_by_match_time_and_emits_trade_then_quote() {
    let (bids, asks) = complete_book();
    let match_time = "2026-07-27T09:28:49.274622+08:00";
    let intermediate = quote(
        "STOCK_REALTIME",
        match_time,
        true,
        ("[]", "[]"),
        r#"{"price":100,"quantity":1}"#,
        10,
        (16, 0),
    );
    let final_quote = quote(
        "STOCK_REALTIME",
        match_time,
        false,
        (bids, asks),
        r#"{"price":100.5,"quantity":2}"#,
        12,
        (16, 0),
    );

    let report = normalizer()
        .normalize_json_lines([final_quote, intermediate])
        .unwrap();
    assert_eq!(report.events().len(), 2);

    let EventPayload::TradeBatch(batch) = report.events()[0].payload() else {
        panic!("expected intermediate trade batch first")
    };
    assert_eq!(batch.trades().len(), 1);
    assert_eq!(
        batch.trades()[0].observation_kind(),
        TradeObservationKind::Intermediate
    );
    assert_eq!(batch.cumulative_volume().as_set().unwrap().value(), 10);

    let EventPayload::QuoteSnapshot(snapshot) = report.events()[1].payload() else {
        panic!("expected final quote second")
    };
    assert_eq!(snapshot.cumulative_volume().as_set().unwrap().value(), 12);
}

#[test]
fn realtime_group_preserves_multiple_intermediate_prints_before_pause_trigger() {
    let match_time = "2026-07-27T09:28:49.274622+08:00";
    let first_intermediate = quote(
        "STOCK_REALTIME",
        match_time,
        true,
        ("[]", "[]"),
        r#"{"price":100,"quantity":1}"#,
        373,
        (16, 0),
    );
    let second_intermediate = quote(
        "STOCK_REALTIME",
        match_time,
        true,
        ("[]", "[]"),
        r#"{"price":101,"quantity":2}"#,
        375,
        (16, 0),
    );
    let pause_trigger = quote(
        "STOCK_REALTIME",
        match_time,
        false,
        ("[]", "[]"),
        r#"{"price":101,"quantity":0}"#,
        375,
        (16, 0x02),
    );

    let report = normalizer()
        .normalize_json_lines([first_intermediate, second_intermediate, pause_trigger])
        .unwrap();
    assert_eq!(report.events().len(), 2);

    let EventPayload::TradeBatch(batch) = report.events()[0].payload() else {
        panic!("intermediate prints must be preserved as one trade batch")
    };
    assert_eq!(batch.trades().len(), 2);
    assert_eq!(
        batch.trades()[0].price().atoms(),
        100_000_000_000_000_000_000
    );
    assert_eq!(
        batch.trades()[1].price().atoms(),
        101_000_000_000_000_000_000
    );
    assert_eq!(batch.cumulative_volume().as_set().unwrap().value(), 375);

    let EventPayload::MarketStatus(status) = report.events()[1].payload() else {
        panic!("pause trigger must become a status observation")
    };
    assert_eq!(status.cumulative_volume().as_set().unwrap().value(), 375);
}

#[test]
fn incomplete_and_volume_mismatched_realtime_groups_are_rejected() {
    let match_time = "2026-07-27T09:28:49.274622+08:00";
    let intermediate = quote(
        "STOCK_REALTIME",
        match_time,
        true,
        ("[]", "[]"),
        r#"{"price":100,"quantity":1}"#,
        10,
        (16, 0),
    );
    let error = normalizer()
        .normalize_json_lines([intermediate.clone()])
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::UnsupportedRealtimeMatchGroup(
            RealtimeGroupError::ExpectedOneIntermediateAndOneFinal { records: 1 }
        )
    ));

    let (bids, asks) = complete_book();
    let final_quote = quote(
        "STOCK_REALTIME",
        match_time,
        false,
        (bids, asks),
        r#"{"price":100.5,"quantity":2}"#,
        13,
        (16, 0),
    );
    let error = normalizer()
        .normalize_json_lines([intermediate, final_quote])
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::UnsupportedRealtimeMatchGroup(
            RealtimeGroupError::CumulativeVolumeMismatch
        )
    ));
}

#[test]
fn window_skip_format_and_reserved_flags_are_explicitly_classified() {
    let (bids, asks) = complete_book();
    let outside = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T08:54:59.999999+08:00",
        false,
        (bids, asks),
        "null",
        0,
        (128, 0),
    );
    let odd_lot = quote(
        "INTRADAY_ODDLOT_REALTIME",
        "2026-07-27T09:00:01+08:00",
        false,
        (bids, asks),
        "null",
        0,
        (0, 0),
    );
    let reserved = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T09:00:02+08:00",
        false,
        (bids, asks),
        "null",
        0,
        (3, 0b11_11_11_11),
    );
    let report = normalizer()
        .normalize_json_lines([outside, odd_lot, reserved])
        .unwrap();

    assert_eq!(report.events().len(), 1);
    assert_eq!(report.outside_replay_window().len(), 1);
    assert_eq!(report.known_skipped().len(), 1);
    assert_eq!(
        report.known_skipped()[0].reason(),
        KnownSkipReason::IntradayOddLot
    );
    assert_eq!(report.warnings().len(), 5);
    assert_eq!(
        report.warnings()[0].kind(),
        WarningKind::ReservedStatusBits(3)
    );
}

#[test]
fn unknown_format_and_invalid_book_are_rejected_with_context() {
    let (bids, asks) = complete_book();
    let unknown = quote(
        "FUTURE_FORMAT",
        "2026-07-27T09:00:00+08:00",
        false,
        (bids, asks),
        "null",
        0,
        (0, 0),
    );
    let error = normalizer().normalize_json_lines([unknown]).unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::UnsupportedFormat(format) if format.as_ref() == "FUTURE_FORMAT"
    ));
    assert_eq!(error.context().source_format(), Some("FUTURE_FORMAT"));

    let bad_book = quote(
        "STOCK_SNAPSHOT",
        "2026-07-27T09:00:00+08:00",
        false,
        (
            r#"[{"price":99,"quantity":1},{"price":100,"quantity":1}]"#,
            asks,
        ),
        "null",
        0,
        (0, 0),
    );
    let error = normalizer().normalize_json_lines([bad_book]).unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::InvalidBook(_)
    ));
}
