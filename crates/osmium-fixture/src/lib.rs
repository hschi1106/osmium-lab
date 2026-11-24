//! Writes small marketdata trees for the tests of the crates above `osmium-md`: the
//! same layout, columns and footers as `/srv/md/v1` (`docs/reference/marketdata.md`).
//!
//! The tests of `osmium-md` keep a writer of their own, because they also write the
//! malformed files the reader must reject.

pub mod sample;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int8Array, Int16Array,
    Int32Array, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
};
use osmium_types::{Date, Feed, Venue};
use parquet::{arrow::ArrowWriter, file::metadata::KeyValue, file::properties::WriterProperties};

/// The `md.git_commit` of every file written.
pub const COMMIT: &str = "c0ffee";

type Columns = Vec<(&'static str, ArrayRef)>;
/// One side's `(price, qty)` levels, best first; `None` when the row discloses no book.
/// A NaN price is written as null.
pub type Levels = Option<Vec<(f64, i64)>>;

/// One `books` row.
#[derive(Debug, Clone)]
pub struct BookRow {
    pub symbol: &'static str,
    pub seq: i64,
    pub ts: i64,
    pub phase: &'static str,
    pub has_trade: bool,
    pub bid: Levels,
    pub ask: Levels,
    pub bid_mkt: Option<i64>,
    pub ask_mkt: Option<i64>,
    pub indicative: bool,
    pub matched: Option<(f64, i64)>,
    pub delay_open: Option<bool>,
    pub delay_close: Option<bool>,
    pub delay_state: i8,
}

/// A continuous-trading disclosure of 100 × 7 bid and 100.5 × 3 ask.
pub fn book(symbol: &'static str, seq: i64, ts: i64) -> BookRow {
    BookRow {
        symbol,
        seq,
        ts,
        phase: "CONTINUOUS",
        has_trade: false,
        bid: Some(vec![(100.0, 7)]),
        ask: Some(vec![(100.5, 3)]),
        bid_mkt: None,
        ask_mkt: None,
        indicative: false,
        matched: None,
        delay_open: Some(false),
        delay_close: Some(false),
        delay_state: 0,
    }
}

#[derive(Debug, Clone)]
pub struct TradeRow {
    pub symbol: &'static str,
    pub seq: i64,
    pub ts: i64,
    pub phase: &'static str,
    pub price: f64,
    pub qty: i64,
    pub cum_qty: Option<i64>,
    pub sub_seq: i16,
}

pub fn trade(symbol: &'static str, seq: i64, ts: i64, price: f64, qty: i64) -> TradeRow {
    TradeRow {
        symbol,
        seq,
        ts,
        phase: "CONTINUOUS",
        price,
        qty,
        cum_qty: Some(qty),
        sub_seq: 0,
    }
}

#[derive(Debug, Clone)]
pub struct InstrumentRow {
    pub symbol: &'static str,
    pub kind: &'static str,
    pub lot: Option<i32>,
    pub ref_price: Option<f64>,
    pub limit_up: Option<f64>,
    pub limit_down: Option<f64>,
    pub interval: Option<i32>,
    pub currency: &'static str,
    pub derived: bool,
    pub day_trade: Option<&'static str>,
    pub non_10_par: Option<bool>,
}

pub fn instrument(symbol: &'static str, lot: Option<i32>) -> InstrumentRow {
    InstrumentRow {
        symbol,
        kind: "STOCK",
        lot,
        ref_price: Some(100.0),
        limit_up: Some(110.0),
        limit_down: Some(90.0),
        interval: Some(0),
        currency: "TWD",
        derived: false,
        day_trade: Some("BOTH"),
        non_10_par: Some(false),
    }
}

fn strings<'a>(values: impl IntoIterator<Item = &'a str>) -> ArrayRef {
    Arc::new(StringArray::from(values.into_iter().collect::<Vec<_>>()))
}

fn timestamps(values: Vec<i64>) -> ArrayRef {
    Arc::new(TimestampMicrosecondArray::from(values).with_timezone("UTC"))
}

