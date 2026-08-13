use std::{error::Error, fmt};

use market_types::MatchTime;

use crate::{ControlPhase, ControlTimeQueue, ControlTimeQueueError, ScheduledControl};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleObservation<T> {
    match_time: MatchTime,
    visible_at: MatchTime,
    observation: T,
}

impl<T> VisibleObservation<T> {
    #[must_use]
    pub const fn match_time(&self) -> MatchTime {
        self.match_time
    }

    #[must_use]
    pub const fn visible_at(&self) -> MatchTime {
        self.visible_at
    }

    #[must_use]
    pub const fn observation(&self) -> &T {
        &self.observation
    }

    #[must_use]
    pub fn into_observation(self) -> T {
        self.observation
    }
}

#[derive(Debug)]
pub struct ObservationVisibilityQueue<T> {
    controls: ControlTimeQueue<VisibleObservation<T>>,
    market_data_latency_ms: u64,
}

impl<T> ObservationVisibilityQueue<T> {
    #[must_use]
    pub const fn new(market_data_latency_ms: u64) -> Self {
        Self {
            controls: ControlTimeQueue::new(),
            market_data_latency_ms,
        }
    }

    #[must_use]
    pub const fn market_data_latency_ms(&self) -> u64 {
        self.market_data_latency_ms
    }

    pub fn enqueue(
        &mut self,
        match_time: MatchTime,
        observation: T,
    ) -> Result<MatchTime, ObservationVisibilityError> {
        let visible_at = add_milliseconds(match_time, self.market_data_latency_ms)?;
        self.controls
            .schedule(
                visible_at,
                ControlPhase::ReleaseObservation,
                VisibleObservation {
                    match_time,
                    visible_at,
                    observation,
                },
            )
            .map_err(ObservationVisibilityError::ControlQueue)?;
        Ok(visible_at)
    }

    #[must_use]
    pub fn next_visible_at(&self) -> Option<MatchTime> {
        self.controls.next_time()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.controls.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }

    pub fn pop_before(
        &mut self,
        boundary: MatchTime,
    ) -> Vec<ScheduledControl<VisibleObservation<T>>> {
        self.controls.pop_before(boundary)
    }

    pub fn pop_at(&mut self, at: MatchTime) -> Vec<ScheduledControl<VisibleObservation<T>>> {
        self.controls.pop_at(at)
    }

    pub fn pop_through(
        &mut self,
        boundary: MatchTime,
    ) -> Vec<ScheduledControl<VisibleObservation<T>>> {
        self.controls.pop_through(boundary)
    }
}

pub fn add_milliseconds(
    time: MatchTime,
    milliseconds: u64,
) -> Result<MatchTime, ObservationVisibilityError> {
    let microseconds = milliseconds
        .checked_mul(1_000)
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(ObservationVisibilityError::TimeOverflow)?;
    let value = time
        .as_unix_microseconds()
        .checked_add(microseconds)
        .ok_or(ObservationVisibilityError::TimeOverflow)?;
    Ok(MatchTime::from_unix_microseconds(value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationVisibilityError {
    TimeOverflow,
    ControlQueue(ControlTimeQueueError),
}

impl fmt::Display for ObservationVisibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeOverflow => formatter.write_str("observation visible time overflow"),
            Self::ControlQueue(error) => error.fmt(formatter),
        }
    }
}

impl Error for ObservationVisibilityError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(micros: i64) -> MatchTime {
        MatchTime::from_unix_microseconds(micros)
    }

    #[test]
    fn visibility_adds_only_market_data_latency() {
        let mut queue = ObservationVisibilityQueue::new(200);
        let visible_at = queue.enqueue(time(1_000_000), "book").unwrap();
        assert_eq!(visible_at, time(1_200_000));
        assert_eq!(queue.market_data_latency_ms(), 200);

        let release = queue.pop_at(visible_at).pop().unwrap();
        assert_eq!(release.phase(), ControlPhase::ReleaseObservation);
        assert_eq!(release.payload().match_time(), time(1_000_000));
        assert_eq!(release.payload().visible_at(), time(1_200_000));
        assert_eq!(release.payload().observation(), &"book");
    }

    #[test]
    fn equal_visible_times_preserve_deterministic_enqueue_order() {
        let mut queue = ObservationVisibilityQueue::new(0);
        queue.enqueue(time(10), "first").unwrap();
        queue.enqueue(time(10), "second").unwrap();

        assert_eq!(
            queue
                .pop_at(time(10))
                .into_iter()
                .map(ScheduledControl::into_payload)
                .map(VisibleObservation::into_observation)
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn visible_time_overflow_is_explicit() {
        let mut queue = ObservationVisibilityQueue::new(1);
        assert_eq!(
            queue.enqueue(time(i64::MAX), ()).unwrap_err(),
            ObservationVisibilityError::TimeOverflow
        );
        assert!(queue.is_empty());
    }
}
