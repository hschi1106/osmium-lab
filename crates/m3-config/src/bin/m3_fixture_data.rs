use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
};

use data_sync::{
    ArchiveKind, ArchiveTimestamp, CacheBuilder, PartitionNormalizerConfig, StagingRevision,
    TeralionCredential, TeralionQuery, TeralionRequest, TeralionSync, TeralionTransport,
    TransportError,
};
use m3_config::{M3Config, load};
use market_types::{InstrumentId, InstrumentKind, MarketId};
use run_planner::SourcePartitionKey;
use strategy_api::SessionKind;

#[derive(Debug)]
struct FixtureTransport {
    pages: BTreeMap<InstrumentId, Vec<Vec<Vec<u8>>>>,
    daily: BTreeMap<InstrumentId, Vec<u8>>,
}

impl FixtureTransport {
    fn for_partition(
        fixture_root: &Path,
        key: &SourcePartitionKey,
    ) -> Result<Self, Box<dyn Error>> {
        let root = fixture_partition_root(fixture_root, key)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("fixture partition is missing: {}", root.display()),
            )
            .into());
        }

        let mut pages = Vec::new();
        for (session, directory) in session_directories(key) {
            if !key.session_kinds().contains(&session) {
                continue;
            }
            let path = root.join(directory);
            let mut files = fs::read_dir(&path)
                .map_err(|error| {
                    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
                })?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|value| value == "jsonl"))
                .collect::<Vec<_>>();
            files.sort();
            for file in files {
                pages.push(read_fixture_page(&file)?);
            }
        }
        if pages.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("fixture partition has no JSONL shards: {}", root.display()),
            )
            .into());
        }

        let daily_path = root.join("daily.json");
        let daily = fs::read(&daily_path).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", daily_path.display()))
        })?;
        let mut page_map = BTreeMap::new();
        page_map.insert(key.instrument().clone(), pages);
        let mut daily_map = BTreeMap::new();
        daily_map.insert(key.instrument().clone(), daily);
        Ok(Self {
            pages: page_map,
            daily: daily_map,
        })
    }

    fn page_body(pages: &[Vec<Vec<u8>>], cursor: Option<&str>) -> Result<Vec<u8>, TransportError> {
        let index = cursor
            .map(|value| {
                value
                    .strip_prefix("fixture-")
                    .ok_or_else(|| TransportError::new(false, "fixture cursor prefix"))?
                    .parse::<usize>()
                    .map_err(|_| TransportError::new(false, "fixture cursor index"))
            })
            .transpose()?
            .unwrap_or(0);
        let page = pages
            .get(index)
            .ok_or_else(|| TransportError::new(false, "fixture cursor out of range"))?;
        let next = (index + 1 < pages.len()).then(|| format!("fixture-{}", index + 1));
        let mut body = br#"{"items":["#.to_vec();
        for (offset, record) in page.iter().enumerate() {
            if offset > 0 {
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
        Ok(body)
    }
}

impl TeralionTransport for FixtureTransport {
    fn execute(
        &mut self,
        request: &TeralionRequest,
        _: &TeralionCredential,
    ) -> Result<Vec<u8>, TransportError> {
        let instrument = request
            .query()
            .instrument()
            .ok_or_else(|| TransportError::new(false, "fixture query has no instrument"))?;
        if request.query().is_paged() {
            let pages = self
                .pages
                .get(instrument)
                .ok_or_else(|| TransportError::new(false, "fixture pages are missing"))?;
            Self::page_body(pages, request.cursor())
        } else {
            self.daily
                .get(instrument)
                .cloned()
                .ok_or_else(|| TransportError::new(false, "fixture daily instrument is missing"))
        }
    }
}

fn read_fixture_page(path: &Path) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
    let file = File::open(path)?;
    let lines = BufReader::new(file)
        .lines()
        .map(|line| {
            let line = line?;
            serde_json::from_str::<serde_json::Value>(&line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            Ok(line.into_bytes())
        })
        .collect::<Result<Vec<_>, io::Error>>()?;
    Ok(lines)
}

fn fixture_partition_root(
    fixture_root: &Path,
    key: &SourcePartitionKey,
) -> Result<PathBuf, Box<dyn Error>> {
    let market = match key.instrument().market() {
        MarketId::Twse => "twse",
        MarketId::Tpex => "tpex",
        MarketId::Taifex => "taifex",
    };
    let root = fixture_root
        .join(market)
        .join(key.instrument().symbol().as_str())
        .join(key.trading_date().to_string());
    Ok(root)
}

fn session_directories(kind: &SourcePartitionKey) -> Vec<(SessionKind, &'static str)> {
    match kind.instrument().market() {
        MarketId::Twse => vec![(SessionKind::Regular, "regular-quotes")],
        MarketId::Tpex => vec![(SessionKind::Regular, "regular-quotes")],
        MarketId::Taifex => vec![
            (SessionKind::AfterHours, "after-hours"),
            (SessionKind::Regular, "regular"),
        ],
    }
}

fn kinds_for(instrument: &InstrumentId) -> &'static [ArchiveKind] {
    match instrument.market() {
        MarketId::Twse => &[ArchiveKind::Quote],
        MarketId::Tpex => &[ArchiveKind::Quote],
        MarketId::Taifex => &[
            ArchiveKind::Book,
            ArchiveKind::Close,
            ArchiveKind::Stats,
            ArchiveKind::Trade,
        ],
    }
}