fn book_columns(rows: &[BookRow]) -> Columns {
    let mut columns: Columns = vec![
        ("symbol", strings(rows.iter().map(|row| row.symbol))),
        (
            "exch_ts",
            timestamps(rows.iter().map(|row| row.ts).collect()),
        ),
        (
            "seq",
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|row| row.seq))),
        ),
        ("phase", strings(rows.iter().map(|row| row.phase))),
        (
            "has_trade",
            Arc::new(BooleanArray::from_iter(
                rows.iter().map(|row| Some(row.has_trade)),
            )),
        ),
    ];
    type Side = (
        &'static str,
        fn(&BookRow) -> &Levels,
        fn(&BookRow) -> Option<i64>,
    );
    let sides: [Side; 2] = [
        ("bid", |row| &row.bid, |row| row.bid_mkt),
        ("ask", |row| &row.ask, |row| row.ask_mkt),
    ];
    for (side, levels, market) in sides {
        let count: &'static str = format!("n_{side}").leak();
        columns.push((
            count,
            Arc::new(Int8Array::from_iter(rows.iter().map(|row| {
                levels(row)
                    .as_ref()
                    .map(|levels| i8::try_from(levels.len()).unwrap())
            }))),
        ));
        for level in 0..5 {
            let cell = |row: &BookRow| -> Option<(f64, i64)> {
                levels(row)
                    .as_ref()
                    .and_then(|levels| levels.get(level).copied())
            };
            let price: &'static str = format!("{side}_px_{}", level + 1).leak();
            let qty: &'static str = format!("{side}_qty_{}", level + 1).leak();
            columns.push((
                price,
                Arc::new(Float64Array::from_iter(rows.iter().map(|row| {
                    cell(row).map(|cell| cell.0).filter(|price| !price.is_nan())
                }))),
            ));
            columns.push((
                qty,
                Arc::new(Int64Array::from_iter(
                    rows.iter().map(|row| cell(row).map(|cell| cell.1)),
                )),
            ));
        }
        let market_name: &'static str = format!("{side}_mkt_qty").leak();
        columns.push((
            market_name,
            Arc::new(Int64Array::from_iter(rows.iter().map(market))),
        ));
    }
    columns.extend([
        (
            "is_indicative",
            Arc::new(BooleanArray::from_iter(
                rows.iter().map(|row| Some(row.indicative)),
            )) as ArrayRef,
        ),
        (
            "indicative_px",
            Arc::new(Float64Array::from_iter(
                rows.iter().map(|row| row.matched.map(|matched| matched.0)),
            )),
        ),
        (
            "indicative_qty",
            Arc::new(Int64Array::from_iter(
                rows.iter().map(|row| row.matched.map(|matched| matched.1)),
            )),
        ),
        (
            "delay_open",
            Arc::new(BooleanArray::from_iter(
                rows.iter().map(|row| row.delay_open),
            )),
        ),
        (
            "delay_close",
            Arc::new(BooleanArray::from_iter(
                rows.iter().map(|row| row.delay_close),
            )),
        ),
        (
            "delay_state",
            Arc::new(Int8Array::from_iter_values(
                rows.iter().map(|row| row.delay_state),
            )),
        ),
    ]);
    columns
}

fn trade_columns(rows: &[TradeRow]) -> Columns {
    vec![
        ("symbol", strings(rows.iter().map(|row| row.symbol))),
        (
            "exch_ts",
            timestamps(rows.iter().map(|row| row.ts).collect()),
        ),
        (
            "seq",
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|row| row.seq))),
        ),
        (
            "sub_seq",
            Arc::new(Int16Array::from_iter_values(
                rows.iter().map(|row| row.sub_seq),
            )),
        ),
        (
            "price",
            Arc::new(Float64Array::from_iter_values(
                rows.iter().map(|row| row.price),
            )),
        ),
        (
            "qty",
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|row| row.qty))),
        ),
        (
            "cum_qty",
            Arc::new(Int64Array::from_iter(rows.iter().map(|row| row.cum_qty))),
        ),
        ("phase", strings(rows.iter().map(|row| row.phase))),
    ]
}

