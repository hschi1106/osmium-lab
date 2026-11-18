use std::fmt;

use osmium_types::{Date, NewOrder, OrderChange, OrderId, Price, Shares, Timestamp};

use crate::{Account, Market};

/// Identifies a timer within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimerId(pub u64);

/// Why a timer was not set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerError {
    /// The time is before the replay clock.
    Past,
    /// The time is not on the current trading day (Taipei time).
    AnotherDay,
    /// The day is over.
    DayOver,
}

impl fmt::Display for TimerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Past => "the time is before the replay clock",
            Self::AnotherDay => "the time is not on the current trading day",
            Self::DayOver => "the day is over",
        })
    }
}

impl std::error::Error for TimerError {}

/// What a strategy asked for during one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Submit { id: OrderId, order: NewOrder },
    Change { id: OrderId, change: OrderChange },
    Timer { id: TimerId, time: Timestamp },
}

/// Collects the commands of a strategy and hands out order and timer ids, which are
/// unique within a run. For the code that runs a strategy.
#[derive(Debug, Default)]
pub struct Outbox {
    next_order: u64,
    next_timer: u64,
    commands: Vec<Command>,
}

impl Outbox {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next unused order id.
    pub const fn next_order_id(&mut self) -> OrderId {
        self.next_order += 1;
        OrderId(self.next_order)
    }

    /// Removes and returns the commands collected so far, in the order they were made.
    pub fn drain(&mut self) -> impl Iterator<Item = Command> + '_ {
        self.commands.drain(..)
    }
}

/// What a strategy can see and do during one call.
pub struct Context<'a> {
    now: Timestamp,
    date: Date,
    day_over: bool,
    market: &'a Market,
    account: &'a Account,
    outbox: &'a mut Outbox,
}

impl<'a> Context<'a> {
    /// For the code that runs a strategy. `day_over` refuses orders and timers.
    #[must_use]
    pub const fn new(
        now: Timestamp,
        date: Date,
        day_over: bool,
        market: &'a Market,
        account: &'a Account,
        outbox: &'a mut Outbox,
    ) -> Self {
        Self {
            now,
            date,
            day_over,
            market,
            account,
            outbox,
        }
    }

    /// The replay clock: the time of the event the strategy is being called for.
    #[must_use]
    pub const fn now(&self) -> Timestamp {
        self.now
    }

    /// The trading day.
    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    #[must_use]
    pub const fn market(&self) -> &'a Market {
        self.market
    }

    #[must_use]
    pub const fn account(&self) -> &'a Account {
        self.account
    }

    /// Sends a new order. It reaches the exchange after the order latency, and what
    /// becomes of it is reported through `on_order_event`. Returns `None` when the day
    /// is over (in `on_day_end`).
    pub fn submit(&mut self, order: NewOrder) -> Option<OrderId> {
        if self.day_over {
            return None;
        }
        let id = self.outbox.next_order_id();
        self.outbox.commands.push(Command::Submit { id, order });
        Some(id)
    }

    /// Asks to cancel what remains of an order.
    pub fn cancel(&mut self, id: OrderId) {
        self.change(id, OrderChange::Cancel);
    }

    /// Asks to reduce an order by `by` shares; it keeps its place in the queue.
    pub fn reduce(&mut self, id: OrderId, by: Shares) {
        self.change(id, OrderChange::Reduce { by });
    }

    /// Asks to move an order to another price; it loses its place in the queue.
    pub fn reprice(&mut self, id: OrderId, price: Price) {
        self.change(id, OrderChange::Reprice { price });
    }

    fn change(&mut self, id: OrderId, change: OrderChange) {
        if !self.day_over {
            self.outbox.commands.push(Command::Change { id, change });
        }
    }

    /// Sets a timer for `time`, which must not be before the replay clock and must be
    /// on the current trading day.
    pub fn set_timer(&mut self, time: Timestamp) -> Result<TimerId, TimerError> {
        if self.day_over {
            return Err(TimerError::DayOver);
        }
        if time < self.now {
            return Err(TimerError::Past);
        }
        if time.taipei_date() != self.date {
            return Err(TimerError::AnotherDay);
        }
        self.outbox.next_timer += 1;
        let id = TimerId(self.outbox.next_timer);
        self.outbox.commands.push(Command::Timer { id, time });
        Ok(id)
    }
}
