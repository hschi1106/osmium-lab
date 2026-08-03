use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    fmt,
    path::{Path, PathBuf},
    time::Instant,
};

use data_sync::{CacheReader, LocalCacheFactory};
use market_state::MarketState;
use market_types::{
    DomainEvent, EventPayload, InstrumentId, MatchTime, Observation, Price, Quantity, TradeOrder,
    TradePrint,
};
use osmium_config::{RunConfig, load, plan};
use replay_engine::{EventStream, OrderingKey, ReplayCore, ReplayError, ReplayStreamFactory};

const MICROSECONDS_PER_SECOND: i64 = 1_000_000;
const VOLUME_BUCKET_MICROSECONDS: i64 = 60 * MICROSECONDS_PER_SECOND;
const SCALED_NANOS_DENOMINATOR: u128 = 1_000_000;

/// The fixed playback rates exposed by the market replay UI.
pub const PLAYBACK_SPEEDS_MILLI: [u16; 9] =
    [100, 250, 500, 1_000, 2_000, 5_000, 10_000, 25_000, 50_000];
const PLAYBACK_SPEED_LABELS: [&str; 9] = [
    "0.1x", "0.25x", "0.5x", "1.0x", "2.0x", "5.0x", "10.0x", "25.0x", "50.0x",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackSpeed {
    index: usize,
}

impl PlaybackSpeed {
    #[must_use]
    pub const fn normal() -> Self {
        Self { index: 3 }
    }

    #[must_use]
    pub const fn factor_milli(self) -> u16 {
        PLAYBACK_SPEEDS_MILLI[self.index]
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        PLAYBACK_SPEED_LABELS[self.index]
    }

    #[must_use]
    pub const fn faster(self) -> Self {
        Self {
            index: if self.index + 1 < PLAYBACK_SPEEDS_MILLI.len() {
                self.index + 1
            } else {
                self.index
            },
        }
    }

    #[must_use]
    pub const fn slower(self) -> Self {
        Self {
            index: if self.index > 0 { self.index - 1 } else { 0 },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeRow {
    match_time: MatchTime,
    price: Price,
    quantity: Quantity,
}

impl TradeRow {
    #[must_use]
    pub const fn match_time(self) -> MatchTime {
        self.match_time
    }

    #[must_use]
    pub const fn price(self) -> Price {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> Quantity {
        self.quantity
    }
}

#[derive(Debug, Default)]
pub struct ReplayHistory {
    price_points: Vec<(f64, f64)>,
    volume_points: Vec<(f64, f64)>,
    last_volume_bucket: Option<(i64, u64)>,
    trades: VecDeque<TradeRow>,
    latest_price: Option<Price>,
    maximum_volume: u64,
}

impl ReplayHistory {
    #[must_use]
    pub fn price_points(&self) -> &[(f64, f64)] {
        &self.price_points
    }

    #[must_use]
    pub fn volume_points(&self) -> &[(f64, f64)] {
        &self.volume_points
    }

    #[must_use]
    pub fn trades(&self) -> &VecDeque<TradeRow> {
        &self.trades
    }

    #[must_use]
    pub const fn latest_price(&self) -> Option<Price> {
        self.latest_price
    }

    #[must_use]
    pub const fn maximum_volume(&self) -> u64 {
        self.maximum_volume
    }
}

#[derive(Debug)]
struct ReplayRuntime {
    core: ReplayCore,
    streams: Vec<CacheReader>,
    heads: Vec<Option<DomainEvent>>,
}

struct PreparedReplay {
    runtime: ReplayRuntime,
    instruments: Vec<InstrumentId>,
    replay_start: MatchTime,
    replay_end: MatchTime,
}

/// Playback state shared by the TUI and its input handlers.
///
/// This type owns the frozen cache streams and advances one shared `match_time`
/// for every selected instrument. The UI only asks it for a read-only view.
#[derive(Debug)]
pub struct MarketReplay {
    config_path: PathBuf,
    runtime: ReplayRuntime,
    instruments: Vec<InstrumentId>,
    histories: BTreeMap<InstrumentId, ReplayHistory>,
    selected_index: usize,
    replay_start: MatchTime,
    replay_end: MatchTime,
    current_time: MatchTime,
    status: PlaybackStatus,
    speed: PlaybackSpeed,
    last_wall_time: Instant,
    scaled_remainder: u128,
}

impl MarketReplay {
    pub fn from_config(path: impl AsRef<Path>) -> Result<Self, MarketReplayError> {
        let path = path.as_ref().to_path_buf();
        let prepared = prepare_replay(&path)?;
        let histories = prepared
            .instruments
            .iter()
            .cloned()
            .map(|instrument| (instrument, ReplayHistory::default()))
            .collect();
        Ok(Self {
            config_path: path,
            runtime: prepared.runtime,
            instruments: prepared.instruments,
            histories,
            selected_index: 0,
            replay_start: prepared.replay_start,
            replay_end: prepared.replay_end,
            current_time: prepared.replay_start,
            status: PlaybackStatus::Playing,
            speed: PlaybackSpeed::normal(),
            last_wall_time: Instant::now(),
            scaled_remainder: 0,
        })
    }

    #[must_use]
    pub fn instruments(&self) -> &[InstrumentId] {
        &self.instruments
    }

    #[must_use]
    pub const fn selected_index(&self) -> usize {
        self.selected_index
    }

    #[must_use]
    pub fn selected_instrument(&self) -> &InstrumentId {
        &self.instruments[self.selected_index]
    }

    #[must_use]
    pub const fn replay_start(&self) -> MatchTime {
        self.replay_start
    }

    #[must_use]
    pub const fn replay_end(&self) -> MatchTime {
        self.replay_end
    }

    #[must_use]
    pub const fn current_time(&self) -> MatchTime {
        self.current_time
    }

    #[must_use]
    pub const fn status(&self) -> PlaybackStatus {
        self.status
    }

    #[must_use]
    pub const fn speed(&self) -> PlaybackSpeed {
        self.speed
    }

    #[must_use]
    pub fn selected_state(&self) -> Option<&MarketState> {
        self.state(self.selected_instrument())
    }

    #[must_use]
    pub fn state(&self, instrument: &InstrumentId) -> Option<&MarketState> {
        self.runtime.core.state(instrument)
    }

    #[must_use]
    pub fn history(&self, instrument: &InstrumentId) -> Option<&ReplayHistory> {
        self.histories.get(instrument)
    }

    #[must_use]
    pub fn selected_history(&self) -> &ReplayHistory {
        self.histories
            .get(self.selected_instrument())
            .expect("history is created for every replay instrument")
    }

    pub fn select_previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        } else {
            self.selected_index = self.instruments.len() - 1;
        }
    }

    pub fn select_next(&mut self) {
        self.selected_index = (self.selected_index + 1) % self.instruments.len();
    }

    pub fn toggle_at(&mut self, now: Instant) {
        self.last_wall_time = now;
        self.status = match self.status {
            PlaybackStatus::Playing => PlaybackStatus::Paused,
            PlaybackStatus::Paused => PlaybackStatus::Playing,
            PlaybackStatus::Finished => PlaybackStatus::Finished,
        };
    }

    pub fn faster(&mut self) {
        self.speed = self.speed.faster();
    }

    pub fn slower(&mut self) {
        self.speed = self.speed.slower();
    }

    pub fn reset(&mut self, now: Instant) -> Result<(), MarketReplayError> {
        let prepared = prepare_replay(&self.config_path)?;
        self.runtime = prepared.runtime;
        self.instruments = prepared.instruments;
        self.histories = self
            .instruments
            .iter()
            .cloned()
            .map(|instrument| (instrument, ReplayHistory::default()))
            .collect();
        self.selected_index = 0;
        self.replay_start = prepared.replay_start;
        self.replay_end = prepared.replay_end;
        self.current_time = prepared.replay_start;
        self.status = PlaybackStatus::Playing;
        self.speed = PlaybackSpeed::normal();
        self.last_wall_time = now;
        self.scaled_remainder = 0;
        Ok(())
    }

    /// Advances the common replay clock according to elapsed wall time.
    pub fn tick(&mut self, now: Instant) -> Result<usize, MarketReplayError> {
        let elapsed = now.saturating_duration_since(self.last_wall_time);
        self.last_wall_time = now;
        if self.status != PlaybackStatus::Playing {
            return Ok(0);
        }

        let scaled_nanos = elapsed
            .as_nanos()
            .saturating_mul(u128::from(self.speed.factor_milli()))
            .saturating_add(self.scaled_remainder);
        let replay_micros = scaled_nanos / SCALED_NANOS_DENOMINATOR;
        self.scaled_remainder = scaled_nanos % SCALED_NANOS_DENOMINATOR;
        let delta = i64::try_from(replay_micros).unwrap_or(i64::MAX);
        let target = MatchTime::from_unix_microseconds(
            self.current_time
                .as_unix_microseconds()
                .saturating_add(delta),
        );
        self.advance_to(target)
    }

    /// Advances to an explicit replay time. This is useful for deterministic tests
    /// and does not change the selected symbol or playback rate.
    pub fn advance_to(&mut self, target: MatchTime) -> Result<usize, MarketReplayError> {
        let target = target.max(self.current_time).min(self.replay_end);
        let mut applied = 0;
        while let Some(index) = self.next_due_stream(target)? {
            let event = self.runtime.heads[index]
                .take()
                .expect("selected stream head is present");
            self.runtime
                .core
                .apply_ordered(&event)
                .map_err(MarketReplayError::Replay)?;
            self.record_event(&event)?;
            applied += 1;
        }
        self.current_time = target;
        if target >= self.replay_end && self.all_streams_consumed()? {
            self.status = PlaybackStatus::Finished;
        }
        Ok(applied)
    }

    fn next_due_stream(&mut self, target: MatchTime) -> Result<Option<usize>, MarketReplayError> {
        self.fill_heads()?;
        let mut selected: Option<(usize, OrderingKey)> = None;
        for (index, event) in self.runtime.heads.iter().enumerate() {
            let Some(event) = event else { continue };
            if event.match_time() > target {
                continue;
            }
            let key = OrderingKey::for_event(event).map_err(MarketReplayError::Ordering)?;
            if selected
                .as_ref()
                .is_none_or(|(_, previous)| key < *previous)
            {
                selected = Some((index, key));
            }
        }
        Ok(selected.map(|(index, _)| index))
    }

    fn fill_heads(&mut self) -> Result<(), MarketReplayError> {
        for (index, stream) in self.runtime.streams.iter_mut().enumerate() {
            if self.runtime.heads[index].is_none() {
                self.runtime.heads[index] =
                    stream.next_event().map_err(MarketReplayError::CacheRead)?;
            }
        }
        Ok(())
    }

    fn all_streams_consumed(&mut self) -> Result<bool, MarketReplayError> {
        self.fill_heads()?;
        Ok(self.runtime.heads.iter().all(Option::is_none))
    }

    fn record_event(&mut self, event: &DomainEvent) -> Result<(), MarketReplayError> {
        let history = self.histories.get_mut(event.instrument()).ok_or_else(|| {
            MarketReplayError::Other("event instrument is outside replay history".into())
        })?;
        match event.payload() {
            EventPayload::QuoteSnapshot(snapshot) => {
                if let Observation::Set(trade) = snapshot.trade() {
                    record_trades(
                        history,
                        self.replay_start,
                        event.match_time(),
                        std::slice::from_ref(trade),
                        true,
                    )?;
                }
            }
            EventPayload::TradeBatch(batch) => {
                record_trades(
                    history,
                    self.replay_start,
                    event.match_time(),
                    batch.trades(),
                    batch.trade_order() == TradeOrder::SourceOrdered,
                )?;
            }
            EventPayload::BookSnapshot(_)
            | EventPayload::IndicativeOpeningAuction(_)
            | EventPayload::IndicativeClosingAuction(_) => {}
        }
        Ok(())
    }
}

fn record_trades(
    history: &mut ReplayHistory,
    replay_start: MatchTime,
    match_time: MatchTime,
    trades: &[TradePrint],
    source_ordered: bool,
) -> Result<(), MarketReplayError> {
    if trades.is_empty() {
        return Ok(());
    }
    let bucket = match_time
        .as_unix_microseconds()
        .div_euclid(VOLUME_BUCKET_MICROSECONDS);
    let bucket_start = bucket.saturating_mul(VOLUME_BUCKET_MICROSECONDS);
    let x = bucket_start.saturating_sub(replay_start.as_unix_microseconds()) as f64
        / MICROSECONDS_PER_SECOND as f64;
    let mut volume = 0_u64;
    for trade in trades {
        volume = volume
            .checked_add(trade.quantity().value())
            .ok_or_else(|| MarketReplayError::Other("trade volume overflow".into()))?;
        history.price_points.push((x, price_as_f64(trade.price())));
    }
    let bucket_volume = match history.last_volume_bucket {
        Some((previous_bucket, previous_volume)) if previous_bucket == bucket => previous_volume
            .checked_add(volume)
            .ok_or_else(|| MarketReplayError::Other("trade volume overflow".into()))?,
        _ => volume,
    };
    if history
        .last_volume_bucket
        .is_some_and(|(previous_bucket, _)| previous_bucket == bucket)
    {
        let point = history
            .volume_points
            .last_mut()
            .expect("volume bucket has a corresponding chart point");
        point.1 = bucket_volume as f64;
    } else {
        history.volume_points.push((x, bucket_volume as f64));
    }
    history.last_volume_bucket = Some((bucket, bucket_volume));
    history.maximum_volume = history.maximum_volume.max(bucket_volume);
    if source_ordered {
        history.latest_price = trades.last().map(|trade| trade.price());
    }
    for trade in trades.iter().rev() {
        history.trades.push_front(TradeRow {
            match_time,
            price: trade.price(),
            quantity: trade.quantity(),
        });
    }
    const MAX_VISIBLE_TRADES: usize = 200;
    history.trades.truncate(MAX_VISIBLE_TRADES);
    Ok(())
}

fn price_as_f64(price: Price) -> f64 {
    price.atoms() as f64 / market_types::Decimal::SCALE_FACTOR as f64
}

fn prepare_replay(path: &Path) -> Result<PreparedReplay, MarketReplayError> {
    let config = load(path).map_err(|error| MarketReplayError::Preparation {
        message: format!("market replay config failed: {error}").into_boxed_str(),
        exit_code: 2,
    })?;
    if config.effective().trading_dates().len() != 1 {
        return Err(MarketReplayError::Preparation {
            message: "market replay currently requires exactly one trading date".into(),
            exit_code: 2,
        });
    }
    let bundle = plan(config.clone()).map_err(|error| MarketReplayError::Preparation {
        message: format!("market replay plan failed: {error}").into_boxed_str(),
        exit_code: 20,
    })?;
    let replay_plan = bundle
        .replay
        .as_ref()
        .ok_or(MarketReplayError::CacheMissing)?;
    let (replay_start, replay_end) = replay_bounds(&config)?;
    let core = crate::command::replay_core(&config, &bundle).map_err(|error| {
        MarketReplayError::Preparation {
            message: format!("market replay state setup failed: {error}").into_boxed_str(),
            exit_code: error.exit_code(),
        }
    })?;
    let mut factory = LocalCacheFactory::new_partitioned(config.effective().data_root());
    let mut streams = Vec::with_capacity(replay_plan.bindings().len());
    for binding in replay_plan.bindings() {
        streams.push(
            factory
                .open(binding)
                .map_err(MarketReplayError::CacheRead)?,
        );
    }
    Ok(finish_prepared(
        core,
        streams,
        replay_plan,
        replay_start,
        replay_end,
    ))
}

fn finish_prepared(
    core: ReplayCore,
    streams: Vec<CacheReader>,
    replay_plan: &replay_engine::ReplayPlan,
    replay_start: MatchTime,
    replay_end: MatchTime,
) -> PreparedReplay {
    let instruments = replay_plan
        .bindings()
        .iter()
        .map(|binding| binding.instrument().clone())
        .collect::<Vec<_>>();
    PreparedReplay {
        runtime: ReplayRuntime {
            core,
            heads: (0..streams.len()).map(|_| None).collect(),
            streams,
        },
        instruments,
        replay_start,
        replay_end,
    }
}

fn replay_bounds(config: &RunConfig) -> Result<(MatchTime, MatchTime), MarketReplayError> {
    let keys = config
        .partition_keys()
        .map_err(|error| MarketReplayError::Preparation {
            message: format!("market replay session setup failed: {error}").into_boxed_str(),
            exit_code: 2,
        })?;
    let mut start = None;
    let mut end = None;
    for key in &keys {
        let session =
            config
                .session_plan_for(key)
                .map_err(|error| MarketReplayError::Preparation {
                    message: format!("market replay session setup failed: {error}")
                        .into_boxed_str(),
                    exit_code: 2,
                })?;
        for window in session.windows() {
            start = Some(start.map_or(window.replay_start(), |value: MatchTime| {
                value.min(window.replay_start())
            }));
            end = Some(
                end.map_or(window.replay_end_exclusive(), |value: MatchTime| {
                    value.max(window.replay_end_exclusive())
                }),
            );
        }
    }
    match (start, end) {
        (Some(start), Some(end)) if start < end => Ok((start, end)),
        _ => Err(MarketReplayError::Preparation {
            message: "market replay has no valid session window".into(),
            exit_code: 2,
        }),
    }
}

#[derive(Debug)]
pub enum MarketReplayError {
    Preparation { message: Box<str>, exit_code: u8 },
    CacheMissing,
    CacheRead(data_sync::CacheReadError),
    Ordering(replay_engine::OrderingError),
    Replay(ReplayError),
    Io(std::io::Error),
    Other(Box<str>),
}

impl MarketReplayError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Preparation { exit_code, .. } => *exit_code,
            Self::CacheMissing | Self::CacheRead(_) => 20,
            Self::Ordering(_) | Self::Replay(_) => 50,
            Self::Io(_) | Self::Other(_) => 1,
        }
    }
}

