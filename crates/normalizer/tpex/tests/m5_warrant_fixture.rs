use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::PathBuf,
};

use market_types::{EventPayload, InstrumentId, MarketId, MatchTime, Symbol, TradingDate};
use tpex_normalizer::{NormalizerConfig, TpexNormalizer};

#[test]
fn complete_committed_warrant_fixture_normalizes_offline() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/teralion/tpex/72328U/2026-07-20/regular-quotes");
    let mut shards = fs::read_dir(&fixture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect::<Vec<_>>();
    shards.sort();

    let lines = shards.into_iter().flat_map(|path| {
        BufReader::new(File::open(path).unwrap())
            .lines()
            .map(Result::unwrap)
    });
    let normalizer = TpexNormalizer::new(
        NormalizerConfig::new_warrant(
            InstrumentId::new(MarketId::Tpex, Symbol::new("72328U").unwrap()),
            TradingDate::parse("2026-07-20").unwrap(),
            MatchTime::parse("2026-07-20T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-20T13:35:00+08:00").unwrap(),
        )
        .unwrap(),
    );
    let report = normalizer.normalize_json_lines(lines).unwrap();

    assert_eq!(report.input_records(), 11);
    assert_eq!(report.events().len(), 11);
    assert!(report.outside_replay_window().is_empty());
    assert!(report.known_skipped().is_empty());
    assert!(report.warnings().is_empty());

    let (quotes, opening, closing) = report.events().iter().fold(
        (0_usize, 0_usize, 0_usize),
        |(quotes, opening, closing), event| match event.payload() {
            EventPayload::QuoteSnapshot(_) => (quotes + 1, opening, closing),
            EventPayload::IndicativeOpeningAuction(_) => (quotes, opening + 1, closing),
            EventPayload::IndicativeClosingAuction(_) => (quotes, opening, closing + 1),
            EventPayload::TradeBatch(_) => panic!("TPEx warrant fixture has no trade prints"),
            EventPayload::BookSnapshot(_) => {
                panic!("TPEx warrant fixture must not produce BookSnapshot")
            }
        },
    );
    assert_eq!(quotes, 3);
    assert_eq!(opening, 2);
    assert_eq!(closing, 6);
}
