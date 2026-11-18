use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, VecDeque},
};

use osmium_types::{Message, Timestamp};

use crate::MergedFeeds;

/// The kinds of event a caller schedules, in the order they are processed when they
/// fall on the same instant. Market data takes the places before and between them:
/// a message at the exchange comes first, a message reaching the strategy comes
/// after reports and before timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScheduledClass {
    /// A simulated order, amendment or cancellation arrives at the exchange.
    OrderArrival,
    /// A report reaches the strategy.
    Report,
    /// A strategy timer fires.
    Timer,
}

impl ScheduledClass {
    /// The class number of `docs/reference/replay.md` section 3.
    const fn rank(self) -> u8 {
        match self {
            Self::OrderArrival => 1,
            Self::Report => 2,
            Self::Timer => 4,
        }
    }
}

const MARKET_AT_EXCHANGE: u8 = 0;
const MARKET_AT_STRATEGY: u8 = 3;

/// One step of the timeline.
///
/// A message is lent, not copied: it stays with the timeline, which every step of a
/// trading day goes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<'a, E> {
    /// A market data message takes effect at the exchange.
    MarketAtExchange(&'a Message),
    /// An event passed to [`Timeline::schedule`].
    Scheduled(ScheduledClass, E),
    /// A market data message reaches the strategy.
    MarketAtStrategy(&'a Message),
}

struct Entry<E> {
    time: Timestamp,
    class: ScheduledClass,
    ordinal: u64,
    event: E,
}

impl<E> Entry<E> {
    const fn key(&self) -> (Timestamp, u8, u64) {
        (self.time, self.class.rank(), self.ordinal)
    }
}

impl<E> PartialEq for Entry<E> {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl<E> Eq for Entry<E> {}

impl<E> PartialOrd for Entry<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<E> Ord for Entry<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

/// One trading day's events in processing order.
///
/// Every step is the pending event with the smallest `(time, class, order)`: market
/// data in the feeds' total order, scheduled events of one class in the order they
/// were scheduled. The time of the last step is the replay clock.
pub struct Timeline<I, E> {
    market: MergedFeeds<I>,
    market_data_latency: i64,
    /// Messages applied at the exchange and not yet delivered to the strategy.
    in_flight: VecDeque<Message>,
    /// The front of `in_flight` was lent as [`Event::MarketAtStrategy`] by the last
    /// step and leaves the queue at the next one.
    delivered: bool,
    scheduled: BinaryHeap<Reverse<Entry<E>>>,
    next_ordinal: u64,
    now: Option<Timestamp>,
}

impl<I: Iterator<Item = Message>, E> Timeline<I, E> {
    /// `market_data_latency` is the microseconds between a message's observed time
    /// and its delivery to the strategy.
    ///
    /// # Panics
    ///
    /// Panics when `market_data_latency` is negative.
    #[must_use]
    pub fn new(market: MergedFeeds<I>, market_data_latency: i64) -> Self {
        assert!(market_data_latency >= 0, "latency is not negative");
        Self {
            market,
            market_data_latency,
            in_flight: VecDeque::new(),
            delivered: false,
            scheduled: BinaryHeap::new(),
            next_ordinal: 0,
            now: None,
        }
    }

    /// The replay clock: the time of the event returned last; `None` before the first.
    #[must_use]
    pub const fn now(&self) -> Option<Timestamp> {
        self.now
    }

    /// Adds an event at `time`.
    ///
    /// An event scheduled for the current instant with a class that precedes the
    /// current event's is still the next one processed.
    ///
    /// # Panics
    ///
    /// Panics when `time` is before the replay clock.
    pub fn schedule(&mut self, time: Timestamp, class: ScheduledClass, event: E) {
        assert!(
            self.now.is_none_or(|now| time >= now),
            "an event cannot be scheduled before the replay clock"
        );
        self.scheduled.push(Reverse(Entry {
            time,
            class,
            ordinal: self.next_ordinal,
            event,
        }));
        self.next_ordinal += 1;
    }

    /// Removes and returns the next event with its time; `None` when nothing is pending.
    pub fn next_event(&mut self) -> Option<(Timestamp, Event<'_, E>)> {
        if self.delivered {
            self.in_flight.pop_front();
            self.delivered = false;
        }
        let mut next: Option<(Timestamp, u8)> = None;
        let mut offer = |candidate: (Timestamp, u8)| {
            if next.is_none_or(|next| candidate < next) {
                next = Some(candidate);
            }
        };
        if let Some(message) = self.market.peek() {
            offer((message.observed_time, MARKET_AT_EXCHANGE));
        }
        if let Some(message) = self.in_flight.front() {
            offer((self.delivery_time(message), MARKET_AT_STRATEGY));
        }
        if let Some(Reverse(entry)) = self.scheduled.peek() {
            offer((entry.time, entry.class.rank()));
        }
        let (time, class) = next?;
        self.now = Some(time);
        let event = match class {
            MARKET_AT_EXCHANGE => {
                self.market.pop_into(&mut self.in_flight);
                Event::MarketAtExchange(self.in_flight.back()?)
            }
            MARKET_AT_STRATEGY => {
                self.delivered = true;
                Event::MarketAtStrategy(self.in_flight.front()?)
            }
            _ => {
                let Reverse(entry) = self.scheduled.pop()?;
                Event::Scheduled(entry.class, entry.event)
            }
        };
        Some((time, event))
    }

    fn delivery_time(&self, message: &Message) -> Timestamp {
        message.observed_time.plus_micros(self.market_data_latency)
    }
}

#[cfg(test)]
mod tests {
    use osmium_types::{Feed, Message, Timestamp};

    use super::{Event, ScheduledClass, Timeline};
    use crate::{MergedFeeds, merge::tests::message};

    type Day = Timeline<std::vec::IntoIter<Message>, &'static str>;

    fn timeline(messages: Vec<Message>, latency: i64) -> Day {
        Timeline::new(MergedFeeds::new([messages.into_iter()]), latency)
    }

    fn at(micros: i64) -> Timestamp {
        Timestamp::from_micros(micros)
    }

    /// Drains the timeline into `(time, description)`.
    fn drain(timeline: &mut Day) -> Vec<(i64, String)> {
        let mut steps = Vec::new();
        while let Some((time, event)) = timeline.next_event() {
            steps.push((time.micros(), describe(&event)));
            assert_eq!(timeline.now(), Some(time));
        }
        steps
    }

    fn describe(event: &Event<'_, &'static str>) -> String {
        match event {
            Event::MarketAtExchange(message) => format!("exchange {}", message.seq),
            Event::MarketAtStrategy(message) => format!("strategy {}", message.seq),
            Event::Scheduled(_, name) => (*name).to_owned(),
        }
    }

    fn steps(expected: &[(i64, &str)]) -> Vec<(i64, String)> {
        expected
            .iter()
            .map(|(time, name)| (*time, (*name).to_owned()))
            .collect()
    }

    #[test]
    fn messages_reach_the_strategy_after_the_market_data_latency() {
        let mut day = timeline(
            vec![
                message(Feed::TwseF6, 1, 100),
                message(Feed::TwseF6, 2, 103),
                message(Feed::TwseF6, 3, 120),
            ],
            5,
        );
        assert_eq!(day.now(), None);
        assert_eq!(
            drain(&mut day),
            steps(&[
                (100, "exchange 1"),
                (103, "exchange 2"),
                (105, "strategy 1"),
                (108, "strategy 2"),
                (120, "exchange 3"),
                (125, "strategy 3"),
            ])
        );
    }

    #[test]
    fn events_at_one_instant_are_processed_by_class() {
        let mut day = timeline(
            vec![message(Feed::TwseF6, 1, 90), message(Feed::TwseF6, 2, 100)],
            10,
        );
        // Scheduled against class order on purpose.
        day.schedule(at(100), ScheduledClass::Timer, "timer");
        day.schedule(at(100), ScheduledClass::Report, "report");
        day.schedule(at(100), ScheduledClass::OrderArrival, "order");
        assert_eq!(
            drain(&mut day),
            steps(&[
                (90, "exchange 1"),
                (100, "exchange 2"),
                (100, "order"),
                (100, "report"),
                (100, "strategy 1"),
                (100, "timer"),
                (110, "strategy 2"),
            ])
        );
    }

    #[test]
    fn events_of_one_class_keep_the_order_they_were_scheduled_in() {
        let mut day = timeline(vec![], 0);
        day.schedule(at(7), ScheduledClass::Report, "late");
        day.schedule(at(5), ScheduledClass::Report, "first");
        day.schedule(at(5), ScheduledClass::Report, "second");
        day.schedule(at(5), ScheduledClass::Report, "third");
        assert_eq!(
            drain(&mut day),
            steps(&[(5, "first"), (5, "second"), (5, "third"), (7, "late")])
        );
    }

    #[test]
    fn an_order_sent_with_zero_latency_arrives_before_later_events_of_the_instant() {
        let mut day = timeline(
            vec![message(Feed::TwseF6, 1, 100), message(Feed::TwseF6, 2, 100)],
            0,
        );
        day.schedule(at(100), ScheduledClass::Timer, "timer");
        let mut log = Vec::new();
        while let Some((time, event)) = day.next_event() {
            log.push((time.micros(), describe(&event)));
            // The strategy answers the first message it sees with an order that needs
            // no time to arrive: it is processed next, although its class is lower.
            if matches!(event, Event::MarketAtStrategy(message) if message.seq == 1) {
                day.schedule(time, ScheduledClass::OrderArrival, "order");
            }
        }
        assert_eq!(
            log,
            steps(&[
                // Every message of the instant is at the exchange before the strategy
                // sees the first.
                (100, "exchange 1"),
                (100, "exchange 2"),
                (100, "strategy 1"),
                (100, "order"),
                (100, "strategy 2"),
                (100, "timer"),
            ])
        );
    }

    #[test]
    fn an_order_arrives_after_every_message_observed_by_then() {
        let mut day = timeline(
            vec![
                message(Feed::TwseF6, 1, 100),
                message(Feed::TwseF6, 2, 150),
                message(Feed::TwseF6, 3, 151),
            ],
            1_000,
        );
        day.schedule(at(150), ScheduledClass::OrderArrival, "order");
        assert_eq!(
            drain(&mut day)[..4],
            steps(&[
                (100, "exchange 1"),
                (150, "exchange 2"),
                (150, "order"),
                (151, "exchange 3"),
            ])
        );
    }

    #[test]
    fn timers_after_the_last_message_still_fire() {
        let mut day = timeline(vec![message(Feed::TwseF6, 1, 100)], 0);
        day.schedule(at(900), ScheduledClass::Timer, "timer");
        assert_eq!(
            drain(&mut day),
            steps(&[(100, "exchange 1"), (100, "strategy 1"), (900, "timer")])
        );
        assert!(day.next_event().is_none());
    }

    #[test]
    #[should_panic(expected = "before the replay clock")]
    fn nothing_can_be_scheduled_in_the_past() {
        let mut day = timeline(vec![message(Feed::TwseF6, 1, 100)], 0);
        day.next_event();
        day.schedule(at(99), ScheduledClass::Timer, "timer");
    }
}
