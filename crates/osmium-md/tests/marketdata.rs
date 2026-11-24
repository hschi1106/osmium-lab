//! Reads of a real marketdata tree. Ignored by default because they need the data:
//!
//! ```text
//! OSMIUM_MD_ROOT=/srv/md/v1 cargo test --release --locked -p osmium-md --test marketdata -- --ignored --nocapture
//! ```

use std::time::Instant;

use osmium_md::{
    DEFAULT_MD_ROOT, DayInstruments, DayManifest, FeedDay, MdError, MdRoot, MissingLotSize,
};
use osmium_types::{Date, Feed, Message, MessageBody, Symbol, Venue};

fn root() -> MdRoot {
    MdRoot::new(std::env::var("OSMIUM_MD_ROOT").unwrap_or_else(|_| DEFAULT_MD_ROOT.to_owned()))
}

fn load(
    date: &str,
    feed: Feed,
    selection: Option<&[&str]>,
    missing_lot_size: MissingLotSize,
) -> Result<FeedDay, MdError> {
    let root = root();
    let date: Date = date.parse().unwrap();
    let manifest = DayManifest::load(&root, date)?;
    let instruments = DayInstruments::load(&root, &manifest, feed.venue())?;
    let selection: Option<Vec<Symbol>> = selection.map(|codes| {
        codes
            .iter()
            .map(|code| Symbol::new(code).unwrap())
            .collect()
    });
    FeedDay::load(
        &root,
        &manifest,
        feed,
        &instruments,
        selection.as_deref(),
        missing_lot_size,
    )
}

/// Message counts by kind.
#[derive(Debug, Default, PartialEq, Eq)]
struct Composition {
    /// Book changes without a trade.
    book: usize,
    /// Trades that disclose the book after them.
    fill: usize,
    intermediate: usize,
    indicative: usize,
    delay: usize,
}

/// Checks what the messages of every feed day must satisfy and counts them.
fn check_messages(day: &FeedDay) -> Composition {
    let mut composition = Composition::default();
    let mut previous: Option<Message> = None;
    for message in day.messages() {
        assert_eq!(message.feed, day.feed());
        assert_eq!(message.exch_ts.taipei_date(), day.date(), "{message:?}");
        assert!(message.observed_time >= message.exch_ts, "{message:?}");
        if let Some(previous) = previous {
            assert!(message.seq > previous.seq, "{message:?}");
            assert!(
                message.observed_time >= previous.observed_time,
                "{message:?}"
            );
        }
        match message.body {
            MessageBody::Disclosure { trade: None, .. } => composition.book += 1,
            MessageBody::Disclosure { trade: Some(_), .. } => composition.fill += 1,
            MessageBody::IntermediateTrade(_) => composition.intermediate += 1,
            MessageBody::Indicative { .. } => composition.indicative += 1,
            MessageBody::Delay(_) => composition.delay += 1,
        }
        previous = Some(message);
    }
    composition
}

/// [`check_messages`] for a day read without a selection, whose messages must also account
/// for every row of the files.
fn check(day: &FeedDay) -> Composition {
    let composition = check_messages(day);
    let identity = day.identity();
    let rows = |rows: i64| usize::try_from(rows).unwrap();
    // Every trades row is one fill. Every message has a books row, except an intermediate
    // fill, which is in trades alone.
    assert_eq!(
        composition.fill + composition.intermediate,
        rows(identity.trades.rows)
    );
    let in_books = composition.book + composition.fill + composition.indicative + composition.delay;
    assert_eq!(
        rows(identity.books.rows),
        in_books,
        "{composition:?} against {} books rows",
        identity.books.rows
    );
    composition
}

#[test]
#[ignore = "needs a marketdata tree"]
fn feed_days_decode_into_the_documented_message_kinds() {
    // An exchange-instruments day for every stock feed but the largest, and a day converted
    // from the older source, whose instruments marketdata derives.
    for (date, feed) in [
        ("2026-10-08", Feed::TwseF6),
        ("2026-10-08", Feed::TpexF6),
        ("2026-10-08", Feed::TpexF17),
        ("2026-10-08", Feed::TwseF23),
        ("2026-10-08", Feed::TpexF23),
        ("2022-06-15", Feed::TwseF6),
    ] {
        let started = Instant::now();
        let day = load(date, feed, None, MissingLotSize::Reject).unwrap();
        let loaded = started.elapsed();
        let composition = check(&day);
        println!(
            "{date} {}: {} messages, {} symbols, {} assumed lot sizes, {composition:?}, \
             loaded in {loaded:.1?}, checked in {:.1?}",
            feed.part(),
            day.len(),
            day.symbols().count(),
            day.assumed_lot_sizes().count(),
            started.elapsed().saturating_sub(loaded),
        );
        assert!(composition.fill > 0 && composition.indicative > 0);
    }
}

#[test]
#[ignore = "needs a marketdata tree"]
fn the_composition_of_a_known_day_is_stable() {
    // Counted independently from the parquet files with polars.
    let day = load("2026-10-08", Feed::TwseF6, None, MissingLotSize::Reject).unwrap();
    assert_eq!(day.len(), 15_442_565);
    assert_eq!(day.assumed_lot_sizes().count(), 0);
    let composition = check(&day);
    assert_eq!(composition.intermediate, 16_943);
    assert_eq!(composition.fill, 1_225_819);

    // A day converted from the older source.
    let older = load("2022-06-15", Feed::TwseF6, None, MissingLotSize::Reject).unwrap();
    assert_eq!(older.len(), 6_948_054);
    let composition = check(&older);
    assert_eq!(composition.intermediate, 10_510);
    assert_eq!(composition.delay, 30);
}

#[test]
#[ignore = "needs a marketdata tree"]
fn a_selection_reads_the_same_messages_as_the_whole_feed() {
    let codes = ["0050", "00636K", "2330"];
    let whole = load("2026-10-08", Feed::TwseF6, None, MissingLotSize::Reject).unwrap();
    let selected = load(
        "2026-10-08",
        Feed::TwseF6,
        Some(&codes),
        MissingLotSize::Reject,
    )
    .unwrap();
    let wanted: Vec<Symbol> = codes
        .iter()
        .map(|code| Symbol::new(code).unwrap())
        .collect();
    let expected: Vec<Message> = whole
        .messages()
        .filter(|message| wanted.contains(&message.symbol))
        .collect();
    assert!(!expected.is_empty());
    assert!(selected.messages().eq(expected));
    assert_eq!(selected.symbols().collect::<Vec<_>>(), wanted);
}

#[test]
#[ignore = "needs a marketdata tree"]
fn derived_instruments_have_lot_sizes() {
    // marketdata derives instruments where the exchange's are missing, lot sizes included:
    // a few foreign-currency ETFs trade in lots of 100 shares.
    let day = load("2022-06-15", Feed::TwseF6, None, MissingLotSize::Reject).unwrap();
    assert_eq!(day.assumed_lot_sizes().count(), 0);
    let manifest = DayManifest::load(&root(), "2022-06-15".parse().unwrap()).unwrap();
    let instruments = DayInstruments::load(&root(), &manifest, Venue::Twse).unwrap();
    let lot = |code: &str| {
        let instrument = instruments.get(Symbol::new(code).unwrap()).unwrap();
        assert!(instrument.derived, "{code}");
        instrument.shares_per_lot
    };
    assert_eq!((lot("2330"), lot("00636K")), (Some(1_000), Some(100)));
}
