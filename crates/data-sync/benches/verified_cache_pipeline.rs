use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use data_sync::{
    ArchiveKind, ArchiveTimestamp, CacheBuilder, CacheReader, LocalSourceRepository,
    StagingRevision, TeralionCredential, TeralionQuery, TeralionRequest, TeralionSync,
    TeralionTransport, TransportError,
};
use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{InstrumentId, MarketId, MatchTime, Symbol, TradingDate, UtcOffsetMinutes};
use replay_engine::ReplayCore;

const EVENTS: usize = 50_000;
const PAGE_SIZE: usize = 5_000;
const ROUNDS: usize = 5;
const SOURCE_FIXTURE: &str = include_str!(
    "../../../fixtures/providers/teralion/twse/SYNTH-TWSE-EQ/2026-07-20/regular-quotes/0001.jsonl"
);

struct FixtureTransport {
    pages: Vec<Vec<u8>>,
    daily: Vec<u8>,
}

impl TeralionTransport for FixtureTransport {
    fn execute(
        &mut self,
        request: &TeralionRequest,
        _: &TeralionCredential,
    ) -> Result<Vec<u8>, TransportError> {
        if !request.query().is_paged() {
            return Ok(self.daily.clone());
        }
        let page_index = match request.cursor() {
            None => 0,
            Some(cursor) => cursor
                .strip_prefix("bench-page-")
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| TransportError::new(false, "invalid benchmark cursor"))?,
        };
        self.pages
            .get(page_index)
            .cloned()
            .ok_or_else(|| TransportError::new(false, "benchmark cursor is out of range"))
    }
}

