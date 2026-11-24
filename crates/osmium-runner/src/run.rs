//! A run: the trading days of a date range, one after the other
//! (`docs/reference/replay.md` section 4).

use std::{collections::HashMap, ops::Range, path::Path};

use osmium_book::BookState;
use osmium_ledger::{
    Calendar, DayFacts, Diagnostic, Eligibility, ForcedOrder, Ledger, LedgerError,
};
use osmium_matching::Engine;
use osmium_md::{
    CorporateActions, DailyQuotes, DayManifest, ManifestEntry, MarketEvents, MdError, MdRoot,
    Messages, Table, TradingCalendar,
};
use osmium_replay::{Event, MergedFeeds, ScheduledClass, Timeline};
use osmium_strategy::{Account, Command, Context, Market, Outbox, Strategy, TimerId};
use osmium_types::{
    DailyQuote, Date, Feed, Money, NewOrder, OrderChange, OrderEvent, OrderEventKind, OrderId,
    Position, RejectReason, Request, Symbol, Timestamp,
};

use crate::{
    Latency, RunConfig, RunError,
    day::{DayData, Reference, load_day},
    output::Output,
    summary::{DayFiles, DaySummary, FileIdentity, RunSummary, write_summary},
};

/// What a run needs besides its configuration.
pub struct RunRequest<'a> {
    pub config: &'a RunConfig,
    /// The text the configuration was parsed from; it is copied into the output.
    pub config_text: &'a str,
    pub strategy: &'a mut dyn Strategy,
    /// Recorded in the output, so that results of different versions of a strategy
    /// are not mistaken for each other.
    pub strategy_version: &'a str,
    /// The directory the results are written to. It must be empty or not exist.
    pub output: &'a Path,
    /// Called after every trading day.
    pub progress: &'a mut dyn FnMut(&DaySummary),
}

/// Something waiting on the timeline.
#[derive(Debug, Clone, Copy)]
enum Pending {
    New {
        id: OrderId,
        order: NewOrder,
    },
    /// Forced order number `index` of the day.
    Forced {
        id: OrderId,
        index: usize,
        order: NewOrder,
    },
    Change {
        id: OrderId,
        change: OrderChange,
    },
    /// A report with the account as it stood when the report was produced.
    Report {
        event: OrderEvent,
        available: Money,
        position: Option<(Symbol, Position)>,
    },
    Timer(TimerId),
}

#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    orders: u64,
    fills: u64,
}

/// One trading day in progress.
struct DayRun<'a, 'd> {
    date: Date,
    latency: Latency,
    engine: Engine,
    timeline: Timeline<Messages<'d>, Pending>,
    ledger: &'a mut Ledger,
    market: &'a mut Market,
    account: &'a mut Account,
    outbox: &'a mut Outbox,
    strategy: &'a mut dyn Strategy,
    output: &'a mut Output,
    counts: Counts,
    /// The instrument of every order of the day.
    orders: HashMap<OrderId, Symbol>,
    events: Vec<OrderEvent>,
    /// The exchange has expired what was left; nothing new is accepted.
    closed: bool,
}

