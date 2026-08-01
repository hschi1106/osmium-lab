use std::{error::Error, fmt};

use market_types::{InstrumentId, InstrumentKind, MarketId, MatchTime, TradingDate};
use strategy_api::SessionKind;

use crate::partition::SessionPlanIdentity;

/// Version of the exchange-calendar rules used by the built-in M3 profiles.
pub const SESSION_CALENDAR_VERSION: u16 = 1;
/// Version of the profile definitions (instrument-to-session mapping).
pub const SESSION_PROFILE_VERSION: u16 = 1;
/// Version of the fixed replay-window margin policy.
pub const SESSION_WINDOW_POLICY_VERSION: u16 = 1;

const REPLAY_MARGIN_MICROSECONDS: i64 = 5 * 60 * 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum SessionProfileId {
    TwseRegular = 1,
    TaifexIndexFutures = 2,
    TaifexStockFutures = 3,
    TaifexStockFuturesRegularOnly = 4,
    TpexRegular = 5,
    TaifexIndexOptions = 6,
}

impl SessionProfileId {
    /// Resolves the profile for the M3 acceptance instruments.
    pub fn for_instrument(instrument: &InstrumentId) -> Result<Self, SessionPlanError> {
        let kind = match instrument.market() {
            MarketId::Twse | MarketId::Tpex => InstrumentKind::Equity,
            MarketId::Taifex => InstrumentKind::Future,
        };
        Self::for_instrument_kind(instrument, kind)
    }

    pub fn for_instrument_kind(
        instrument: &InstrumentId,
        kind: InstrumentKind,
    ) -> Result<Self, SessionPlanError> {
        match instrument.market() {
            MarketId::Twse => match kind {
                InstrumentKind::Equity | InstrumentKind::Warrant => Ok(Self::TwseRegular),
                _ => Err(SessionPlanError::UnsupportedInstrument(instrument.clone())),
            },
            MarketId::Tpex => match kind {
                InstrumentKind::Equity | InstrumentKind::Warrant => Ok(Self::TpexRegular),
                _ => Err(SessionPlanError::UnsupportedInstrument(instrument.clone())),
            },
            MarketId::Taifex => match kind {
                InstrumentKind::Option => Ok(Self::TaifexIndexOptions),
                InstrumentKind::Future => match instrument.symbol().as_str() {
                    "TXFH6" => Ok(Self::TaifexIndexFutures),
                    "CDFH6" => Ok(Self::TaifexStockFutures),
                    "CAFH6" => Ok(Self::TaifexStockFuturesRegularOnly),
                    _ => Err(SessionPlanError::UnsupportedInstrument(instrument.clone())),
                },
                _ => Err(SessionPlanError::UnsupportedInstrument(instrument.clone())),
            },
        }
    }

