//! Replays of a real marketdata tree. Ignored by default because they need the data:
//!
//! ```text
//! OSMIUM_MD_ROOT=/srv/md/v1 cargo test --release --locked -p osmium-replay --test marketdata -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;

use osmium_book::{BookState, Collecting, Method, MethodEvidence, Mode, is_auction_result};
use osmium_md::{DEFAULT_MD_ROOT, DayInstruments, DayManifest, FeedDay, MdRoot, MissingLotSize};
use osmium_replay::{Event, MergedFeeds, MessageOrder, ScheduledClass, Timeline};
use osmium_types::{Board, Date, Feed, MessageBody, Phase, Symbol, Timestamp};

fn root() -> MdRoot {
    MdRoot::new(std::env::var("OSMIUM_MD_ROOT").unwrap_or_else(|_| DEFAULT_MD_ROOT.to_owned()))
}

fn load(date: &str, feed: Feed) -> (FeedDay, DayInstruments) {
    let root = root();
    let date: Date = date.parse().unwrap();
    let manifest = DayManifest::load(&root, date).unwrap();
    let instruments = DayInstruments::load(&root, &manifest, feed.venue()).unwrap();
    let day = FeedDay::load(
        &root,
        &manifest,
        feed,
        &instruments,
        None,
        MissingLotSize::Assume(1_000),
    )
    .unwrap();
    (day, instruments)
}

#[derive(Default)]
struct BookFacts {
    evidence: MethodEvidence,
    open_results: u32,
    close_results: u32,
}

#[derive(Debug, Default)]
struct Methods {
    known_call: usize,
    known_continuous: usize,
    inferred_call: usize,
    inferred_continuous: usize,
}

/// Replays one feed day through a [`BookState`] per book and checks the mode model of
/// `docs/reference/matching.md` section 4 against the phase of every message.
fn check_modes(day: &FeedDay, instruments: &DayInstruments) -> Methods {
    let board = day.feed().board();
    let close_collection = BookState::close_collection(day.date());

    let mut facts: BTreeMap<Symbol, BookFacts> = BTreeMap::new();
    for message in day.messages() {
        let facts = facts.entry(message.symbol).or_default();
        facts.evidence.observe(&message, close_collection);
        if is_auction_result(&message) {
            match message.phase {
                Phase::OpenAuction => facts.open_results += 1,
                Phase::CloseAuction => facts.close_results += 1,
                Phase::PeriodicAuction | Phase::Continuous => {}
            }
        }
    }

    let mut methods = Methods::default();
    let mut states: BTreeMap<Symbol, BookState> = BTreeMap::new();
    for (&symbol, facts) in &facts {
        assert_eq!(facts.open_results, 1, "{symbol}: opening auction results");
        assert_eq!(facts.close_results, 1, "{symbol}: closing auction results");
        let interval = instruments
            .get(symbol)
            .and_then(|instrument| instrument.match_interval_s);
        let inferred = facts.evidence.method();
        let method = match Method::known(board, interval) {
            Some(known) => {
                match known {
                    Method::Call => methods.known_call += 1,
                    Method::Continuous => methods.known_continuous += 1,
                }
                if board == Board::Regular {
                    assert_eq!(inferred, known, "{symbol}: match_interval_s {interval:?}");
                }
                known
            }
            None => {
                match inferred {
                    Method::Call => methods.inferred_call += 1,
                    Method::Continuous => methods.inferred_continuous += 1,
                }
                inferred
            }
        };
        states.insert(symbol, BookState::new(method, day.date()));
    }

    for message in day.messages() {
        let state = states.get_mut(&message.symbol).unwrap();
        let mode = state.mode(message.observed_time);
        let indicative = matches!(message.body, MessageBody::Indicative { .. });
        let delay = matches!(message.body, MessageBody::Delay(_));
        let expected = match mode {
            // The older source labels the trial matches of a delayed open as periodic.
            Mode::Collecting(Collecting::Open) => {
                message.phase == Phase::OpenAuction
                    || (message.phase == Phase::PeriodicAuction && indicative)
            }
            Mode::Collecting(Collecting::Periodic) => {
                matches!(message.phase, Phase::PeriodicAuction | Phase::CloseAuction)
            }
            // The clock is the feed's running maximum, so the last continuous messages of
            // a book can come after another book's first message of 13:25.
            Mode::Collecting(Collecting::Close) => {
                message.phase == Phase::CloseAuction
                    || (message.phase == Phase::Continuous && message.exch_ts < close_collection)
            }
            Mode::Collecting(Collecting::Volatility) => {
                message.phase == Phase::PeriodicAuction && !delay
            }
            Mode::Continuous => message.phase == Phase::Continuous,
            Mode::Closed => false,
        };
        assert!(expected, "mode {mode:?} before {message:?}");
        state.apply(&message);
    }
    for (symbol, state) in &states {
        assert_eq!(
            state.mode(Timestamp::from_micros(i64::MAX)),
            Mode::Closed,
            "{symbol}"
        );
    }
    methods
}

