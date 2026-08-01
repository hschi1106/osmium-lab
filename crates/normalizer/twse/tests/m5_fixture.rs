use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::PathBuf,
};

use market_types::{EventPayload, InstrumentId, MarketId, MatchTime, Symbol, TradingDate};
use twse_normalizer::{NormalizationErrorKind, NormalizerConfig, TwseNormalizer};

fn fixture_lines() -> Vec<String> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/teralion/twse/03003T/2026-07-20/regular-quotes");
    let mut paths = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
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

fn warrant_normalizer() -> TwseNormalizer {
    TwseNormalizer::new(
        NormalizerConfig::new_warrant(
            InstrumentId::new(MarketId::Twse, Symbol::new("03003T").unwrap()),
            TradingDate::parse("2026-07-20").unwrap(),
            MatchTime::parse("2026-07-20T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-20T13:35:00+08:00").unwrap(),
        )
        .unwrap(),
    )
}

#[test]
fn m5_warrant_fixture_normalizes_all_real_records() {
    let report = warrant_normalizer()
        .normalize_json_lines(fixture_lines())
        .unwrap();

    assert_eq!(report.input_records(), 111);
    assert_eq!(report.events().len(), 111);
    assert!(report.outside_replay_window().is_empty());
    assert!(report.known_skipped().is_empty());
    assert!(report.warnings().is_empty());

    let (quotes, trades, opening, closing) = report.events().iter().fold(
        (0_usize, 0_usize, 0_usize, 0_usize),
        |(quotes, trades, opening, closing), event| match event.payload() {
            EventPayload::QuoteSnapshot(_) => (quotes + 1, trades, opening, closing),
            EventPayload::TradeBatch(_) => (quotes, trades + 1, opening, closing),
            EventPayload::IndicativeOpeningAuction(_) => (quotes, trades, opening + 1, closing),
            EventPayload::IndicativeClosingAuction(_) => (quotes, trades, opening, closing + 1),
            EventPayload::BookSnapshot(_) => panic!("warrant quote must not produce a book event"),
        },
    );
    assert_eq!(quotes, 99);
    assert_eq!(trades, 0);
    assert_eq!(opening, 0);
    assert_eq!(closing, 12);
}

#[test]
fn m5_warrant_profile_rejects_wrong_market_and_equity_profile_rejects_warrant_format() {
    let line = fixture_lines().into_iter().next().unwrap();
    let wrong_market = line.replace("\"market\":\"twse\"", "\"market\":\"taifex_opt\"");
    let error = warrant_normalizer()
        .normalize_json_lines([wrong_market])
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::InvalidIdentity {
            field: "market",
            ..
        }
    ));

    let equity = TwseNormalizer::new(
        NormalizerConfig::new(
            InstrumentId::new(MarketId::Twse, Symbol::new("03003T").unwrap()),
            TradingDate::parse("2026-07-20").unwrap(),
            MatchTime::parse("2026-07-20T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-20T13:35:00+08:00").unwrap(),
        )
        .unwrap(),
    );
    let error = equity.normalize_json_lines([line]).unwrap_err();
    assert!(matches!(
        error.kind(),
        NormalizationErrorKind::UnsupportedFormat(format)
            if format.as_ref() == "WARRANT_REALTIME"
    ));
}