fn replay_window(
    config: &M3Config,
    key: &SourcePartitionKey,
) -> Result<(ArchiveTimestamp, ArchiveTimestamp), Box<dyn Error>> {
    let plan = config.session_plan_for(key)?;
    let start = plan
        .windows()
        .iter()
        .map(|window| window.replay_start())
        .min()
        .ok_or("session plan has no replay windows")?;
    let end = plan
        .windows()
        .iter()
        .map(|window| window.replay_end_exclusive())
        .max()
        .ok_or("session plan has no replay windows")?;
    Ok((
        ArchiveTimestamp::parse(start.to_iso8601(480))?,
        ArchiveTimestamp::parse(end.to_iso8601(480))?,
    ))
}

fn prepare_partition(
    config: &M3Config,
    fixture_root: &Path,
    data_root: &Path,
    key: &SourcePartitionKey,
) -> Result<(), Box<dyn Error>> {
    let (start, end) = replay_window(config, key)?;
    let kind = config.instrument_kind_for(key.instrument());
    let query = match kind {
        InstrumentKind::Warrant | InstrumentKind::Option => TeralionQuery::ticks_for_market(
            key.instrument().clone(),
            start,
            end,
            kinds_for(key.instrument()).iter().copied(),
            5_000,
            config.archive_market_for(key.instrument()),
        )?,
        _ => TeralionQuery::ticks(
            key.instrument().clone(),
            start,
            end,
            kinds_for(key.instrument()).iter().copied(),
            5_000,
        )?,
    };
    let daily_query = TeralionQuery::daily_instrument(key.instrument().clone(), key.trading_date());
    let mut sync = TeralionSync::new(FixtureTransport::for_partition(fixture_root, key)?);
    let attempt = format!("fixture-{}", hex(key.identity().as_bytes()));
    let mut staging = StagingRevision::create_for_partition(data_root, key, &attempt)?;
    let report = sync.sync_pages(
        query.clone(),
        &TeralionCredential::new("fixture-only")?,
        &mut staging,
    )?;
    let daily = sync.fetch_single(
        daily_query.clone(),
        &TeralionCredential::new("fixture-only")?,
    )?;
    staging.stage_daily_instrument(daily_query.identity(), &daily)?;
    let published = staging.publish(query.identity(), report.terminal)?;
    let normalizer = match (key.instrument().market(), kind) {
        (MarketId::Twse, InstrumentKind::Warrant) => {
            let (start, end) = replay_window(config, key)?;
            PartitionNormalizerConfig::Warrant(twse_normalizer::NormalizerConfig::new_warrant(
                key.instrument().clone(),
                key.trading_date(),
                start.utc(),
                end.utc(),
            )?)
        }
        (MarketId::Twse, _) => {
            let (start, end) = replay_window(config, key)?;
            PartitionNormalizerConfig::Twse(twse_normalizer::NormalizerConfig::new(
                key.instrument().clone(),
                key.trading_date(),
                start.utc(),
                end.utc(),
            )?)
        }
        (MarketId::Tpex, _) => {
            let (start, end) = replay_window(config, key)?;
            PartitionNormalizerConfig::Tpex(tpex_normalizer::NormalizerConfig::new(
                key.instrument().clone(),
                key.trading_date(),
                start.utc(),
                end.utc(),
            )?)
        }
        (MarketId::Taifex, InstrumentKind::Option) => {
            let plan = config.session_plan_for(key)?;
            let windows = plan
                .windows()
                .iter()
                .map(|window| (window.replay_start(), window.replay_end_exclusive()));
            PartitionNormalizerConfig::TaifexOption(
                taifex_normalizer::NormalizerConfig::for_profile(
                    key.instrument().clone(),
                    key.trading_date(),
                    taifex_normalizer::InstrumentProfile::IndexOptions,
                    windows,
                )?,
            )
        }
        (MarketId::Taifex, _) => {
            let (start, end) = replay_window(config, key)?;
            PartitionNormalizerConfig::Taifex(taifex_normalizer::NormalizerConfig::new(
                key.instrument().clone(),
                key.trading_date(),
                start.utc(),
                end.utc(),
            )?)
        }
    };
    let cache = CacheBuilder::new(data_root).build_partition(key, normalizer)?;
    println!(
        "partition={:?}/{}@{} pages={} records={} source_revision={} cache_identity={}",
        key.instrument().market(),
        key.instrument().symbol(),
        key.trading_date(),
        report.page_count,
        published.manifest().tick_record_count,
        published.manifest().revision_identity,
        cache.descriptor().cache_identity,
    );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn argument(
    name: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<PathBuf, Box<dyn Error>> {
    let value = args.next().ok_or_else(|| format!("missing {name}"))?;
    if value.starts_with('-') {
        return Err(format!("missing {name}").into());
    }
    Ok(PathBuf::from(value))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let config_path = match args.next().as_deref() {
        Some("--config") => argument("--config value", &mut args)?,
        _ => {
            return Err(
                "usage: m3-fixture-data --config <file> --fixtures <dir> --data-root <dir>".into(),
            );
        }
    };
    let fixture_root = match args.next().as_deref() {
        Some("--fixtures") => argument("--fixtures value", &mut args)?,
        _ => {
            return Err(
                "usage: m3-fixture-data --config <file> --fixtures <dir> --data-root <dir>".into(),
            );
        }
    };
    let data_root = match args.next().as_deref() {
        Some("--data-root") => argument("--data-root value", &mut args)?,
        _ => {
            return Err(
                "usage: m3-fixture-data --config <file> --fixtures <dir> --data-root <dir>".into(),
            );
        }
    };
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    if data_root.exists() {
        return Err(format!("refusing to overwrite data root: {}", data_root.display()).into());
    }

    let config = load(config_path)?;
    for key in config.partition_keys()? {
        prepare_partition(&config, &fixture_root, &data_root, &key)?;
    }
    Ok(())
}
