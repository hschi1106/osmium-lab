use std::{collections::BTreeMap, error::Error, fmt};

use market_types::MatchTime;

pub const CONTROL_ORDERING_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ControlPhase {
    ReleaseObservation = 1,
    StrategyDecision = 2,
    OrderExpiry = 3,
    OrderActivation = 4,
    FillAllocation = 5,
    Accounting = 6,
    Feedback = 7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ControlOrderingKey {
    at: MatchTime,
    phase: ControlPhase,
    sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledControl<T> {
    at: MatchTime,
    phase: ControlPhase,
    sequence: u64,
    payload: T,
}

impl<T> ScheduledControl<T> {
    #[must_use]
    pub const fn at(&self) -> MatchTime {
        self.at
    }

    #[must_use]
    pub const fn phase(&self) -> ControlPhase {
        self.phase
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    #[must_use]
    pub fn into_payload(self) -> T {
        self.payload
    }
}

#[derive(Debug)]
pub struct ControlTimeQueue<T> {
    pending: BTreeMap<ControlOrderingKey, T>,
    next_sequence: u64,
}

impl<T> ControlTimeQueue<T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
            next_sequence: 1,
        }
    }

    pub fn schedule(
        &mut self,
        at: MatchTime,
        phase: ControlPhase,
        payload: T,
    ) -> Result<u64, ControlTimeQueueError> {
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(ControlTimeQueueError::SequenceOverflow)?;
        let previous = self.pending.insert(
            ControlOrderingKey {
                at,
                phase,
                sequence,
            },
            payload,
        );
        debug_assert!(previous.is_none(), "control sequence makes the key unique");
        Ok(sequence)
    }

    #[must_use]
    pub fn next_time(&self) -> Option<MatchTime> {
        self.pending.first_key_value().map(|(key, _)| key.at)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn pop_before(&mut self, boundary: MatchTime) -> Vec<ScheduledControl<T>> {
        self.pop_while(|key| key.at < boundary)
    }

    pub fn pop_at(&mut self, at: MatchTime) -> Vec<ScheduledControl<T>> {
        self.pop_while(|key| key.at == at)
    }

    pub fn pop_through(&mut self, boundary: MatchTime) -> Vec<ScheduledControl<T>> {
        self.pop_while(|key| key.at <= boundary)
    }

    fn pop_while(
        &mut self,
        predicate: impl Fn(&ControlOrderingKey) -> bool,
    ) -> Vec<ScheduledControl<T>> {
        let mut popped = Vec::new();
        while let Some((&key, _)) = self.pending.first_key_value() {
            if !predicate(&key) {
                break;
            }
            let payload = self
                .pending
                .pop_first()
                .expect("first entry was observed above")
                .1;
            popped.push(ScheduledControl {
                at: key.at,
                phase: key.phase,
                sequence: key.sequence,
                payload,
            });
        }
        popped
    }
}

impl<T> Default for ControlTimeQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlTimeQueueError {
    SequenceOverflow,
}

impl fmt::Display for ControlTimeQueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("control-time sequence overflow")
    }
}

impl Error for ControlTimeQueueError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(micros: i64) -> MatchTime {
        MatchTime::from_unix_microseconds(micros)
    }

    #[test]
    fn phases_then_insertion_sequence_define_same_time_order() {
        let mut queue = ControlTimeQueue::new();
        queue
            .schedule(time(10), ControlPhase::OrderActivation, "activate")
            .unwrap();
        queue
            .schedule(time(10), ControlPhase::OrderExpiry, "expire-first")
            .unwrap();
        queue
            .schedule(time(10), ControlPhase::OrderExpiry, "expire-second")
            .unwrap();
        queue
            .schedule(time(10), ControlPhase::StrategyDecision, "decision")
            .unwrap();

        let values = queue
            .pop_at(time(10))
            .into_iter()
            .map(ScheduledControl::into_payload)
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            ["decision", "expire-first", "expire-second", "activate"]
        );
    }

    #[test]
    fn before_excludes_market_boundary_and_at_handles_it_after_commit() {
        let mut queue = ControlTimeQueue::new();
        queue
            .schedule(time(9), ControlPhase::StrategyDecision, "before")
            .unwrap();
        queue
            .schedule(time(10), ControlPhase::ReleaseObservation, "same")
            .unwrap();
        queue
            .schedule(time(11), ControlPhase::Feedback, "after")
            .unwrap();

        assert_eq!(
            queue
                .pop_before(time(10))
                .into_iter()
                .map(ScheduledControl::into_payload)
                .collect::<Vec<_>>(),
            ["before"]
        );
        assert_eq!(
            queue
                .pop_at(time(10))
                .into_iter()
                .map(ScheduledControl::into_payload)
                .collect::<Vec<_>>(),
            ["same"]
        );
        assert_eq!(queue.next_time(), Some(time(11)));
    }

    #[test]
    fn pop_through_drains_in_global_control_order() {
        let mut queue = ControlTimeQueue::new();
        queue.schedule(time(12), ControlPhase::Feedback, 3).unwrap();
        queue.schedule(time(10), ControlPhase::Feedback, 1).unwrap();
        queue.schedule(time(11), ControlPhase::Feedback, 2).unwrap();

        assert_eq!(
            queue
                .pop_through(time(11))
                .into_iter()
                .map(ScheduledControl::into_payload)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(queue.len(), 1);
    }
}
