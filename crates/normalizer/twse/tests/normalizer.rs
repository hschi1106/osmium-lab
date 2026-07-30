use market_types::{
    EventPayload, InstrumentId, MarketId, MatchTime, Observation, QuantityUnit, Symbol,
    TradePrintKind, TradingDate,
};
use twse_normalizer::{
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
        (128, 0),
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
    assert_eq!(batch.trades()[0].print_kind(), TradePrintKind::Intermediate);
    assert_eq!(batch.cumulative_volume().as_set().unwrap().value(), 10);

    let EventPayload::QuoteSnapshot(snapshot) = report.events()[1].payload() else {
        panic!("expected final quote second")
    };
    assert_eq!(snapshot.cumulative_volume().as_set().unwrap().value(), 12);
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