impl DayRun<'_, '_> {
    /// Calls the strategy and puts what it asked for on the timeline.
    fn call(
        &mut self,
        now: Timestamp,
        call: impl FnOnce(&mut dyn Strategy, &mut Context<'_>),
    ) -> Result<(), RunError> {
        let mut context = Context::new(
            now,
            self.date,
            false,
            self.market,
            self.account,
            self.outbox,
        );
        call(self.strategy, &mut context);
        self.flush(now)
    }

    /// Puts what the strategy asked for in its last call on the timeline.
    fn flush(&mut self, now: Timestamp) -> Result<(), RunError> {
        let commands: Vec<Command> = self.outbox.drain().collect();
        for command in commands {
            match command {
                Command::Submit { id, order } => {
                    let arrives = now.plus_micros(self.latency.order);
                    self.output
                        .order(id, self.date, now, arrives, &order, None)?;
                    self.orders.insert(id, order.symbol);
                    self.counts.orders += 1;
                    self.timeline.schedule(
                        arrives,
                        ScheduledClass::OrderArrival,
                        Pending::New { id, order },
                    );
                }
                Command::Change { id, change } => self.timeline.schedule(
                    now.plus_micros(self.latency.order),
                    ScheduledClass::OrderArrival,
                    Pending::Change { id, change },
                ),
                Command::Timer { id, time } => {
                    self.timeline
                        .schedule(time, ScheduledClass::Timer, Pending::Timer(id));
                }
            }
        }
        Ok(())
    }

    /// Books what the exchange produced and sends the reports on their way.
    fn report(&mut self) -> Result<(), RunError> {
        let mut events = std::mem::take(&mut self.events);
        for event in &events {
            self.ledger.on_event(event);
            self.output.event(event)?;
            if matches!(event.kind, OrderEventKind::Filled(_)) {
                self.counts.fills += 1;
            }
            let position = self
                .orders
                .get(&event.order)
                .map(|&symbol| (symbol, self.ledger.position(symbol)));
            self.timeline.schedule(
                event.time.plus_micros(self.latency.report),
                ScheduledClass::Report,
                Pending::Report {
                    event: *event,
                    available: self.ledger.available(),
                    position,
                },
            );
        }
        events.clear();
        self.events = events;
        Ok(())
    }

    fn arrive(&mut self, now: Timestamp, pending: Pending) -> Result<(), RunError> {
        let ledger = &mut *self.ledger;
        match pending {
            Pending::New { id, .. } | Pending::Forced { id, .. } if self.closed => {
                self.events.push(OrderEvent {
                    order: id,
                    time: now,
                    kind: OrderEventKind::Rejected {
                        request: Request::New,
                        reason: RejectReason::MarketClosed,
                    },
                });
            }
            Pending::New { id, order } => self.engine.submit(
                id,
                &order,
                now,
                || ledger.check_new(id, &order),
                &mut self.events,
            ),
            Pending::Forced { id, index, order } => self.engine.submit(
                id,
                &order,
                now,
                || ledger.check_forced(id, index),
                &mut self.events,
            ),
            Pending::Change { id, change } => self.engine.change(
                id,
                change,
                now,
                |_| match change {
                    OrderChange::Reprice { price } => ledger.check_reprice(id, price),
                    OrderChange::Cancel | OrderChange::Reduce { .. } => Ok(()),
                },
                &mut self.events,
            ),
            Pending::Report {
                event,
                available,
                position,
            } => {
                if let Some((symbol, position)) = position {
                    self.account.update(available, symbol, position);
                }
                return self.call(now, |strategy, context| {
                    strategy.on_order_event(context, &event);
                });
            }
            Pending::Timer(id) => {
                return self.call(now, |strategy, context| strategy.on_timer(context, id));
            }
        }
        self.report()
    }

    /// Runs the timeline until nothing is left and returns the last instant.
    fn run(&mut self, start: Timestamp) -> Result<Timestamp, RunError> {
        let mut now = start;
        loop {
            let Some((time, event)) = self.timeline.next_event() else {
                if self.closed {
                    return Ok(now);
                }
                // The data is over: what still rests expires, and the reports of
                // that are the last events of the day.
                self.closed = true;
                self.engine.end_day(now, &mut self.events);
                self.report()?;
                continue;
            };
            now = time;
            match event {
                Event::MarketAtExchange(message) => {
                    self.engine.on_market(message, &mut self.events);
                    if !self.events.is_empty() {
                        self.report()?;
                    }
                }
                Event::MarketAtStrategy(message) => {
                    // The message stays with the timeline, so the strategy is called
                    // here rather than through `call`, which needs all of `self`.
                    self.market.apply(message);
                    let mut context = Context::new(
                        now,
                        self.date,
                        false,
                        self.market,
                        self.account,
                        self.outbox,
                    );
                    self.strategy.on_market(&mut context, message);
                    self.flush(now)?;
                }
                Event::Scheduled(_, pending) => self.arrive(now, pending)?,
            }
        }
    }
}

/// What lasts for the whole run.
struct Run<'a> {
    config: &'a RunConfig,
    root: MdRoot,
    ledger: Ledger,
    actions: CorporateActions,
    events: MarketEvents,
    reference: Reference,
    market: Market,
    account: Account,
    outbox: Outbox,
    output: Output,
    /// The closing quotes of the last trading day processed.
    quotes: Option<DailyQuotes>,
    /// The equity at the last close.
    equity: Money,
}

