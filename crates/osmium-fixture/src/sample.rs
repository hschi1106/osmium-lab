//! A small market that is enough to see every kind of fill: one instrument on the
//! regular board, two days with an opening auction, continuous trading and a closing
//! auction, and the reference tables of the surrounding week.

use osmium_types::{Date, Feed};

use crate::{BookRow, FeedRows, TradeRow, Tree, book, instrument, quote, trade};

/// The instrument of the sample: board lots of 1,000 shares, a reference price of 100
/// and limits of 90 and 110.
pub const SYMBOL: &str = "2330";
/// The feed of the sample.
pub const FEED: Feed = Feed::TwseF6;
/// The two days with tick data.
pub const DAYS: [&str; 2] = ["2026-10-15", "2026-10-16"];
/// The first and the last day a run over the sample can cover. The days after
/// [`DAYS`] have no tick data, so such a run needs `data.missing_days = "skip"`; they
/// are there for the settlements to be paid.
pub const RANGE: (&str, &str) = ("2026-10-15", "2026-10-21");

/// `exch_ts` of a Taiwan wall-clock time on `day`.
///
/// # Panics
///
/// Panics when `day` is not a date.
#[must_use]
pub fn at(day: &str, hour: u32, minute: u32, second: u32) -> i64 {
    let date: Date = day.parse().expect("a date as YYYY-MM-DD");
    date.at_taipei(hour, minute, second).micros()
}

fn levels(bid: &[(f64, i64)], ask: &[(f64, i64)], row: BookRow) -> BookRow {
    BookRow {
        bid: Some(bid.to_vec()),
        ask: Some(ask.to_vec()),
        ..row
    }
}

/// One day of the sample, numbered from `seq`:
///
/// | time | what happens | book afterwards |
/// | --- | --- | --- |
/// | 09:00:00 | opening auction: 4 lots at 100 | 100 × 7 / 100.5 × 3 |
/// | 09:00:05 | disclosure | 100 × 7 / 100.5 × 3 |
/// | 09:00:10 | 4 lots sold at 100 | 100 × 3 / 100.5 × 3 |
/// | 09:00:12 | 6 lots join the bid | 100 × 9 / 100.5 × 3 |
/// | 09:00:15 | 5 lots sold at 100 | 100 × 4 / 100.5 × 3 |
/// | 09:00:20 | 3 lots bought at 100.5 | 100 × 4 / 101 × 2 |
/// | 09:00:25 | 1 lot bought at 101 | 100 × 4 / 101 × 1 |
/// | 13:30:00 | closing auction: 6 lots at 100.5 | 100 × 2 / 101 × 1 |
#[must_use]
pub fn day(day: &str, seq: i64) -> FeedRows {
    let after = |seconds: u32| at(day, 9, 0, seconds);
    let traded = |seq: i64, ts: i64, bid: &[(f64, i64)], ask: &[(f64, i64)]| {
        levels(
            bid,
            ask,
            BookRow {
                has_trade: true,
                ..book(SYMBOL, seq, ts)
            },
        )
    };
    let close = at(day, 13, 30, 0);
    FeedRows {
        feed: FEED,
        books: vec![
            BookRow {
                phase: "OPEN_AUCTION",
                ..traded(seq, after(0), &[(100.0, 7)], &[(100.5, 3)])
            },
            book(SYMBOL, seq + 1, after(5)),
            traded(seq + 2, after(10), &[(100.0, 3)], &[(100.5, 3)]),
            levels(
                &[(100.0, 9)],
                &[(100.5, 3)],
                book(SYMBOL, seq + 3, after(12)),
            ),
            traded(seq + 4, after(15), &[(100.0, 4)], &[(100.5, 3)]),
            traded(seq + 5, after(20), &[(100.0, 4)], &[(101.0, 2)]),
            traded(seq + 6, after(25), &[(100.0, 4)], &[(101.0, 1)]),
            BookRow {
                phase: "CLOSE_AUCTION",
                ..traded(seq + 7, close, &[(100.0, 2)], &[(101.0, 1)])
            },
        ],
        trades: vec![
            TradeRow {
                phase: "OPEN_AUCTION",
                ..trade(SYMBOL, seq, after(0), 100.0, 4)
            },
            trade(SYMBOL, seq + 2, after(10), 100.0, 4),
            trade(SYMBOL, seq + 4, after(15), 100.0, 5),
            trade(SYMBOL, seq + 5, after(20), 100.5, 3),
            trade(SYMBOL, seq + 6, after(25), 101.0, 1),
            TradeRow {
                phase: "CLOSE_AUCTION",
                ..trade(SYMBOL, seq + 7, close, 100.5, 6)
            },
        ],
    }
}

/// Writes the sample: [`day`] on both of [`DAYS`], a calendar from the 14th to the
/// 23rd of October 2026 (the 17th and 18th are a weekend), closing quotes of 100.5 for
/// every day up to the 21st, and no credit quotas, corporate actions or events.
#[must_use]
pub fn tree(name: &str) -> Tree {
    let tree = Tree::new(name);
    tree.calendar(&[
        ("2026-10-14", true),
        ("2026-10-15", true),
        ("2026-10-16", true),
        ("2026-10-19", true),
        ("2026-10-20", true),
        ("2026-10-21", true),
        ("2026-10-22", true),
        ("2026-10-23", true),
    ]);
    let quoted = [
        "2026-10-14",
        "2026-10-15",
        "2026-10-16",
        "2026-10-19",
        "2026-10-20",
        "2026-10-21",
    ];
    let quotes: Vec<_> = quoted.iter().map(|day| quote(day, SYMBOL, 100.5)).collect();
    tree.quotes(&quotes);
    tree.quotas("2026", &[]);
    for (index, date) in DAYS.iter().enumerate() {
        let seq = 100 * (i64::try_from(index).expect("two days") + 1);
        tree.day(
            date,
            &[instrument(SYMBOL, Some(1_000))],
            &[],
            &[day(date, seq)],
        );
    }
    tree
}

/// The text of a configuration that runs `strategy` over the whole sample with an
/// account of 1,000,000 and latencies of 100, 1,000 and 2,000 microseconds.
/// `params` is the body of `[strategy.params]`.
#[must_use]
pub fn config(tree: &Tree, strategy: &str, params: &str) -> String {
    format!(
        r#"[run]
start = "{}"
end = "{}"

[data]
md_root = "{}"
feeds = ["{}"]
missing_days = "skip"

[latency]
market_data = 100
order = 1000
report = 2000

[account]
initial_cash = 1000000

[credit]
eligibility = "none"

[strategy]
name = "{strategy}"

[strategy.params]
{params}
"#,
        RANGE.0,
        RANGE.1,
        tree.path().display(),
        FEED.part(),
    )
}
