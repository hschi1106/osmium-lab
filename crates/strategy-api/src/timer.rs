use std::{error::Error, fmt};

use market_types::{Decimal, MatchTime};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StrategyTimerId(Box<str>);

impl StrategyTimerId {
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, StrategyTimerError> {
        let value = value.into();
        if value.is_empty() {
            return Err(StrategyTimerError::EmptyId);
        }
        if value.len() > 128 {
            return Err(StrategyTimerError::IdTooLong);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyTimerRequest {
    timer_id: StrategyTimerId,
    fire_at: MatchTime,
}

impl StrategyTimerRequest {
    #[must_use]
    pub const fn new(timer_id: StrategyTimerId, fire_at: MatchTime) -> Self {
        Self { timer_id, fire_at }
    }

    #[must_use]
    pub const fn timer_id(&self) -> &StrategyTimerId {
        &self.timer_id
    }

    #[must_use]
    pub const fn fire_at(&self) -> MatchTime {
        self.fire_at
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StrategyTimerContext<'a> {
    timer_id: &'a StrategyTimerId,
    fire_at: MatchTime,
    current_equity: Decimal,
}

impl<'a> StrategyTimerContext<'a> {
    #[must_use]
    pub const fn new(
        timer_id: &'a StrategyTimerId,
        fire_at: MatchTime,
        current_equity: Decimal,
    ) -> Self {
        Self {
            timer_id,
            fire_at,
            current_equity,
        }
    }

    #[must_use]
    pub const fn timer_id(self) -> &'a StrategyTimerId {
        self.timer_id
    }

    #[must_use]
    pub const fn fire_at(self) -> MatchTime {
        self.fire_at
    }

    /// Returns mark-to-market portfolio equity from the runner's authoritative ledger.
    #[must_use]
    pub const fn current_equity(self) -> Decimal {
        self.current_equity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyTimerError {
    EmptyId,
    IdTooLong,
    CapabilityUnavailable,
    FireBeforeDecision,
}

impl fmt::Display for StrategyTimerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyId => "strategy timer identifier must not be empty",
            Self::IdTooLong => "strategy timer identifier exceeds 128 UTF-8 bytes",
            Self::CapabilityUnavailable => {
                "strategy timer capability is unavailable in this execution mode"
            }
            Self::FireBeforeDecision => "strategy timer is earlier than the current decision time",
        })
    }
}

impl Error for StrategyTimerError {}

#[cfg(test)]
mod tests {
    use crate::StrategyOutputSink;

    use super::*;

    #[test]
    fn timer_ids_and_capability_are_explicit() {
        assert_eq!(
            StrategyTimerId::new("").unwrap_err(),
            StrategyTimerError::EmptyId
        );
        let request = StrategyTimerRequest::new(
            StrategyTimerId::new("opening-decision").unwrap(),
            MatchTime::from_unix_microseconds(10),
        );
        assert!(
            StrategyOutputSink::new()
                .emit_timer(request.clone())
                .is_err()
        );
        let mut sink = StrategyOutputSink::with_scheduled_orders();
        sink.emit_timer(request.clone()).unwrap();
        assert_eq!(sink.timers(), &[request]);
    }
}
