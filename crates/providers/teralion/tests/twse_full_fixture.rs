use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::PathBuf,
};

use market_types::{
    AuctionEvidence, AuctionPurpose, EventPayload, InstrumentId, MarketId, MatchTime, Symbol,
    TradingDate,
};
use teralion_provider::twse::{NormalizerConfig, TwseNormalizer};

#[test]
fn synthetic_regular_fixture_normalizes_offline() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/providers/teralion/twse/SYNTH-TWSE-EQ/2026-07-20/regular-quotes");
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
            InstrumentId::new(MarketId::Twse, Symbol::new("SYNTH-TWSE-EQ").unwrap()),
            TradingDate::parse("2026-07-20").unwrap(),
            MatchTime::parse("2026-07-20T08:55:00+08:00").unwrap(),
            MatchTime::parse("2026-07-20T13:35:00+08:00").unwrap(),
        )
        .unwrap(),
    );
    let report = normalizer.normalize_json_lines(lines).unwrap();

    assert!(report.input_records() <= 512);
    assert!(!report.events().is_empty());
    assert!(report.outside_replay_window().len() as u64 <= report.input_records());
    assert!(report.known_skipped().is_empty());
    assert!(report.warnings().is_empty());

    let (quotes, trades, opening, closing) = report.events().iter().fold(
        (0_usize, 0_usize, 0_usize, 0_usize),
        |(quotes, trades, opening, closing), event| match event.payload() {
            EventPayload::QuoteSnapshot(_) => (quotes + 1, trades, opening, closing),
            EventPayload::TradeBatch(_) => (quotes, trades + 1, opening, closing),
            EventPayload::IndicativeAuction(auction) => match auction.observation().purpose() {
                AuctionEvidence::Known(AuctionPurpose::Opening) => {
                    (quotes, trades, opening + 1, closing)
                }
                AuctionEvidence::Known(AuctionPurpose::Closing) => {
                    (quotes, trades, opening, closing + 1)
                }
                AuctionEvidence::Known(AuctionPurpose::Periodic)
                | AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
                | AuctionEvidence::NoObservation
                | AuctionEvidence::Unknown => (quotes, trades, opening, closing),
            },
            EventPayload::BookSnapshot(_) => {
                panic!("TWSE M1 fixture must not produce BookSnapshot")
            }
            EventPayload::MarketStatus(_) => {
                panic!("TWSE M1 fixture has no status-only observations")
            }
        },
    );
    assert!(quotes > 0);
    assert!(trades + opening + closing > 0);
}