fn instrument_columns(rows: &[InstrumentRow]) -> Columns {
    let prices = |value: fn(&InstrumentRow) -> Option<f64>| -> ArrayRef {
        Arc::new(Float64Array::from_iter(rows.iter().map(value)))
    };
    vec![
        ("symbol", strings(rows.iter().map(|row| row.symbol))),
        ("instrument_type", strings(rows.iter().map(|row| row.kind))),
        (
            "shares_per_lot",
            Arc::new(Int32Array::from_iter(rows.iter().map(|row| row.lot))),
        ),
        ("ref_price", prices(|row| row.ref_price)),
        ("limit_up", prices(|row| row.limit_up)),
        ("limit_down", prices(|row| row.limit_down)),
        (
            "match_interval_s",
            Arc::new(Int32Array::from_iter(rows.iter().map(|row| row.interval))),
        ),
        ("currency", strings(rows.iter().map(|row| row.currency))),
        (
            "derived",
            Arc::new(BooleanArray::from_iter(
                rows.iter().map(|row| Some(row.derived)),
            )),
        ),
        (
            "daytrade",
            Arc::new(StringArray::from_iter(rows.iter().map(|row| row.day_trade))),
        ),
        (
            "f1_non10_par",
            Arc::new(BooleanArray::from_iter(
                rows.iter().map(|row| row.non_10_par),
            )),
        ),
    ]
}

fn dates<'a>(values: impl IntoIterator<Item = &'a str>) -> ArrayRef {
    Arc::new(Date32Array::from_iter_values(
        values.into_iter().map(|text| date(text).epoch_days()),
    ))
}

/// marketdata's `decimal(18,8)`, in units of 1e-8.
fn decimals(values: impl IntoIterator<Item = Option<i128>>) -> ArrayRef {
    Arc::new(
        Decimal128Array::from_iter(values)
            .with_precision_and_scale(18, 8)
            .expect("a valid precision"),
    )
}

fn floats(values: impl IntoIterator<Item = Option<f64>>) -> ArrayRef {
    Arc::new(Float64Array::from_iter(values))
}

fn date(text: &str) -> Date {
    text.parse().expect("a date as YYYY-MM-DD")
}

/// The books and trades of one feed for one day. Rows are written in the order
/// given, so they are listed sorted by `(symbol, seq)`.
#[derive(Debug, Clone)]
pub struct FeedRows {
    pub feed: Feed,
    pub books: Vec<BookRow>,
    pub trades: Vec<TradeRow>,
}

/// One row of `corp_actions`.
#[derive(Debug, Clone)]
pub struct ActionRow {
    pub symbol: &'static str,
    pub ex_date: &'static str,
    pub action: &'static str,
    pub before: Option<f64>,
    pub reference: Option<f64>,
    pub after: Option<f64>,
    pub cash: Option<f64>,
    pub pay_date: Option<&'static str>,
    /// Decimals in units of 1e-8.
    pub stock_per_1000: Option<i128>,
    pub value: Option<f64>,
    pub shares_old: Option<i128>,
    pub shares_new: Option<i128>,
    pub refund: Option<i128>,
}

/// One row of `daily`.
#[derive(Debug, Clone)]
pub struct QuoteRow {
    pub date: &'static str,
    pub symbol: &'static str,
    pub reference: Option<f64>,
    pub limit_down: Option<f64>,
    pub close: Option<f64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
}

/// A quote that closed at `close` with a reference price of 100.
#[must_use]
pub fn quote(date: &'static str, symbol: &'static str, close: f64) -> QuoteRow {
    QuoteRow {
        date,
        symbol,
        reference: Some(100.0),
        limit_down: Some(90.0),
        close: Some(close),
        bid: None,
        ask: None,
    }
}