#[test]
#[ignore = "needs a marketdata tree"]
fn book_modes_agree_with_the_phase_of_every_message() {
    for (date, feed) in [
        ("2026-10-08", Feed::TwseF6),
        ("2026-10-08", Feed::TpexF6),
        ("2026-10-08", Feed::TwseF23),
        ("2026-10-08", Feed::TpexF23),
        // A market-wide fall with many matching delays.
        ("2025-04-07", Feed::TpexF6),
        // Days without `match_interval_s`: the older source, and no exchange instruments.
        ("2022-06-15", Feed::TwseF6),
        ("2026-10-15", Feed::TwseF6),
    ] {
        let (day, instruments) = load(date, feed);
        let methods = check_modes(&day, &instruments);
        println!(
            "{date} {}: {} messages, {methods:?}",
            feed.part(),
            day.len()
        );
        if feed.board() == Board::Regular {
            assert!(methods.known_call + methods.inferred_call > 0);
        }
    }
}

#[test]
#[ignore = "needs a marketdata tree"]
fn merged_feeds_replay_in_exchange_order() {
    const LATENCY: i64 = 350;
    let days: Vec<FeedDay> = [Feed::TwseF6, Feed::TpexF6, Feed::TwseF23, Feed::TpexF23]
        .into_iter()
        .map(|feed| load("2026-10-08", feed).0)
        .collect();
    let total: usize = days.iter().map(FeedDay::len).sum();

    let merged = MergedFeeds::new(days.iter().map(FeedDay::messages));
    let mut timeline: Timeline<_, u32> = Timeline::new(merged, LATENCY);
    let date = days[0].date();
    let timers = [date.at_taipei(9, 0, 0), date.at_taipei(13, 30, 0)];
    let mut pending_timers = 0;

    let (mut at_exchange, mut at_strategy, mut fired) = (0_usize, 0_usize, 0_usize);
    let mut previous_time: Option<Timestamp> = None;
    let mut previous_order: Option<MessageOrder> = None;
    let mut previous_delivery: Option<MessageOrder> = None;
    let mut feeds_seen = [false; 6];
    while let Some((time, event)) = timeline.next_event() {
        assert!(previous_time.is_none_or(|previous| time >= previous));
        previous_time = Some(time);
        match event {
            Event::MarketAtExchange(message) => {
                assert_eq!(time, message.observed_time);
                let order = MessageOrder::of(message);
                assert!(previous_order.is_none_or(|previous| order > previous));
                previous_order = Some(order);
                feeds_seen[usize::from(message.feed.rank())] = true;
                at_exchange += 1;
                // Schedule each timer from the first event of the day.
                if at_exchange == 1 {
                    for (index, &timer) in timers.iter().enumerate() {
                        timeline.schedule(
                            timer,
                            ScheduledClass::Timer,
                            u32::try_from(index).unwrap(),
                        );
                        pending_timers += 1;
                    }
                }
            }
            Event::MarketAtStrategy(message) => {
                assert_eq!(time, message.observed_time.plus_micros(LATENCY));
                let order = MessageOrder::of(message);
                assert!(previous_delivery.is_none_or(|previous| order > previous));
                previous_delivery = Some(order);
                at_strategy += 1;
            }
            Event::Scheduled(class, index) => {
                assert_eq!(class, ScheduledClass::Timer);
                assert_eq!(time, timers[index as usize]);
                fired += 1;
            }
        }
        assert_eq!(timeline.now(), Some(time));
    }
    assert_eq!(at_exchange, total);
    assert_eq!(at_strategy, total);
    assert_eq!(fired, pending_timers);
    assert_eq!(feeds_seen.iter().filter(|&&seen| seen).count(), days.len());
    println!("{total} messages of {} feeds replayed twice", days.len());
}