fn ledger_error(date: Date) -> impl FnOnce(LedgerError) -> RunError {
    move |source| RunError::Ledger { date, source }
}

fn identity(entry: &ManifestEntry) -> FileIdentity {
    FileIdentity {
        table: entry.table.clone(),
        part: entry.part.clone(),
        path: entry.path.clone(),
        rows: entry.rows,
        git_commit: entry.git_commit.clone(),
        code_hash: entry.code_hash.clone(),
        inputs_hash: entry.inputs_hash.clone(),
    }
}

impl Run<'_> {
    fn refresh_account(&mut self) {
        let ledger = &self.ledger;
        self.account.reset(
            ledger.cash(),
            ledger.available(),
            self.equity,
            ledger
                .holdings()
                .map(|(symbol, _)| (symbol, ledger.position(symbol))),
        );
    }

    /// The diagnostics of the day's data.
    fn note_data(&mut self, date: Date, data: &DayData) -> Result<(), RunError> {
        for feed in &data.skipped {
            self.output
                .diagnostic(date, "feed_skipped", None, feed.part())?;
        }
        if !data.assumed_lots.is_empty() {
            self.output.diagnostic(
                date,
                "lot_size_assumed",
                None,
                &format!("{} instruments", data.assumed_lots.len()),
            )?;
        }
        for file in &data.files {
            let (gaps, duplicates) = (file.seq_gaps.unwrap_or(0), file.seq_dup.unwrap_or(0));
            if gaps != 0 || duplicates != 0 || file.partial {
                self.output.diagnostic(
                    date,
                    "data_quality",
                    None,
                    &format!(
                        "{} {}: seq_gaps={gaps} seq_dup={duplicates} partial={}",
                        file.table, file.part, file.partial
                    ),
                )?;
            }
        }
        for reason in [
            RejectReason::UnknownInstrument,
            RejectReason::UnsupportedInstrument,
        ] {
            let books = data
                .books
                .iter()
                .filter(|book| book.spec.err() == Some(reason))
                .count();
            if books != 0 {
                self.output.diagnostic(
                    date,
                    "untradable_books",
                    None,
                    &format!("{}: {books} books", reason.as_str()),
                )?;
            }
        }
        Ok(())
    }

    fn day(
        &mut self,
        date: Date,
        previous: Option<Date>,
        strategy: &mut dyn Strategy,
    ) -> Result<(DaySummary, DayFiles), RunError> {
        let config = self.config;
        let start = date.at_taipei(8, 30, 0);

        // Before the open (`docs/reference/accounting.md` section 3).
        let previous = match previous {
            None => HashMap::new(),
            Some(previous) => {
                let quotes = match self.quotes.take() {
                    Some(quotes) if quotes.date() == previous => quotes,
                    _ => self.reference.quotes(&self.root, previous)?,
                };
                let (facts, has_quotas) = self.reference.previous(&self.root, &quotes)?;
                if !has_quotas && config.ledger.credit.eligibility == Eligibility::MarketdataApprox
                {
                    self.output.diagnostic(
                        date,
                        "no_credit_quotas",
                        None,
                        &format!("daily_ext has no row for {previous}: nothing can be bought on margin or sold short"),
                    )?;
                }
                facts
            }
        };
        let data = load_day(&self.root, config, date)?;
        self.note_data(date, &data)?;
        let facts = DayFacts::build(
            date,
            &data.instruments,
            &previous,
            self.events.rows(),
            self.actions.rows(),
            &config.ledger,
        );
        drop(previous);
        let open = self.ledger.begin_day(facts).map_err(ledger_error(date))?;
        for entry in &open.entries {
            self.output.entry(entry)?;
        }
        for realized in &open.realized {
            self.output.realized(realized)?;
        }
        self.note_ledger(date, &open.diagnostics)?;

        let mut engine = Engine::new(date, config.auction_marginal);
        for book in &data.books {
            match book.spec {
                Ok(spec) => engine.add_book(book.id, spec),
                Err(reason) => engine.add_untradable_book(book.id, book.method, reason),
            }
        }
        self.market.start_day(
            data.instruments.iter().copied(),
            data.books
                .iter()
                .map(|book| (book.id, BookState::new(book.method, date))),
        );
        self.refresh_account();
        for feed in &data.feeds {
            if let Some(first) = feed.messages().next()
                && first.observed_time < start
            {
                return Err(RunError::Unsupported {
                    date,
                    reason: format!(
                        "{} has a message at {}, before orders are taken",
                        feed.feed().part(),
                        first.observed_time
                    ),
                });
            }
        }

        let mut run = DayRun {
            date,
            latency: config.latency,
            engine,
            timeline: Timeline::new(
                MergedFeeds::new(data.feeds.iter().map(osmium_md::FeedDay::messages)),
                config.latency.market_data,
            ),
            ledger: &mut self.ledger,
            market: &mut self.market,
            account: &mut self.account,
            outbox: &mut self.outbox,
            strategy: &mut *strategy,
            output: &mut self.output,
            counts: Counts::default(),
            orders: HashMap::new(),
            events: Vec::new(),
            closed: false,
        };
        // Forced orders go straight to the exchange when their board starts to take
        // orders (accounting section 12.4).
        for (index, &ForcedOrder { order, reason }) in open.forced.iter().enumerate() {
            let id = run.outbox.next_order_id();
            let arrives = Engine::collection_start(date, order.board);
            run.output
                .order(id, date, start, arrives, &order, Some(reason))?;
            run.orders.insert(id, order.symbol);
            run.counts.orders += 1;
            run.timeline.schedule(
                arrives,
                ScheduledClass::OrderArrival,
                Pending::Forced { id, index, order },
            );
            run.call(start, |strategy, context| {
                strategy.on_forced_order(context, id, &order);
            })?;
        }
        run.call(start, |strategy, context| strategy.on_day_start(context))?;
        let end = run.run(start)?;
        let diagnostics = *run.engine.diagnostics();
        let counts = run.counts;
        drop(run);

        // The close.
        let quotes = self.reference.quotes(&self.root, date)?;
        let by_symbol: HashMap<Symbol, DailyQuote> = quotes
            .rows()
            .iter()
            .map(|quote| (quote.symbol, *quote))
            .collect();
        let close = self
            .ledger
            .end_day(&by_symbol)
            .map_err(ledger_error(date))?;
        drop(by_symbol);
        self.quotes = Some(quotes);
        for entry in &close.entries {
            self.output.entry(entry)?;
        }
        for realized in &close.realized {
            self.output.realized(realized)?;
        }
        self.output.day(&close)?;
        for (symbol, holding) in self.ledger.holdings() {
            self.output
                .position(date, symbol, holding, self.ledger.valuation(symbol))?;
        }
        self.note_ledger(date, &close.diagnostics)?;
        for (kind, count) in [
            ("aggressor_unknown", diagnostics.aggressor_unknown),
            ("limit_unknown", diagnostics.limit_unknown),
        ] {
            if count != 0 {
                self.output
                    .diagnostic(date, kind, None, &count.to_string())?;
            }
        }

        self.equity = close.equity;
        self.refresh_account();
        let mut context = Context::new(
            end,
            date,
            true,
            &self.market,
            &self.account,
            &mut self.outbox,
        );
        strategy.on_day_end(&mut context);

        let messages: usize = data.feeds.iter().map(osmium_md::FeedDay::len).sum();
        Ok((
            DaySummary {
                date,
                messages: messages as u64,
                cash: close.cash,
                equity: close.equity,
                orders: counts.orders,
                fills: counts.fills,
            },
            DayFiles {
                date,
                files: data.files.iter().map(identity).collect(),
            },
        ))
    }

    fn note_ledger(&mut self, date: Date, diagnostics: &[Diagnostic]) -> Result<(), RunError> {
        for diagnostic in diagnostics {
            match *diagnostic {
                Diagnostic::StalePosition { symbol } => self.output.diagnostic(
                    date,
                    "stale_position",
                    Some(symbol),
                    "not in today's instruments; valued at its last price",
                )?,
            }
        }
        Ok(())
    }
}