/// One row of `daily_ext`: the margin and short quotas of a symbol.
#[derive(Debug, Clone)]
pub struct QuotaRow {
    pub date: &'static str,
    pub symbol: &'static str,
    pub margin_limit: Option<i64>,
    pub short_limit: Option<i64>,
}

/// One row of `events`.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub symbol: &'static str,
    pub event: &'static str,
    pub start_date: &'static str,
    pub end_date: Option<&'static str>,
}

/// A marketdata tree in the temporary directory, removed when dropped.
#[derive(Debug)]
pub struct Tree {
    root: PathBuf,
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn year(date: &str) -> &str {
    &date[..4]
}

impl Tree {
    /// An empty tree with the reference tables that every run reads: no corporate
    /// actions and no events. `name` keeps the trees of different tests apart.
    ///
    /// # Panics
    ///
    /// Panics, like every method of the tree, when a file cannot be written.
    #[must_use]
    pub fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("osmium-fixture-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let tree = Self { root };
        tree.actions(&[]);
        tree.events(&[]);
        tree
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    fn write(&self, relative: &str, columns: Columns) -> i64 {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("a file below the root")).unwrap();
        let batch = RecordBatch::try_from_iter(columns).unwrap();
        let properties = WriterProperties::builder()
            .set_key_value_metadata(Some(vec![
                KeyValue::new("md.schema_version".to_owned(), "1".to_owned()),
                KeyValue::new("md.git_commit".to_owned(), COMMIT.to_owned()),
            ]))
            .build();
        let mut writer = ArrowWriter::try_new(
            fs::File::create(path).unwrap(),
            batch.schema(),
            Some(properties),
        )
        .unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        i64::try_from(batch.num_rows()).unwrap()
    }

