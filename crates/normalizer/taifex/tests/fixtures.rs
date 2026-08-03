use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::PathBuf,
};

use market_types::{
    EventPayload, InstrumentId, MarketId, MatchTime, Observation, QuantityUnit, Symbol, TradingDate,
};
use taifex_normalizer::{
    KnownSkipReason, NormalizationErrorKind, NormalizerConfig, TaifexNormalizer,
};

fn fixture_root(symbol: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../../fixtures/teralion/taifex/{symbol}/2026-07-20"
    ))
}

fn lines(symbol: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for segment in ["after-hours", "regular"] {
        let directory = fixture_root(symbol).join(segment);
        if let Ok(entries) = fs::read_dir(directory) {
            paths.extend(entries.map(|entry| entry.unwrap().path()).filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "jsonl")
            }));
        }
    }
    paths.sort();
    paths
        .into_iter()
        .flat_map(|path| {
            BufReader::new(File::open(path).unwrap())
                .lines()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn normalizer(symbol: &str) -> TaifexNormalizer {
    TaifexNormalizer::new(
        NormalizerConfig::new(
            InstrumentId::new(MarketId::Taifex, Symbol::new(symbol).unwrap()),
            TradingDate::parse("2026-07-20").unwrap(),
            MatchTime::parse("2026-07-17T14:00:00+08:00").unwrap(),
            MatchTime::parse("2026-07-20T14:00:00+08:00").unwrap(),
        )
        .unwrap(),
    )
}

#[test]
fn synthetic_futures_fixture_normalizes_with_opening_events_only() {
    let symbol = "SYNTH-FUT";
    let report = normalizer(symbol)
        .normalize_json_lines(lines(symbol))
        .unwrap();
    assert!(report.input_records() > 0, "{symbol}");
    assert!(report.input_records() <= 1_024, "{symbol}");
    assert!(!report.events().is_empty(), "{symbol}");
    assert!(
        report.known_skipped().len() as u64 <= report.input_records(),
        "{symbol}"
    );
    assert!(report.outside_replay_window().is_empty(), "{symbol}");

    assert!(
        report
            .events()
            .iter()
            .all(|event| { !matches!(event.payload(), EventPayload::IndicativeClosingAuction(_)) })
    );
    assert!(
        report
            .events()
            .iter()
            .any(|event| { matches!(event.payload(), EventPayload::IndicativeOpeningAuction(_)) })
    );
}

#[test]
fn i022_zero_zero_is_a_no_observation_opening_event() {
    let line = r#"{"first_packet":true,"format":"I022","market":"taifex_fut","match_time":"2026-07-20T08:40:00+08:00","received_at":"2026-07-20T08:40:00.001000+08:00","symbol":"SYNTH-FUT","trades":[{"price":0,"quantity":0}],"type":"trade"}"#;
    let report = normalizer("SYNTH-FUT")
        .normalize_json_lines([line])
        .unwrap();
    let event = report
        .events()
        .iter()
        .find(|event| {
            event.source_format().as_str() == "I022"
                && matches!(
                    event.payload(),
                    EventPayload::IndicativeOpeningAuction(auction)
                        if auction.price().as_set().is_none()
                )
        })
        .expect("fixture contains zero/zero I022");
    let EventPayload::IndicativeOpeningAuction(auction) = event.payload() else {
        panic!("I022 must map to opening auction event")
    };
    assert_eq!(auction.price(), &Observation::NoObservation);
    assert_eq!(auction.quantity(), &Observation::NoObservation);
    assert_eq!(auction.book(), &Observation::NoObservation);
}

#[test]
fn skips_keep_exchange_specific_reasons() {
    let report = normalizer("SYNTH-FUT")
        .normalize_json_lines(lines("SYNTH-FUT"))
        .unwrap();
    let mut counts = BTreeMap::new();
    for skipped in report.known_skipped() {
        *counts.entry(skipped.reason()).or_insert(0_usize) += 1;
    }
    assert!(!counts.is_empty());
    assert!(counts.contains_key(&KnownSkipReason::OrderStatistics));
}

#[test]
fn timeline_quantities_use_contract_units() {
    let report = normalizer("SYNTH-FUT")
        .normalize_json_lines(lines("SYNTH-FUT"))
        .unwrap();
    for event in report.events() {
        match event.payload() {
            EventPayload::TradeBatch(batch) => {
                assert_eq!(batch.trades()[0].quantity().unit(), QuantityUnit::Contract);
            }
            EventPayload::BookSnapshot(snapshot) => {
                assert_eq!(
                    snapshot.book().quantity_unit(),
                    Some(QuantityUnit::Contract)
                );
            }
            EventPayload::IndicativeOpeningAuction(auction) => {
                assert!(
                    auction
                        .quantity()
                        .as_set()
                        .is_none_or(|quantity| quantity.unit() == QuantityUnit::Contract)
                );
            }
            EventPayload::QuoteSnapshot(_) | EventPayload::IndicativeClosingAuction(_) => {
                panic!("unexpected payload in TAIFEX fixture")
            }
        }
    }
}

#[test]
fn i020_continuation_is_rejected_in_the_current_source_boundary() {
    let line = r#"{"first_packet":false,"format":"I020","market":"taifex_fut","match_time":"2026-07-20T08:45:00+08:00","received_at":"2026-07-20T08:45:00.001000+08:00","symbol":"SYNTH-FUT","trades":[{"price":200,"quantity":1}],"type":"trade"}"#;
    let error = normalizer("SYNTH-FUT")
        .normalize_json_lines([line])
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::InvalidPayload("unsupported I020 continuation")
    ));
}

#[test]
fn unknown_format_reports_the_format_identity() {
    let line = r#"{"format":"I999","market":"taifex_fut","match_time":"2026-07-20T08:45:00+08:00","received_at":"2026-07-20T08:45:00.001000+08:00","symbol":"SYNTH-FUT","trades":[],"type":"trade"}"#;
    let error = normalizer("SYNTH-FUT")
        .normalize_json_lines([line])
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::UnsupportedFormat(format) if format.as_ref() == "I999"
    ));
}
