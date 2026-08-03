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
    InstrumentProfile, KnownSkipReason, NormalizationErrorKind, NormalizerConfig, TaifexNormalizer,
};

fn fixture_lines() -> Vec<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/teralion/taifex/SYNTH-OPT/2026-07-20");
    let mut paths = ["after-hours", "regular"]
        .into_iter()
        .flat_map(|segment| {
            fs::read_dir(root.join(segment))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "jsonl")
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
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

fn option_normalizer() -> TaifexNormalizer {
    TaifexNormalizer::new(
        NormalizerConfig::for_profile(
            InstrumentId::new(MarketId::Taifex, Symbol::new("SYNTH-OPT").unwrap()),
            TradingDate::parse("2026-07-20").unwrap(),
            InstrumentProfile::IndexOptions,
            [
                (
                    MatchTime::parse("2026-07-20T08:40:00+08:00").unwrap(),
                    MatchTime::parse("2026-07-20T13:50:00+08:00").unwrap(),
                ),
                (
                    MatchTime::parse("2026-07-17T14:55:00+08:00").unwrap(),
                    MatchTime::parse("2026-07-20T05:05:00+08:00").unwrap(),
                ),
            ],
        )
        .unwrap(),
    )
}

#[test]
fn synthetic_option_fixture_normalizes_cross_session_events() {
    let report = option_normalizer()
        .normalize_json_lines(fixture_lines())
        .unwrap();

    assert!(report.input_records() <= 1_024);
    assert!(!report.events().is_empty());
    assert!(report.outside_replay_window().len() as u64 <= report.input_records());
    assert!(report.known_skipped().len() as u64 <= report.input_records());

    let mut formats = BTreeMap::new();
    let mut payloads = (0_usize, 0_usize, 0_usize, 0_usize);
    for event in report.events() {
        *formats
            .entry(event.source_format().as_str())
            .or_insert(0_usize) += 1;
        match event.payload() {
            EventPayload::BookSnapshot(snapshot) => {
                payloads.0 += 1;
                for level in snapshot
                    .book()
                    .bids()
                    .levels()
                    .chain(snapshot.book().asks().levels())
                {
                    assert_eq!(level.displayed_quantity().unit(), QuantityUnit::Contract);
                }
            }
            EventPayload::TradeBatch(batch) => {
                payloads.1 += 1;
                assert_eq!(batch.trades()[0].quantity().unit(), QuantityUnit::Contract);
            }
            EventPayload::IndicativeOpeningAuction(auction) => {
                payloads.2 += 1;
                assert_eq!(auction.quantity(), &Observation::NoObservation);
            }
            EventPayload::QuoteSnapshot(_) | EventPayload::IndicativeClosingAuction(_) => {
                payloads.3 += 1;
            }
        }
    }
    assert!(formats.contains_key("I020"));
    assert!(formats.contains_key("I022"));
    assert!(formats.contains_key("I080"));
    assert!(formats.contains_key("I082"));
    assert!(payloads.0 > 0);
    assert!(payloads.1 > 0);
    assert!(payloads.2 > 0);
    assert!(
        report
            .known_skipped()
            .iter()
            .any(|skipped| skipped.reason() == KnownSkipReason::IntradayHighLow)
    );
}

#[test]
fn m5_option_profile_rejects_futures_wire_market_and_unknown_format() {
    let line = fixture_lines().into_iter().next().unwrap();
    let wrong_market = line.replace("\"market\":\"taifex_opt\"", "\"market\":\"taifex_fut\"");
    let error = option_normalizer()
        .normalize_json_lines([wrong_market])
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::InvalidIdentity {
            field: "market",
            ..
        }
    ));

    let unknown = line.replace("\"format\":\"I072\"", "\"format\":\"I999\"");
    let error = option_normalizer()
        .normalize_json_lines([unknown])
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::UnsupportedFormat(format) if format.as_ref() == "I999"
    ));
}