    /// `qc/calendar`: each day and whether both markets are open.
    pub fn calendar(&self, days: &[(&'static str, bool)]) {
        let open = || -> ArrayRef {
            Arc::new(BooleanArray::from_iter(days.iter().map(|day| Some(day.1))))
        };
        self.write(
            "qc/calendar.parquet",
            vec![
                ("trade_date", dates(days.iter().map(|day| day.0))),
                ("twse_open", open()),
                ("tpex_open", open()),
            ],
        );
    }

    /// The tick data of one day with its manifest. Instruments are given per market;
    /// a market without instruments gets no file.
    pub fn day(
        &self,
        day: &str,
        twse: &[InstrumentRow],
        tpex: &[InstrumentRow],
        feeds: &[FeedRows],
    ) {
        let relative =
            |table: &str, part: &str| format!("{table}/{}/{day}/{part}.parquet", year(day));
        // (table, part, path, rows)
        let mut entries: Vec<(&'static str, &'static str, String, i64)> = Vec::new();
        for (venue, instruments) in [(Venue::Twse, twse), (Venue::Tpex, tpex)] {
            if instruments.is_empty() {
                continue;
            }
            let path = relative("instruments", venue.as_str());
            let rows = self.write(&path, instrument_columns(instruments));
            entries.push(("instruments", venue.as_str(), path, rows));
        }
        for feed in feeds {
            let part = feed.feed.part();
            let path = relative("books", part);
            let rows = self.write(&path, book_columns(&feed.books));
            entries.push(("books", part, path, rows));
            let path = relative("trades", part);
            let rows = self.write(&path, trade_columns(&feed.trades));
            entries.push(("trades", part, path, rows));
        }
        let len = entries.len();
        self.write(
            &format!("manifest/{}/{day}/tick.parquet", year(day)),
            vec![
                ("job", strings(vec!["tick"; len])),
                ("table", strings(entries.iter().map(|entry| entry.0))),
                ("part", strings(entries.iter().map(|entry| entry.1))),
                (
                    "path",
                    strings(entries.iter().map(|entry| entry.2.as_str())),
                ),
                (
                    "rows",
                    Arc::new(Int64Array::from_iter_values(
                        entries.iter().map(|entry| entry.3),
                    )),
                ),
                ("partial", Arc::new(BooleanArray::from(vec![false; len]))),
                ("seq_gaps", Arc::new(Int64Array::from(vec![Some(0); len]))),
                ("seq_dup", Arc::new(Int64Array::from(vec![Some(0); len]))),
                ("schema_version", strings(vec!["1"; len])),
                ("git_commit", strings(vec![COMMIT; len])),
                ("code_hash", strings(vec!["code"; len])),
                ("inputs_hash", strings(vec!["inputs"; len])),
            ],
        );
    }

    /// `daily`: every row of every year at once, because a year is one file.
    pub fn quotes(&self, rows: &[QuoteRow]) {
        let mut years: BTreeMap<&str, Vec<&QuoteRow>> = BTreeMap::new();
        for row in rows {
            years.entry(year(row.date)).or_default().push(row);
        }
        for (year, rows) in years {
            self.write(
                &format!("daily/{year}.parquet"),
                vec![
                    ("trade_date", dates(rows.iter().map(|row| row.date))),
                    ("symbol", strings(rows.iter().map(|row| row.symbol))),
                    ("ref_price", floats(rows.iter().map(|row| row.reference))),
                    ("limit_down", floats(rows.iter().map(|row| row.limit_down))),
                    ("close", floats(rows.iter().map(|row| row.close))),
                    ("close_bid_px", floats(rows.iter().map(|row| row.bid))),
                    ("close_ask_px", floats(rows.iter().map(|row| row.ask))),
                ],
            );
        }
    }

    /// `daily_ext` of one year, which may have no rows.
    pub fn quotas(&self, year: &str, rows: &[QuotaRow]) {
        self.write(
            &format!("daily_ext/{year}.parquet"),
            vec![
                ("trade_date", dates(rows.iter().map(|row| row.date))),
                ("symbol", strings(rows.iter().map(|row| row.symbol))),
                (
                    "margin_limit",
                    Arc::new(Int64Array::from_iter(
                        rows.iter().map(|row| row.margin_limit),
                    )),
                ),
                (
                    "short_limit",
                    Arc::new(Int64Array::from_iter(
                        rows.iter().map(|row| row.short_limit),
                    )),
                ),
            ],
        );
    }

    /// `corp_actions`.
    pub fn actions(&self, rows: &[ActionRow]) {
        self.write(
            "corp_actions.parquet",
            vec![
                ("symbol", strings(rows.iter().map(|row| row.symbol))),
                ("ex_date", dates(rows.iter().map(|row| row.ex_date))),
                ("action", strings(rows.iter().map(|row| row.action))),
                ("before_price", floats(rows.iter().map(|row| row.before))),
                ("ref_price", floats(rows.iter().map(|row| row.reference))),
                ("after_price", floats(rows.iter().map(|row| row.after))),
                ("cash_dividend", floats(rows.iter().map(|row| row.cash))),
                (
                    "pay_date",
                    Arc::new(Date32Array::from_iter(
                        rows.iter()
                            .map(|row| row.pay_date.map(|text| date(text).epoch_days())),
                    )),
                ),
                (
                    "stock_per_1000",
                    decimals(rows.iter().map(|row| row.stock_per_1000)),
                ),
                ("value", floats(rows.iter().map(|row| row.value))),
                (
                    "shares_old",
                    decimals(rows.iter().map(|row| row.shares_old)),
                ),
                (
                    "shares_new",
                    decimals(rows.iter().map(|row| row.shares_new)),
                ),
                (
                    "refund_per_share",
                    decimals(rows.iter().map(|row| row.refund)),
                ),
            ],
        );
    }

    /// `events`.
    pub fn events(&self, rows: &[EventRow]) {
        self.write(
            "events.parquet",
            vec![
                ("symbol", strings(rows.iter().map(|row| row.symbol))),
                ("event", strings(rows.iter().map(|row| row.event))),
                ("start_date", dates(rows.iter().map(|row| row.start_date))),
                (
                    "end_date",
                    Arc::new(Date32Array::from_iter(
                        rows.iter()
                            .map(|row| row.end_date.map(|text| date(text).epoch_days())),
                    )),
                ),
            ],
        );
    }
}