    #[must_use]
    pub const fn allows(self, kind: SessionKind) -> bool {
        match self {
            Self::TwseRegular | Self::TpexRegular | Self::TaifexStockFuturesRegularOnly => {
                matches!(kind, SessionKind::Regular)
            }
            Self::TaifexIndexFutures | Self::TaifexStockFutures | Self::TaifexIndexOptions => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionWindow {
    kind: SessionKind,
    trading_date: TradingDate,
    open: MatchTime,
    close: MatchTime,
    replay_start: MatchTime,
    replay_end_exclusive: MatchTime,
}

impl SessionWindow {
    #[must_use]
    pub const fn kind(&self) -> SessionKind {
        self.kind
    }

    #[must_use]
    pub const fn trading_date(&self) -> TradingDate {
        self.trading_date
    }

    #[must_use]
    pub const fn open(&self) -> MatchTime {
        self.open
    }

    #[must_use]
    pub const fn close(&self) -> MatchTime {
        self.close
    }

    #[must_use]
    pub const fn replay_start(&self) -> MatchTime {
        self.replay_start
    }

    #[must_use]
    pub const fn replay_end_exclusive(&self) -> MatchTime {
        self.replay_end_exclusive
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlan {
    instrument: InstrumentId,
    trading_date: TradingDate,
    profile: SessionProfileId,
    calendar_version: u16,
    profile_version: u16,
    window_policy_version: u16,
    windows: Box<[SessionWindow]>,
    identity: SessionPlanIdentity,
}

impl SessionPlan {
    pub fn for_instrument(
        instrument: &InstrumentId,
        trading_date: TradingDate,
        session_kinds: impl IntoIterator<Item = SessionKind>,
    ) -> Result<Self, SessionPlanError> {
        let profile = SessionProfileId::for_instrument(instrument)?;
        Self::with_profile(instrument, trading_date, profile, session_kinds)
    }

    pub fn for_instrument_kind(
        instrument: &InstrumentId,
        kind: InstrumentKind,
        trading_date: TradingDate,
        session_kinds: impl IntoIterator<Item = SessionKind>,
    ) -> Result<Self, SessionPlanError> {
        let profile = SessionProfileId::for_instrument_kind(instrument, kind)?;
        Self::with_profile(instrument, trading_date, profile, session_kinds)
    }

    pub fn with_profile(
        instrument: &InstrumentId,
        trading_date: TradingDate,
        profile: SessionProfileId,
        session_kinds: impl IntoIterator<Item = SessionKind>,
    ) -> Result<Self, SessionPlanError> {
        if is_weekend(trading_date) {
            return Err(SessionPlanError::NonBusinessTradingDate(trading_date));
        }

        let mut kinds = session_kinds.into_iter().collect::<Vec<_>>();
        kinds.sort_unstable();
        kinds.dedup();
        if kinds.is_empty() {
            return Err(SessionPlanError::EmptySessions);
        }
        if let Some(kind) = kinds.iter().copied().find(|kind| !profile.allows(*kind)) {
            return Err(SessionPlanError::SessionNotSupported { profile, kind });
        }

        let mut windows = Vec::with_capacity(kinds.len());
        for kind in kinds {
            windows.push(build_window(profile, kind, trading_date)?);
        }

        let identity = SessionPlanIdentity::from_bytes(
            *blake3::hash(&canonical_bytes(
                instrument,
                trading_date,
                profile,
                &windows,
            )?)
            .as_bytes(),
        );
        Ok(Self {
            instrument: instrument.clone(),
            trading_date,
            profile,
            calendar_version: SESSION_CALENDAR_VERSION,
            profile_version: SESSION_PROFILE_VERSION,
            window_policy_version: SESSION_WINDOW_POLICY_VERSION,
            windows: windows.into_boxed_slice(),
            identity,
        })
    }

    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn trading_date(&self) -> TradingDate {
        self.trading_date
    }

    #[must_use]
    pub const fn profile(&self) -> SessionProfileId {
        self.profile
    }

    #[must_use]
    pub const fn calendar_version(&self) -> u16 {
        self.calendar_version
    }

    #[must_use]
    pub const fn profile_version(&self) -> u16 {
        self.profile_version
    }

    #[must_use]
    pub const fn window_policy_version(&self) -> u16 {
        self.window_policy_version
    }

    #[must_use]
    pub const fn windows(&self) -> &[SessionWindow] {
        &self.windows
    }

    #[must_use]
    pub const fn identity(&self) -> SessionPlanIdentity {
        self.identity
    }

    #[must_use]
    pub fn window(&self, kind: SessionKind) -> Option<&SessionWindow> {
        self.windows.iter().find(|window| window.kind == kind)
    }
}

fn build_window(
    profile: SessionProfileId,
    kind: SessionKind,
    trading_date: TradingDate,
) -> Result<SessionWindow, SessionPlanError> {
    let (open_date, open_time, close_date, close_time) = match (profile, kind) {
        (SessionProfileId::TwseRegular | SessionProfileId::TpexRegular, SessionKind::Regular) => {
            (trading_date, "09:00:00", trading_date, "13:30:00")
        }
        (
            SessionProfileId::TaifexIndexFutures | SessionProfileId::TaifexIndexOptions,
            SessionKind::Regular,
        )
        | (SessionProfileId::TaifexStockFutures, SessionKind::Regular)
        | (SessionProfileId::TaifexStockFuturesRegularOnly, SessionKind::Regular) => {
            (trading_date, "08:45:00", trading_date, "13:45:00")
        }
        (
            SessionProfileId::TaifexIndexFutures | SessionProfileId::TaifexIndexOptions,
            SessionKind::AfterHours,
        ) => (
            previous_business_date(trading_date)?,
            "15:00:00",
            trading_date,
            "05:00:00",
        ),
        (SessionProfileId::TaifexStockFutures, SessionKind::AfterHours) => (
            previous_business_date(trading_date)?,
            "17:25:00",
            trading_date,
            "05:00:00",
        ),
        (_, _) => {
            return Err(SessionPlanError::SessionNotSupported { profile, kind });
        }
    };

    let open = parse_local_time(open_date, open_time)?;
    let close = parse_local_time(close_date, close_time)?;
    let replay_start = MatchTime::from_unix_microseconds(
        open.as_unix_microseconds()
            .checked_sub(REPLAY_MARGIN_MICROSECONDS)
            .ok_or(SessionPlanError::WindowOverflow)?,
    );
    let replay_end_exclusive = MatchTime::from_unix_microseconds(
        close
            .as_unix_microseconds()
            .checked_add(REPLAY_MARGIN_MICROSECONDS)
            .ok_or(SessionPlanError::WindowOverflow)?,
    );
    Ok(SessionWindow {
        kind,
        trading_date,
        open,
        close,
        replay_start,
        replay_end_exclusive,
    })
}

fn parse_local_time(date: TradingDate, time: &str) -> Result<MatchTime, SessionPlanError> {
    MatchTime::parse(&format!("{date}T{time}+08:00"))
        .map_err(|error| SessionPlanError::InvalidTimestamp(error.to_string()))
}

fn previous_business_date(date: TradingDate) -> Result<TradingDate, SessionPlanError> {
    let mut epoch_days = date
        .as_epoch_days()
        .checked_sub(1)
        .ok_or(SessionPlanError::CalendarOverflow)?;
    while is_weekend(TradingDate::from_epoch_days(epoch_days)) {
        epoch_days = epoch_days
            .checked_sub(1)
            .ok_or(SessionPlanError::CalendarOverflow)?;
    }
    Ok(TradingDate::from_epoch_days(epoch_days))
}

fn is_weekend(date: TradingDate) -> bool {
    // 1970-01-01 was Thursday (4 when Sunday is 0).
    matches!((date.as_epoch_days() + 4).rem_euclid(7), 0 | 6)
}

fn canonical_bytes(
    instrument: &InstrumentId,
    trading_date: TradingDate,
    profile: SessionProfileId,
    windows: &[SessionWindow],
) -> Result<Vec<u8>, SessionPlanError> {
    let mut output = Vec::new();
    output.extend_from_slice(b"OSSESSION01");
    output.extend_from_slice(&SESSION_CALENDAR_VERSION.to_be_bytes());
    output.extend_from_slice(&SESSION_PROFILE_VERSION.to_be_bytes());
    output.extend_from_slice(&SESSION_WINDOW_POLICY_VERSION.to_be_bytes());
    output.push(instrument.market().discriminant());
    append_bytes(instrument.symbol().as_bytes(), &mut output)?;
    output.extend_from_slice(&trading_date.to_canonical_bytes());
    output.push(profile as u8);
    append_len(windows.len(), &mut output)?;
    for window in windows {
        output.push(window.kind as u8);
        output.extend_from_slice(&window.open.as_unix_microseconds().to_be_bytes());
        output.extend_from_slice(&window.close.as_unix_microseconds().to_be_bytes());
        output.extend_from_slice(&window.replay_start.as_unix_microseconds().to_be_bytes());
        output.extend_from_slice(
            &window
                .replay_end_exclusive
                .as_unix_microseconds()
                .to_be_bytes(),
        );
    }
    Ok(output)
}

fn append_len(length: usize, output: &mut Vec<u8>) -> Result<(), SessionPlanError> {
    output.extend_from_slice(
        &u32::try_from(length)
            .map_err(|_| SessionPlanError::CanonicalLengthOverflow)?
            .to_be_bytes(),
    );
    Ok(())
}

fn append_bytes(value: &[u8], output: &mut Vec<u8>) -> Result<(), SessionPlanError> {
    append_len(value.len(), output)?;
    output.extend_from_slice(value);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPlanError {
    EmptySessions,
    NonBusinessTradingDate(TradingDate),
    UnsupportedMarket(MarketId),
    UnsupportedInstrument(InstrumentId),
    SessionNotSupported {
        profile: SessionProfileId,
        kind: SessionKind,
    },
    InvalidTimestamp(String),
    CalendarOverflow,
    WindowOverflow,
    CanonicalLengthOverflow,
}

impl fmt::Display for SessionPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySessions => formatter.write_str("session plan requires a session kind"),
            Self::NonBusinessTradingDate(date) => {
                write!(formatter, "trading date is not a business date: {date}")
            }
            Self::UnsupportedMarket(market) => write!(formatter, "unsupported market: {market:?}"),
            Self::UnsupportedInstrument(instrument) => {
                write!(formatter, "unsupported instrument profile: {instrument:?}")
            }
            Self::SessionNotSupported { profile, kind } => {
                write!(
                    formatter,
                    "session {kind:?} is not supported by profile {profile:?}"
                )
            }
            Self::InvalidTimestamp(error) => {
                write!(formatter, "invalid session timestamp: {error}")
            }
            Self::CalendarOverflow => formatter.write_str("calendar date arithmetic overflowed"),
            Self::WindowOverflow => {
                formatter.write_str("session replay window arithmetic overflowed")
            }
            Self::CanonicalLengthOverflow => {
                formatter.write_str("session plan canonical field exceeds u32 length")
            }
        }
    }
}

impl Error for SessionPlanError {}

#[cfg(test)]
mod tests {
    use market_types::{InstrumentId, MarketId, Symbol, TradingDate};

    use super::*;

    fn instrument(market: MarketId, symbol: &str) -> InstrumentId {
        InstrumentId::new(market, Symbol::new(symbol).unwrap())
    }

    fn date(value: &str) -> TradingDate {
        value.parse().unwrap()
    }

    #[test]
    fn index_futures_after_hours_crosses_previous_business_date() {
        let plan = SessionPlan::for_instrument(
            &instrument(MarketId::Taifex, "TXFH6"),
            date("2026-07-20"),
            [SessionKind::AfterHours, SessionKind::Regular],
        )
        .unwrap();
        let after_hours = plan.window(SessionKind::AfterHours).unwrap();
        assert_eq!(
            after_hours.open(),
            MatchTime::parse("2026-07-17T15:00:00+08:00").unwrap()
        );
        assert_eq!(
            after_hours.close(),
            MatchTime::parse("2026-07-20T05:00:00+08:00").unwrap()
        );
        assert_eq!(
            after_hours.replay_start(),
            MatchTime::parse("2026-07-17T14:55:00+08:00").unwrap()
        );
        assert_eq!(
            after_hours.replay_end_exclusive(),
            MatchTime::parse("2026-07-20T05:05:00+08:00").unwrap()
        );
        assert_eq!(plan.windows().len(), 2);
    }

    #[test]
    fn regular_only_stock_future_rejects_after_hours() {
        let result = SessionPlan::for_instrument(
            &instrument(MarketId::Taifex, "CAFH6"),
            date("2026-07-20"),
            [SessionKind::AfterHours],
        );
        assert!(matches!(
            result,
            Err(SessionPlanError::SessionNotSupported {
                profile: SessionProfileId::TaifexStockFuturesRegularOnly,
                kind: SessionKind::AfterHours
            })
        ));
    }

    #[test]
    fn requested_session_order_does_not_change_identity() {
        let instrument = instrument(MarketId::Twse, "2330");
        let first =
            SessionPlan::for_instrument(&instrument, date("2026-07-27"), [SessionKind::Regular])
                .unwrap();
        let second = SessionPlan::with_profile(
            &instrument,
            date("2026-07-27"),
            SessionProfileId::TwseRegular,
            [SessionKind::Regular, SessionKind::Regular],
        )
        .unwrap();
        assert_eq!(first.identity(), second.identity());
        assert_eq!(
            first.window(SessionKind::Regular).unwrap().open(),
            MatchTime::parse("2026-07-27T09:00:00+08:00").unwrap()
        );
    }
}