/// Every trading day of the calendar, and which of them the run covers.
fn trading_days(
    calendar: &TradingCalendar,
    config: &RunConfig,
) -> Result<(Vec<Date>, Range<usize>), RunError> {
    let all: Vec<Date> = calendar.trading_days().collect();
    let days = all.partition_point(|&date| date < config.start)
        ..all.partition_point(|&date| date <= config.end);
    if days.is_empty() {
        return Err(RunError::NoTradingDay {
            start: config.start,
            end: config.end,
        });
    }
    Ok((all, days))
}

/// Runs a strategy over the configured days and writes the results.
///
/// The output directory is complete when it holds `run.json`, which is written last.
pub fn run(request: RunRequest<'_>) -> Result<RunSummary, RunError> {
    let RunRequest {
        config,
        config_text,
        strategy,
        strategy_version,
        output,
        progress,
    } = request;
    let root = MdRoot::new(&config.md_root);
    let calendar = TradingCalendar::load(&root)?;
    let (all_days, days) = trading_days(&calendar, config)?;
    let settlement_days: Vec<Date> = calendar.settlement_days().collect();
    let actions = CorporateActions::load(&root, config.start)?;
    let events = MarketEvents::load(&root, config.start)?;
    let mut reference = Reference::new();
    for file in [calendar.file(), actions.file(), events.file()] {
        reference.note(file);
    }
    drop(calendar);

    let mut run = Run {
        config,
        ledger: Ledger::new(
            config.ledger.clone(),
            Calendar::new(all_days.iter().copied(), settlement_days),
        ),
        root,
        actions,
        events,
        reference,
        market: Market::new(),
        account: Account::new(),
        outbox: Outbox::new(),
        output: Output::create(output)?,
        quotes: None,
        equity: config.ledger.account.initial_cash,
    };
    let mut files = Vec::new();
    let mut summaries = Vec::new();
    for index in days {
        let date = all_days[index];
        let previous = index.checked_sub(1).map(|before| all_days[before]);
        let (summary, day_files) = run.day(date, previous, strategy)?;
        progress(&summary);
        summaries.push(summary);
        files.push(day_files);
    }
    write_summary(
        run.output,
        config,
        config_text,
        strategy_version,
        run.reference.into_files(),
        files,
        &summaries,
    )
}