impl fmt::Display for MarketReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preparation { message, .. } => formatter.write_str(message),
            Self::CacheMissing => formatter.write_str(
                "market replay requires a complete source and valid cache; run cache prepare first",
            ),
            Self::CacheRead(error) => write!(formatter, "market replay cache read failed: {error}"),
            Self::Ordering(error) => write!(formatter, "market replay ordering failed: {error}"),
            Self::Replay(error) => write!(formatter, "market replay state update failed: {error}"),
            Self::Io(error) => write!(formatter, "market replay terminal I/O failed: {error}"),
            Self::Other(message) => formatter.write_str(message),
        }
    }
}

impl Error for MarketReplayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CacheRead(error) => Some(error),
            Self::Ordering(error) => Some(error),
            Self::Replay(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Preparation { .. } | Self::CacheMissing | Self::Other(_) => None,
        }
    }
}

impl From<std::io::Error> for MarketReplayError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_types::{QuantityUnit, TradePrintKind};

    #[test]
    fn playback_speed_uses_the_fixed_rates_and_saturates() {
        let mut speed = PlaybackSpeed::normal();
        assert_eq!(speed.label(), "1.0x");
        assert_eq!(speed.factor_milli(), 1_000);
        for _ in 0..20 {
            speed = speed.faster();
        }
        assert_eq!(speed.label(), "50.0x");
        for _ in 0..20 {
            speed = speed.slower();
        }
        assert_eq!(speed.label(), "0.1x");
    }

    #[test]
    fn playback_speed_order_matches_the_user_contract() {
        let mut speed = PlaybackSpeed::slower(PlaybackSpeed::normal());
        let mut labels = vec![speed.label()];
        for _ in 0..7 {
            speed = speed.faster();
            labels.push(speed.label());
        }
        assert_eq!(
            labels,
            [
                "0.5x", "1.0x", "2.0x", "5.0x", "10.0x", "25.0x", "50.0x", "50.0x"
            ]
        );
    }

    #[test]
    fn volume_points_are_aggregated_into_one_minute_buckets() {
        let replay_start = MatchTime::parse("2026-07-20T08:40:00+08:00").unwrap();
        let mut history = ReplayHistory::default();
        let trade = |quantity| {
            TradePrint::new(
                Price::parse("100").unwrap(),
                Quantity::new(quantity, QuantityUnit::TradingUnit).unwrap(),
                TradePrintKind::Regular,
            )
        };

        record_trades(
            &mut history,
            replay_start,
            MatchTime::parse("2026-07-20T09:00:10+08:00").unwrap(),
            &[trade(2)],
            true,
        )
        .unwrap();
        record_trades(
            &mut history,
            replay_start,
            MatchTime::parse("2026-07-20T09:00:50+08:00").unwrap(),
            &[trade(3)],
            true,
        )
        .unwrap();
        record_trades(
            &mut history,
            replay_start,
            MatchTime::parse("2026-07-20T09:01:00+08:00").unwrap(),
            &[trade(4)],
            true,
        )
        .unwrap();

        assert_eq!(history.volume_points(), &[(1_200.0, 5.0), (1_260.0, 4.0)]);
        assert_eq!(history.maximum_volume(), 5);
    }
}