fn make_source_pages() -> Vec<Vec<u8>> {
    let template = SOURCE_FIXTURE
        .lines()
        .nth(1)
        .expect("fixture contains a firm TWSE quote");
    let mut template: serde_json::Value = serde_json::from_str(template).unwrap();
    let match_base = MatchTime::parse("2026-07-20T09:00:00+08:00").unwrap();
    let offset = UtcOffsetMinutes::new(480).unwrap();
    let mut records = Vec::with_capacity(EVENTS);
    for index in 0..EVENTS {
        let micros = match_base
            .as_unix_microseconds()
            .checked_add(index as i64 * 100_000)
            .expect("synthetic benchmark timeline fits in i64");
        let match_time = MatchTime::from_unix_microseconds(micros);
        let received_at = MatchTime::from_unix_microseconds(micros + 1_000);
        template["match_time"] = serde_json::Value::String(match_time.to_iso8601(offset).unwrap());
        template["received_at"] =
            serde_json::Value::String(received_at.to_iso8601(offset).unwrap());
        template["cum_volume"] = serde_json::Value::from(index as u64 + 1);
        records.push(serde_json::to_vec(&template).unwrap());
    }

    records
        .chunks(PAGE_SIZE)
        .enumerate()
        .map(|(page_index, records)| {
            let next = (page_index + 1 < EVENTS.div_ceil(PAGE_SIZE))
                .then(|| format!("bench-page-{}", page_index + 1));
            let mut body = br#"{"items":["#.to_vec();
            for (record_index, record) in records.iter().enumerate() {
                if record_index != 0 {
                    body.push(b',');
                }
                body.extend_from_slice(record);
            }
            body.extend_from_slice(br#"],"next_cursor":"#);
            match next {
                Some(cursor) => {
                    body.push(b'"');
                    body.extend_from_slice(cursor.as_bytes());
                    body.push(b'"');
                }
                None => body.extend_from_slice(b"null"),
            }
            body.push(b'}');
            body
        })
        .collect()
}

fn median_duration(mut durations: Vec<Duration>) -> Duration {
    durations.sort_unstable();
    durations[durations.len() / 2]
}

fn replay_core(instrument: &InstrumentId, date: TradingDate) -> ReplayCore {
    ReplayCore::new(
        vec![MarketState::new(instrument.clone(), date)],
        MarketStateReducer::twse_regular(),
        ReducerContext::new(
            date,
            SessionSegmentId::new("regular").unwrap(),
            SegmentBoundaryPolicy::Carry,
            1,
        ),
    )
    .unwrap()
}

fn main() {
    let instrument = InstrumentId::new(MarketId::Twse, Symbol::new("SYNTH-TWSE-EQ").unwrap());
    let date = TradingDate::parse("2026-07-20").unwrap();
    let query = TeralionQuery::ticks(
        instrument.clone(),
        ArchiveTimestamp::parse("2026-07-20T08:55:00+08:00").unwrap(),
        ArchiveTimestamp::parse("2026-07-20T13:35:00+08:00").unwrap(),
        [ArchiveKind::Quote],
        PAGE_SIZE as u16,
    )
    .unwrap();
    let daily_query = TeralionQuery::daily_instrument(instrument.clone(), date);
    let root = tempfile::tempdir().unwrap();
    let mut staging = StagingRevision::create(root.path(), "benchmark-source").unwrap();
    let credential = TeralionCredential::new("synthetic-benchmark-only").unwrap();
    let mut sync = TeralionSync::new(FixtureTransport {
        pages: make_source_pages(),
        daily: br#"{"symbol":"SYNTH-TWSE-EQ"}"#.to_vec(),
    });
    let source_report = sync
        .sync_pages(query.clone(), &credential, &mut staging)
        .unwrap();
    let daily = sync.fetch_single(daily_query.clone(), &credential).unwrap();
    staging
        .stage_daily_instrument(daily_query.identity(), &daily)
        .unwrap();
    staging
        .publish(query.identity(), source_report.terminal)
        .unwrap();

    let normalizer = twse_normalizer::NormalizerConfig::new(
        instrument.clone(),
        date,
        MatchTime::parse("2026-07-20T08:55:00+08:00").unwrap(),
        MatchTime::parse("2026-07-20T13:35:00+08:00").unwrap(),
    )
    .unwrap();
    let source = LocalSourceRepository::new(root.path())
        .verify_current()
        .unwrap();
    assert_eq!(source.manifest().tick_record_count, EVENTS as u64);

    let prepare_started = Instant::now();
    let published = CacheBuilder::new(root.path())
        .build_current(normalizer)
        .unwrap();
    let prepare_elapsed = prepare_started.elapsed();
    assert_eq!(published.descriptor().event_count, EVENTS as u64);
    println!(
        "verified cache prepare (verify + decompress + normalize/sort + encode/write): {} records, {:.3}s, {:.0} records/s",
        EVENTS,
        prepare_elapsed.as_secs_f64(),
        EVENTS as f64 / prepare_elapsed.as_secs_f64(),
    );

    let mut scan_durations = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        let mut reader = CacheReader::open(published.path()).unwrap();
        let started = Instant::now();
        let mut count = 0_u64;
        while let Some(record) = reader.next_record().unwrap() {
            black_box(record.ordinal());
            count += 1;
        }
        let elapsed = started.elapsed();
        assert_eq!(count, EVENTS as u64);
        println!(
            "cache scan round {}: {:.3}s, {:.0} records/s",
            round + 1,
            elapsed.as_secs_f64(),
            count as f64 / elapsed.as_secs_f64(),
        );
        scan_durations.push(elapsed);
    }
    let scan_median = median_duration(scan_durations);
    println!("cache scan median: {:.3}s", scan_median.as_secs_f64());

    let mut replay_durations = Vec::with_capacity(ROUNDS);
    let mut expected_checksums = None;
    for round in 0..ROUNDS {
        let mut reader = CacheReader::open(published.path()).unwrap();
        let mut core = replay_core(&instrument, date);
        let started = Instant::now();
        core.replay_stream(&mut reader).unwrap();
        let completed = core.complete().unwrap();
        let elapsed = started.elapsed();
        let checksums = (
            *completed.summary().event_checksum().as_bytes(),
            *completed.summary().final_state_checksum().as_bytes(),
        );
        if let Some(expected) = expected_checksums {
            assert_eq!(checksums, expected, "cache replay must be deterministic");
        } else {
            expected_checksums = Some(checksums);
        }
        println!(
            "cache-backed replay round {}: {:.3}s, {:.0} events/s",
            round + 1,
            elapsed.as_secs_f64(),
            EVENTS as f64 / elapsed.as_secs_f64(),
        );
        replay_durations.push(elapsed);
    }
    let replay_median = median_duration(replay_durations);
    println!(
        "cache-backed replay median: {:.3}s",
        replay_median.as_secs_f64()
    );
}
