use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::PathBuf,
};

use market_types::{EventPayload, InstrumentId, MarketId, MatchTime, Symbol, TradingDate};
use twse_normalizer::{NormalizerConfig, TwseNormalizer};

#[test]
fn complete_committed_regular_fixture_normalizes_offline() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/teralion/twse/2330/2026-07-27/regular-quotes");
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
    let normalizer = TwseNormalizer::new(
        NormalizerConfig::new(
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            TradingDate::parse("2026-07-27").unwrap(),
            MatchTime::parse("2026-07-27T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-27T13:35:00+08:00").unwrap(),
        )
        .unwrap(),
    );
    let report = normalizer.normalize_json_lines(lines).unwrap();

    assert_eq!(report.input_records(), 73_796);
    assert_eq!(report.events().len(), 73_795);
    assert_eq!(report.outside_replay_window().len(), 1);
    assert!(report.known_skipped().is_empty());
    assert!(report.warnings().is_empty());

    let (quotes, trades) =
        report
            .events()
            .iter()
            .fold((0_usize, 0_usize), |(quotes, trades), event| {
                match event.payload() {
                    EventPayload::QuoteSnapshot(_) => (quotes + 1, trades),
                    EventPayload::TradeBatch(_) => (quotes, trades + 1),
                    EventPayload::BookSnapshot(_) => {
                        panic!("TWSE M1 fixture must not produce BookSnapshot")
                    }
                }
            });
    assert_eq!(quotes, 73_792);
    assert_eq!(trades, 3);

    for match_time in [
        "2026-07-27T09:28:49.274622+08:00",
        "2026-07-27T09:30:55.252155+08:00",
        "2026-07-27T10:29:59.907157+08:00",
    ] {
        let match_time = MatchTime::parse(match_time).unwrap();
        let matching = report
            .events()
            .iter()
            .filter(|event| event.match_time() == match_time)
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 2);
        assert!(matches!(matching[0].payload(), EventPayload::TradeBatch(_)));
        assert!(matches!(
            matching[1].payload(),
            EventPayload::QuoteSnapshot(_)
        ));
    }
}