/// The days a run would cover and the feeds they lack, found from the calendar and the
/// manifests without reading any tick data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub days: Vec<Date>,
    /// The configured feeds without data, by day. A run stops at the first of them
    /// unless `data.missing_days` is `"skip"`.
    pub missing: Vec<(Date, Feed)>,
}

/// Looks up what [`run`] would read.
pub fn plan(config: &RunConfig) -> Result<Plan, RunError> {
    let root = MdRoot::new(&config.md_root);
    let (all_days, days) = trading_days(&TradingCalendar::load(&root)?, config)?;
    let days = all_days[days].to_vec();
    let absent = |error: &MdError| {
        matches!(
            error,
            MdError::MissingManifest { .. } | MdError::MissingManifestEntry { .. }
        )
    };
    let mut missing = Vec::new();
    for &date in &days {
        let manifest = match DayManifest::load(&root, date) {
            Ok(manifest) => manifest,
            Err(error) if absent(&error) => {
                missing.extend(config.feeds.iter().map(|&feed| (date, feed)));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        for &feed in &config.feeds {
            let entries = [
                manifest.entry(Table::Instruments, feed.venue().as_str()),
                manifest.entry(Table::Books, feed.part()),
                manifest.entry(Table::Trades, feed.part()),
            ];
            for entry in entries {
                match entry {
                    Ok(_) => {}
                    Err(error) if absent(&error) => {
                        missing.push((date, feed));
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }
    Ok(Plan { days, missing })
}
