//! End-to-end reads of a synthetic marketdata tree written by the test itself.

use std::{fs, path::PathBuf, sync::Arc};

use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    RecordBatch, StringArray, TimestampMicrosecondArray,
};
use osmium_md::{DayInstruments, DayManifest, FeedDay, MdError, MdRoot, MissingLotSize, Table};
use osmium_types::{
    Book, Currency, Date, DayTradeFlag, DelayDirection, Feed, InstrumentType, Level, Message,
    MessageBody, Phase, Price, Shares, Symbol, Venue,
};
use parquet::{
    arrow::ArrowWriter,
    file::{
        metadata::KeyValue,
        properties::WriterProperties,
        reader::{FileReader, SerializedFileReader},
    },
};

const DATE: &str = "2026-10-15";
const COMMIT: &str = "c0ffee";

type Columns = Vec<(&'static str, ArrayRef)>;
/// One side's `(price, qty)` levels, best first; `None` when the row discloses no book.
/// A NaN price is written as null.
type Levels = Option<Vec<(f64, i64)>>;

/// One `books` row.
#[derive(Clone)]
struct BookRow {
    symbol: &'static str,
    seq: i64,
    ts: i64,
    phase: &'static str,
    has_trade: bool,
    bid: Levels,
    ask: Levels,
    bid_mkt: Option<i64>,
    ask_mkt: Option<i64>,
    indicative: bool,
    matched: Option<(f64, i64)>,
    delay_open: Option<bool>,
    delay_close: Option<bool>,
    delay_state: i8,
}

/// A continuous-trading disclosure of 100 × 7 bid and 100.5 × 3 ask.
fn book(symbol: &'static str, seq: i64, ts: i64) -> BookRow {
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

#[derive(Clone)]
struct TradeRow {
    symbol: &'static str,
    seq: i64,
    ts: i64,
    phase: &'static str,
    price: f64,
    qty: i64,
    cum_qty: Option<i64>,
    sub_seq: i16,
}

fn trade(symbol: &'static str, seq: i64, ts: i64, price: f64, qty: i64) -> TradeRow {
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

#[derive(Clone)]
struct InstrumentRow {
    symbol: &'static str,
    kind: &'static str,
    lot: Option<i32>,
    ref_price: Option<f64>,
    limit_up: Option<f64>,
    limit_down: Option<f64>,
    interval: Option<i32>,
    currency: &'static str,
    derived: bool,
    day_trade: Option<&'static str>,
    non_10_par: Option<bool>,
}

fn instrument(symbol: &'static str, lot: Option<i32>) -> InstrumentRow {
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
        day_trade: Some("A"),
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
            "f1_daytrade_flag",
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

/// A synthetic marketdata day for one feed. Rows are written in the order given, so a
/// well-formed fixture lists them sorted by `(symbol, seq)`.
struct Fixture {
    feed: Feed,
    books: Vec<BookRow>,
    trades: Vec<TradeRow>,
    instruments: Vec<InstrumentRow>,
    /// The `md.schema_version` written to footers and the manifest.
    schema_version: &'static str,
    /// Changes a table's columns just before it is written.
    tamper: fn(Table, &mut Columns),
    /// Changes the row count the manifest records for a table.
    manifest_rows: fn(Table, i64) -> i64,
    /// The largest number of rows in a row group of `books` and `trades`; `None` writes
    /// each as one row group.
    row_group_rows: Option<usize>,
}

impl Fixture {
    fn new(books: Vec<BookRow>, trades: Vec<TradeRow>) -> Self {
        Self {
            feed: Feed::TwseF6,
            books,
            trades,
            instruments: vec![instrument("A", Some(1_000)), instrument("B", Some(100))],
            schema_version: "1",
            tamper: |_, _| {},
            manifest_rows: |_, rows| rows,
            row_group_rows: None,
        }
    }

    fn write(&self, name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("osmium-md-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let md = MdRoot::new(&root);
        let date = date();
        let write = |path: PathBuf, columns: Columns, row_group_rows: Option<usize>| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let batch = RecordBatch::try_from_iter(columns).unwrap();
            let mut properties = WriterProperties::builder();
            if row_group_rows.is_some() {
                properties = properties.set_max_row_group_row_count(row_group_rows);
            }
            let properties = properties
                .set_key_value_metadata(Some(vec![
                    KeyValue::new(
                        "md.schema_version".to_owned(),
                        self.schema_version.to_owned(),
                    ),
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
        };

        let venue = self.feed.venue().as_str();
        let mut entries = Vec::new();
        for (table, part, mut columns) in [
            (Table::Books, self.feed.part(), book_columns(&self.books)),
            (Table::Trades, self.feed.part(), trade_columns(&self.trades)),
            (
                Table::Instruments,
                venue,
                instrument_columns(&self.instruments),
            ),
        ] {
            (self.tamper)(table, &mut columns);
            let row_group_rows = match table {
                Table::Books | Table::Trades => self.row_group_rows,
                _ => None,
            };
            let rows = write(md.file(table, date, part), columns, row_group_rows);
            entries.push((table, part, (self.manifest_rows)(table, rows)));
        }
        let len = entries.len();
        let paths: Vec<String> = entries
            .iter()
            .map(|entry| MdRoot::relative_file(entry.0, date, entry.1))
            .collect();
        write(
            md.manifest_dir(date).join("tick.parquet"),
            vec![
                ("job", strings(vec!["tick"; len])),
                (
                    "table",
                    strings(entries.iter().map(|entry| entry.0.as_str())),
                ),
                ("part", strings(entries.iter().map(|entry| entry.1))),
                ("path", strings(paths.iter().map(String::as_str))),
                (
                    "rows",
                    Arc::new(Int64Array::from_iter_values(
                        entries.iter().map(|entry| entry.2),
                    )),
                ),
                ("partial", Arc::new(BooleanArray::from(vec![false; len]))),
                ("seq_gaps", Arc::new(Int64Array::from(vec![Some(0); len]))),
                ("seq_dup", Arc::new(Int64Array::from(vec![Some(0); len]))),
                ("schema_version", strings(vec![self.schema_version; len])),
                ("git_commit", strings(vec![COMMIT; len])),
                ("code_hash", strings(vec!["code"; len])),
                ("inputs_hash", strings(vec!["inputs"; len])),
            ],
            None,
        );
        Tree {
            root,
            feed: self.feed,
        }
    }
}

struct Tree {
    root: PathBuf,
    feed: Feed,
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Tree {
    fn instruments(&self) -> Result<DayInstruments, MdError> {
        let root = MdRoot::new(&self.root);
        DayInstruments::load(&root, &DayManifest::load(&root, date())?, self.feed.venue())
    }

    fn load_with(
        &self,
        selection: Option<&[&str]>,
        missing_lot_size: MissingLotSize,
    ) -> Result<FeedDay, MdError> {
        let root = MdRoot::new(&self.root);
        let manifest = DayManifest::load(&root, date())?;
        let instruments = DayInstruments::load(&root, &manifest, self.feed.venue())?;
        let selection: Option<Vec<Symbol>> =
            selection.map(|codes| codes.iter().map(|code| symbol(code)).collect());
        FeedDay::load(
            &root,
            &manifest,
            self.feed,
            &instruments,
            selection.as_deref(),
            missing_lot_size,
        )
    }

    fn load(&self) -> Result<FeedDay, MdError> {
        self.load_with(None, MissingLotSize::Reject)
    }

    fn messages(&self) -> Vec<Message> {
        self.load().unwrap().messages().collect()
    }
}

fn date() -> Date {
    DATE.parse().unwrap()
}

fn symbol(code: &str) -> Symbol {
    Symbol::new(code).unwrap()
}

fn price(text: &str) -> Price {
    text.parse().unwrap()
}

fn level(text: &str, shares: i64) -> Level {
    Level {
        price: price(text),
        qty: Shares::new(shares),
    }
}

/// The reason of the `InvalidRow` a load fails with.
fn invalid_row(result: Result<FeedDay, MdError>) -> String {
    match result {
        Err(MdError::InvalidRow { reason, .. }) => reason,
        other => panic!("expected InvalidRow, got {other:?}"),
    }
}

/// Every kind of message, over two symbols.
fn kinds() -> Fixture {
    Fixture::new(
        vec![
            // A: an indicative row, the opening uncross with its trade, then a sweep:
            // seq 5 is an intermediate fill (trades only) and seq 6 the final fill.
            BookRow {
                phase: "OPEN_AUCTION",
                indicative: true,
                matched: Some((100.0, 4)),
                delay_open: Some(true),
                ..book("A", 1, 10)
            },
            BookRow {
                phase: "OPEN_AUCTION",
                has_trade: true,
                bid: Some(vec![(100.0, 7), (99.5, 2)]),
                ask: Some(vec![(100.5, 3), (101.0, 1)]),
                bid_mkt: Some(5),
                ..book("A", 3, 30)
            },
            BookRow {
                has_trade: true,
                ask: Some(vec![]),
                ..book("A", 6, 60)
            },
            // B: one disclosure, then matching is postponed.
            book("B", 2, 20),
            BookRow {
                bid: None,
                ask: None,
                delay_state: -1,
                ..book("B", 7, 70)
            },
        ],
        vec![
            TradeRow {
                phase: "OPEN_AUCTION",
                ..trade("A", 3, 30, 100.0, 4)
            },
            TradeRow {
                cum_qty: Some(6),
                ..trade("A", 5, 60, 100.5, 2)
            },
            TradeRow {
                cum_qty: None,
                ..trade("A", 6, 60, 101.0, 1)
            },
        ],
    )
}

#[test]
fn a_feed_becomes_messages_in_seq_order() {
    let tree = kinds().write("kinds");
    let day = tree.load().unwrap();
    assert_eq!(day.len(), 6);
    assert_eq!(day.feed(), Feed::TwseF6);
    assert_eq!(day.date(), date());
    assert_eq!(day.identity().books.rows, 5);
    assert_eq!(day.identity().trades.rows, 3);
    assert_eq!(
        day.symbols().collect::<Vec<_>>(),
        [symbol("A"), symbol("B")]
    );
    assert_eq!(day.assumed_lot_sizes().count(), 0);

    let messages: Vec<Message> = day.messages().collect();
    assert_eq!(
        messages
            .iter()
            .map(|message| message.seq)
            .collect::<Vec<_>>(),
        [1, 2, 3, 5, 6, 7]
    );
    assert!(messages.iter().all(|message| message.feed == Feed::TwseF6));

    // A trades in lots of 1,000 shares, B in lots of 100.
    let MessageBody::Indicative {
        book,
        matched,
        delay_open,
        delay_close,
    } = messages[0].body
    else {
        panic!("{:?}", messages[0]);
    };
    assert_eq!(messages[0].phase, Phase::OpenAuction);
    assert_eq!(book.bid().levels(), [level("100", 7_000)]);
    assert_eq!(matched, Some(level("100", 4_000)));
    assert_eq!((delay_open, delay_close), (Some(true), Some(false)));

    let MessageBody::Disclosure { book, trade: None } = messages[1].body else {
        panic!("{:?}", messages[1]);
    };
    assert_eq!(messages[1].symbol, symbol("B"));
    assert_eq!(book.bid().levels(), [level("100", 700)]);
    assert_eq!(book.ask().levels(), [level("100.5", 300)]);

    let MessageBody::Disclosure {
        book,
        trade: Some(open),
    } = messages[2].body
    else {
        panic!("{:?}", messages[2]);
    };
    assert_eq!(
        book.bid().levels(),
        [level("100", 7_000), level("99.5", 2_000)]
    );
    assert_eq!(
        book.ask().levels(),
        [level("100.5", 3_000), level("101", 1_000)]
    );
    assert_eq!(book.bid().market_qty(), Shares::new(5_000));
    assert_eq!(book.ask().market_qty(), Shares::ZERO);
    assert_eq!(
        (open.price, open.qty, open.cum_qty),
        (price("100"), Shares::new(4_000), Some(Shares::new(4_000)))
    );

    let MessageBody::IntermediateTrade(swept) = messages[3].body else {
        panic!("{:?}", messages[3]);
    };
    assert_eq!(messages[3].phase, Phase::Continuous);
    assert_eq!(
        (swept.price, swept.qty, swept.cum_qty),
        (price("100.5"), Shares::new(2_000), Some(Shares::new(6_000)))
    );

    let MessageBody::Disclosure {
        book,
        trade: Some(last),
    } = messages[4].body
    else {
        panic!("{:?}", messages[4]);
    };
    assert_eq!(book.ask().levels(), []);
    assert_eq!((last.price, last.cum_qty), (price("101"), None));

    assert_eq!(messages[5].body, MessageBody::Delay(DelayDirection::Down));
    assert_eq!(messages[5].symbol, symbol("B"));
}

#[test]
fn observed_time_is_the_running_maximum_over_the_whole_feed() {
    // B's seq 2 carries a later time than A's seq 3, and A's seq 4 an earlier one, as a
    // delay message does.
    let fixture = Fixture::new(
        vec![
            book("A", 1, 10),
            book("A", 3, 30),
            book("A", 4, 5),
            book("B", 2, 50),
            book("B", 5, 60),
        ],
        vec![],
    );
    let tree = fixture.write("observed");
    let times = |messages: &[Message]| -> Vec<(u64, i64, i64)> {
        messages
            .iter()
            .map(|message| {
                (
                    message.seq,
                    message.exch_ts.micros(),
                    message.observed_time.micros(),
                )
            })
            .collect()
    };
    let whole = tree.messages();
    assert_eq!(
        times(&whole),
        [
            (1, 10, 10),
            (2, 50, 50),
            (3, 30, 50),
            (4, 5, 50),
            (5, 60, 60)
        ]
    );

    // Selecting only A must not change what A's messages observed.
    let only_a: Vec<Message> = tree
        .load_with(Some(&["A"]), MissingLotSize::Reject)
        .unwrap()
        .messages()
        .collect();
    let expected: Vec<Message> = whole
        .iter()
        .copied()
        .filter(|message| message.symbol == symbol("A"))
        .collect();
    assert_eq!(only_a, expected);
    assert_eq!(times(&only_a), [(1, 10, 10), (3, 30, 50), (4, 5, 50)]);
}

#[test]
fn a_trade_only_row_raises_the_observed_time_of_a_selection() {
    let tree = Fixture::new(
        vec![
            book("A", 1, 10),
            BookRow {
                has_trade: true,
                ..book("B", 3, 40)
            },
        ],
        vec![trade("B", 2, 40, 100.5, 1), trade("B", 3, 40, 101.0, 1)],
    )
    .write("observed-trades");
    let only_b = tree
        .load_with(Some(&["B", "B"]), MissingLotSize::Reject)
        .unwrap();
    let observed: Vec<i64> = only_b
        .messages()
        .map(|message| message.observed_time.micros())
        .collect();
    assert_eq!(observed, [40, 40]);
    assert_eq!(only_b.symbols().collect::<Vec<_>>(), [symbol("B")]);
}

#[test]
fn selecting_symbols_without_rows_gives_no_messages() {
    let tree = Fixture::new(vec![book("A", 1, 10)], vec![]).write("empty");
    let day = tree
        .load_with(Some(&["Z"]), MissingLotSize::Reject)
        .unwrap();
    assert!(day.is_empty());
    assert_eq!(day.messages().len(), 0);
    assert_eq!(day.symbols().count(), 0);
}

#[test]
fn odd_lot_quantities_are_already_shares() {
    let mut fixture = Fixture::new(
        vec![BookRow {
            phase: "PERIODIC_AUCTION",
            has_trade: true,
            ..book("A", 1, 10)
        }],
        vec![TradeRow {
            phase: "PERIODIC_AUCTION",
            ..trade("A", 1, 10, 100.0, 250)
        }],
    );
    fixture.feed = Feed::TwseF23;
    // Odd lots need no lot size, so a missing one is not an error.
    fixture.instruments = vec![instrument("A", None)];
    let messages = fixture.write("odd").messages();
    let MessageBody::Disclosure {
        book,
        trade: Some(fill),
    } = messages[0].body
    else {
        panic!("{:?}", messages[0]);
    };
    assert_eq!(book.bid().levels(), [level("100", 7)]);
    assert_eq!(fill.qty, Shares::new(250));
    assert_eq!(fill.cum_qty, Some(Shares::new(250)));
}

#[test]
fn a_regular_board_symbol_without_a_lot_size_is_rejected_unless_assumed() {
    let mut fixture = Fixture::new(vec![book("A", 1, 10), book("C", 2, 20)], vec![]);
    // A has a null lot size; C has no instruments row at all.
    fixture.instruments = vec![instrument("A", None), instrument("B", Some(1_000))];
    let tree = fixture.write("lot");
    match tree.load() {
        Err(MdError::MissingLotSize {
            feed,
            symbol: missing,
            ..
        }) => {
            assert_eq!((feed, missing), (Feed::TwseF6, symbol("A")));
        }
        other => panic!("{other:?}"),
    }
    // A selection that avoids the symbols needs no assumption.
    assert!(tree.load_with(Some(&["B"]), MissingLotSize::Reject).is_ok());

    let day = tree.load_with(None, MissingLotSize::Assume(1_000)).unwrap();
    assert_eq!(
        day.assumed_lot_sizes().collect::<Vec<_>>(),
        [symbol("A"), symbol("C")]
    );
    let MessageBody::Disclosure { book, .. } = day.messages().next().unwrap().body else {
        panic!();
    };
    assert_eq!(book.bid().levels(), [level("100", 7_000)]);
}

#[test]
fn instruments_keep_what_the_source_provides() {
    let mut fixture = Fixture::new(vec![], vec![]);
    fixture.instruments = vec![
        InstrumentRow {
            kind: "ETF",
            lot: Some(100),
            currency: "USD",
            day_trade: Some("B"),
            non_10_par: Some(true),
            ..instrument("00636K", Some(100))
        },
        InstrumentRow {
            kind: "WARRANT_C",
            ref_price: Some(0.52),
            limit_up: Some(9_999.95),
            limit_down: Some(0.01),
            interval: Some(300),
            ..instrument("030001", Some(1_000))
        },
        InstrumentRow {
            kind: "OTHER",
            lot: None,
            // A price of 0 is the source's way of saying there is none.
            ref_price: Some(0.0),
            limit_up: None,
            limit_down: Some(0.0),
            interval: None,
            derived: true,
            // A derived day has none of the F1 fields.
            day_trade: None,
            non_10_par: None,
            ..instrument("A00102", None)
        },
    ];
    let tree = fixture.write("instruments");
    let instruments = tree.instruments().unwrap();
    assert_eq!(instruments.venue(), Venue::Twse);
    assert_eq!(instruments.date(), date());
    assert_eq!(instruments.manifest_entry().rows, 3);
    assert_eq!(instruments.records().len(), 3);
    assert!(instruments.get(symbol("2330")).is_none());

    let etf = instruments.get(symbol("00636K")).unwrap();
    assert_eq!(etf.instrument_type, InstrumentType::Etf);
    assert_eq!(etf.currency, Currency::Foreign);
    assert_eq!(etf.shares_per_lot, Some(100));
    assert_eq!(etf.match_interval_s, Some(0));
    assert!(!etf.derived);
    assert_eq!(etf.day_trade, Some(DayTradeFlag::BuyFirst));
    assert_eq!(etf.non_10_par, Some(true));

    let warrant = instruments.get(symbol("030001")).unwrap();
    assert_eq!(warrant.instrument_type, InstrumentType::WarrantCall);
    assert_eq!(warrant.currency, Currency::Twd);
    assert_eq!(warrant.ref_price, Some(price("0.52")));
    assert_eq!(warrant.limit_up, Some(price("9999.95")));
    assert_eq!(warrant.limit_down, Some(price("0.01")));
    assert_eq!(warrant.match_interval_s, Some(300));
    assert_eq!(warrant.day_trade, Some(DayTradeFlag::Both));
    assert_eq!(warrant.non_10_par, Some(false));

    let bond = instruments.get(symbol("A00102")).unwrap();
    assert_eq!(bond.instrument_type, InstrumentType::Other);
    assert_eq!(
        (
            bond.shares_per_lot,
            bond.ref_price,
            bond.limit_up,
            bond.limit_down
        ),
        (None, None, None, None)
    );
    assert_eq!(bond.match_interval_s, None);
    assert!(bond.derived);
    assert_eq!((bond.day_trade, bond.non_10_par), (None, None));
}

#[test]
fn instruments_with_undefined_values_are_rejected() {
    for (name, row) in [
        (
            "type",
            InstrumentRow {
                kind: "FUTURE",
                ..instrument("A", Some(1_000))
            },
        ),
        (
            "price",
            InstrumentRow {
                ref_price: Some(1.234_56),
                ..instrument("A", Some(1_000))
            },
        ),
        ("lot", instrument("A", Some(0))),
        (
            "day-trade",
            InstrumentRow {
                day_trade: Some("C"),
                ..instrument("A", Some(1_000))
            },
        ),
        ("symbol", instrument("TOO-LONG!", Some(1_000))),
    ] {
        let mut fixture = Fixture::new(vec![], vec![]);
        fixture.instruments = vec![row];
        let tree = fixture.write(&format!("instrument-{name}"));
        assert!(
            matches!(tree.instruments(), Err(MdError::Inconsistent { .. })),
            "{name}"
        );
    }
    let mut fixture = Fixture::new(vec![], vec![]);
    fixture.instruments = vec![instrument("B", Some(1_000)), instrument("A", Some(1_000))];
    assert!(matches!(
        fixture.write("instrument-order").instruments(),
        Err(MdError::Inconsistent { .. })
    ));
}

#[test]
fn files_must_match_their_manifest_rows() {
    let mut fixture = Fixture::new(vec![book("A", 1, 10)], vec![]);
    fixture.manifest_rows = |table, rows| rows + i64::from(table == Table::Books);
    assert!(matches!(
        fixture.write("rows").load(),
        Err(MdError::Inconsistent { .. })
    ));
}

#[test]
fn a_day_or_feed_absent_from_the_manifest_has_no_data() {
    let tree = Fixture::new(vec![book("A", 1, 10)], vec![]).write("absent");
    let root = MdRoot::new(&tree.root);
    let manifest = DayManifest::load(&root, date()).unwrap();
    let instruments = tree.instruments().unwrap();
    // The fixture wrote only TWSE_F6; TWSE_F17 shares the venue's instruments.
    let warrants = FeedDay::load(
        &root,
        &manifest,
        Feed::TwseF17,
        &instruments,
        None,
        MissingLotSize::Reject,
    );
    assert!(matches!(
        warrants,
        Err(MdError::MissingManifestEntry { table: "books", .. })
    ));

    fs::remove_dir_all(root.manifest_dir(date())).unwrap();
    assert!(matches!(tree.load(), Err(MdError::MissingManifest { .. })));
}

#[test]
fn only_schema_version_1_with_the_documented_column_types_is_read() {
    let mut newer = Fixture::new(vec![book("A", 1, 10)], vec![]);
    newer.schema_version = "2";
    assert!(matches!(
        newer.write("schema-version").load(),
        Err(MdError::Schema { .. })
    ));

    let mut retyped = Fixture::new(vec![book("A", 1, 10)], vec![]);
    retyped.tamper = |table, columns| {
        if table == Table::Books {
            let seq = columns.iter_mut().find(|column| column.0 == "seq").unwrap();
            seq.1 = Arc::new(Int32Array::from(vec![1]));
        }
    };
    match retyped.write("schema-type").load() {
        Err(MdError::Schema { reason, .. }) => assert!(reason.contains("seq"), "{reason}"),
        other => panic!("{other:?}"),
    }

    let mut dropped = Fixture::new(vec![book("A", 1, 10)], vec![trade("A", 1, 10, 100.0, 1)]);
    dropped.tamper = |table, columns| {
        if table == Table::Trades {
            columns.retain(|column| column.0 != "cum_qty");
        }
    };
    match dropped.write("schema-missing").load() {
        Err(MdError::Schema { reason, .. }) => assert!(reason.contains("cum_qty"), "{reason}"),
        other => panic!("{other:?}"),
    }

    // A column osmium does not read may be added within schema version 1.
    let mut extended = Fixture::new(vec![book("A", 1, 10)], vec![]);
    extended.tamper = |table, columns| {
        if table == Table::Books {
            columns.push(("bid_limit_state", Arc::new(Int8Array::from(vec![0]))));
        }
    };
    assert_eq!(extended.write("schema-extra").messages().len(), 1);
}

#[test]
fn rows_that_break_the_message_model_are_rejected() {
    let cases: Vec<(&str, Vec<BookRow>, Vec<TradeRow>, &str)> = vec![
        (
            "has-trade-alone",
            vec![BookRow {
                has_trade: true,
                ..book("A", 1, 10)
            }],
            vec![],
            "has_trade without a trades row",
        ),
        (
            "trade-not-flagged",
            vec![book("A", 1, 10)],
            vec![trade("A", 1, 10, 100.0, 1)],
            "without has_trade",
        ),
        (
            "auction-trade-alone",
            vec![],
            vec![TradeRow {
                phase: "CLOSE_AUCTION",
                ..trade("A", 1, 10, 100.0, 1)
            }],
            "outside continuous trading",
        ),
        (
            "pair-disagrees",
            vec![BookRow {
                has_trade: true,
                ..book("A", 1, 10)
            }],
            vec![trade("A", 1, 11, 100.0, 1)],
            "disagree",
        ),
        (
            "duplicate-seq",
            vec![book("A", 1, 10), book("B", 1, 10)],
            vec![],
            "share one seq",
        ),
        (
            "seq-order",
            vec![book("A", 2, 10), book("A", 1, 10)],
            vec![],
            "strictly increasing seq",
        ),
        (
            "symbol-order",
            vec![book("B", 1, 10), book("A", 2, 10)],
            vec![],
            "not sorted by symbol",
        ),
        (
            "levels-unordered",
            vec![BookRow {
                bid: Some(vec![(99.5, 1), (100.0, 1)]),
                ..book("A", 1, 10)
            }],
            vec![],
            "strictly ordered",
        ),
        (
            "crossed",
            vec![BookRow {
                ask: Some(vec![(100.0, 1)]),
                ..book("A", 1, 10)
            }],
            vec![],
            "best bid is not below best ask",
        ),
        (
            "price-inexact",
            vec![BookRow {
                bid: Some(vec![(100.000_01, 1)]),
                ..book("A", 1, 10)
            }],
            vec![],
            "exact positive 1e-4 decimal",
        ),
        (
            "qty-zero",
            vec![BookRow {
                bid: Some(vec![(100.0, 0)]),
                ..book("A", 1, 10)
            }],
            vec![],
            "level qty 0",
        ),
        (
            "delay-with-book",
            vec![BookRow {
                delay_state: 1,
                ..book("A", 1, 10)
            }],
            vec![],
            "a delay row discloses a book",
        ),
        (
            "delay-state",
            vec![BookRow {
                bid: None,
                ask: None,
                delay_state: 2,
                ..book("A", 1, 10)
            }],
            vec![],
            "unknown delay_state 2",
        ),
        (
            "no-book",
            vec![BookRow {
                bid: None,
                ask: None,
                ..book("A", 1, 10)
            }],
            vec![],
            "discloses no book",
        ),
        (
            "indicative-trade",
            vec![BookRow {
                indicative: true,
                has_trade: true,
                ..book("A", 1, 10)
            }],
            vec![trade("A", 1, 10, 100.0, 1)],
            "an indicative row is marked as a trade",
        ),
        (
            "match-on-disclosure",
            vec![BookRow {
                matched: Some((100.0, 1)),
                ..book("A", 1, 10)
            }],
            vec![],
            "a non-indicative row carries an indicative match",
        ),
        (
            "delayed-open-on-disclosure",
            vec![BookRow {
                delay_open: Some(true),
                ..book("A", 1, 10)
            }],
            vec![],
            "announces a delayed open or close",
        ),
        (
            "phase",
            vec![BookRow {
                phase: "AFTER_HOURS",
                ..book("A", 1, 10)
            }],
            vec![],
            "unknown phase AFTER_HOURS",
        ),
        (
            "sub-seq",
            vec![BookRow {
                has_trade: true,
                ..book("A", 1, 10)
            }],
            vec![TradeRow {
                sub_seq: 1,
                ..trade("A", 1, 10, 100.0, 1)
            }],
            "several fills under one seq",
        ),
    ];
    for (name, books, trades, expected) in cases {
        let mut fixture = Fixture::new(books, trades);
        let reason = invalid_row(fixture.write(&format!("invalid-{name}")).load());
        assert!(reason.contains(expected), "{name}: {reason}");
        // The same row is the first one wrong when every row is a row group of its own.
        fixture.row_group_rows = Some(1);
        let split = invalid_row(fixture.write(&format!("invalid-split-{name}")).load());
        assert_eq!(split, reason, "{name}");
    }
}

#[test]
fn the_messages_do_not_depend_on_how_the_files_are_split_into_row_groups() {
    // Row groups are read independently of each other and then put together; a symbol's
    // rows, a sweep and a joined `trades` row may all be cut by a row group boundary.
    let unordered_times = Fixture::new(
        vec![
            book("A", 1, 10),
            book("A", 3, 30),
            book("A", 4, 5),
            book("B", 2, 50),
            book("B", 5, 60),
        ],
        vec![],
    );
    for (name, mut fixture) in [
        ("kinds", kinds()),
        ("sweeps", sweeps()),
        ("times", unordered_times),
    ] {
        let read = |fixture: &Fixture, rows: usize| -> Vec<Vec<Message>> {
            let tree = fixture.write(&format!("split-{name}-{rows}"));
            [None, Some(&["A"][..]), Some(&["B"][..])]
                .into_iter()
                .map(|selection| {
                    tree.load_with(selection, MissingLotSize::Reject)
                        .unwrap()
                        .messages()
                        .collect()
                })
                .collect()
        };
        let whole = read(&fixture, 0);
        assert!(!whole[0].is_empty());
        for rows in 1..=4 {
            fixture.row_group_rows = Some(rows);
            assert_eq!(
                read(&fixture, rows),
                whole,
                "{name}, {rows} rows a row group"
            );
        }
        // The files really are split: one row a row group at the smallest size.
        fixture.row_group_rows = Some(1);
        let tree = fixture.write(&format!("split-{name}-groups"));
        let books = MdRoot::new(&tree.root).file(Table::Books, date(), tree.feed.part());
        let groups = SerializedFileReader::new(fs::File::open(books).unwrap())
            .unwrap()
            .metadata()
            .num_row_groups();
        assert_eq!(groups, fixture.books.len(), "{name}");
    }
}

#[test]
fn a_first_level_without_a_price_is_the_market_order_quantity() {
    // Days converted from a source that records the exchange's market-order level with a
    // null price keep it as the first level instead of `*_mkt_qty`.
    let tree = Fixture::new(
        vec![
            BookRow {
                bid: Some(vec![(f64::NAN, 20), (100.0, 7), (99.5, 2)]),
                ask: Some(vec![]),
                ..book("A", 1, 10)
            },
            BookRow {
                bid: Some(vec![(100.0, 7)]),
                ask: Some(vec![(f64::NAN, 4)]),
                ..book("A", 2, 20)
            },
        ],
        vec![],
    )
    .write("market-level");
    let messages = tree.messages();
    let MessageBody::Disclosure { book: first, .. } = messages[0].body else {
        panic!("{:?}", messages[0]);
    };
    assert_eq!(first.bid().market_qty(), Shares::new(20_000));
    assert_eq!(
        first.bid().levels(),
        [level("100", 7_000), level("99.5", 2_000)]
    );
    assert_eq!(first.ask().levels(), []);
    let MessageBody::Disclosure { book: second, .. } = messages[1].body else {
        panic!("{:?}", messages[1]);
    };
    assert_eq!(second.ask().market_qty(), Shares::new(4_000));
    assert_eq!(second.ask().levels(), []);
    assert_eq!(second.bid().levels(), [level("100", 7_000)]);

    // A price may be missing only there.
    for (name, row) in [
        (
            "second",
            BookRow {
                bid: Some(vec![(100.0, 7), (f64::NAN, 2)]),
                ..book("A", 1, 10)
            },
        ),
        (
            "both",
            BookRow {
                bid: Some(vec![(f64::NAN, 7), (100.0, 2)]),
                bid_mkt: Some(3),
                ..book("A", 1, 10)
            },
        ),
    ] {
        let tree = Fixture::new(vec![row], vec![]).write(&format!("market-level-{name}"));
        let reason = invalid_row(tree.load());
        assert!(
            reason.contains("disclosed level has a null price"),
            "{name}: {reason}"
        );
    }
}

/// Fills with an empty book, as days converted from a source without the continuation
/// flag carry them: every fill is in `books`, and the fills before the last of a sweep
/// have zero levels on both sides.
fn sweeps() -> Fixture {
    let empty = |symbol, seq, ts| BookRow {
        has_trade: true,
        bid: Some(vec![]),
        ask: Some(vec![]),
        ..book(symbol, seq, ts)
    };
    Fixture::new(
        vec![
            // A sweep of three fills; the last discloses the book.
            empty("A", 1, 10),
            empty("A", 2, 10),
            BookRow {
                has_trade: true,
                ..book("A", 3, 10)
            },
            // A sweep stopped by the price-stabilisation band ends with a delay row.
            empty("A", 4, 20),
            BookRow {
                bid: None,
                ask: None,
                delay_state: 1,
                ..book("A", 5, 20)
            },
            // A trade that really empties the book: the symbol continues later, ...
            empty("A", 6, 30),
            book("A", 7, 31),
            // ... or with something else than a fill or a delay, ...
            empty("A", 8, 40),
            book("A", 9, 40),
            // ... or not at all; another symbol's fill at the same time does not count.
            empty("A", 10, 50),
            BookRow {
                has_trade: true,
                ..book("B", 11, 50)
            },
        ],
        vec![
            trade("A", 1, 10, 100.0, 1),
            trade("A", 2, 10, 100.5, 1),
            trade("A", 3, 10, 101.0, 1),
            trade("A", 4, 20, 101.5, 1),
            trade("A", 6, 30, 100.0, 1),
            trade("A", 8, 40, 100.0, 1),
            trade("A", 10, 50, 100.0, 1),
            trade("B", 11, 50, 100.0, 1),
        ],
    )
}

#[test]
fn a_fill_with_an_empty_book_is_intermediate_when_the_sweep_continues() {
    let tree = sweeps().write("intermediate");
    let kinds: Vec<&str> = tree
        .messages()
        .iter()
        .map(|message| match message.body {
            MessageBody::IntermediateTrade(_) => "intermediate",
            MessageBody::Disclosure {
                book,
                trade: Some(_),
            } if book == Book::EMPTY => "emptied",
            MessageBody::Disclosure { trade: Some(_), .. } => "fill",
            MessageBody::Disclosure { trade: None, .. } => "book",
            MessageBody::Delay(_) => "delay",
            MessageBody::Indicative { .. } => "indicative",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "intermediate",
            "intermediate",
            "fill",
            "intermediate",
            "delay",
            "emptied",
            "book",
            "emptied",
            "book",
            "emptied",
            "fill",
        ]
    );
    let MessageBody::IntermediateTrade(first) = tree.messages()[0].body else {
        panic!();
    };
    assert_eq!((first.price, first.qty), (price("100"), Shares::new(1_000)));
}

#[test]
fn a_call_auction_fill_with_an_empty_book_is_not_intermediate() {
    let auction = |seq, ts, has_book: bool| BookRow {
        phase: "PERIODIC_AUCTION",
        has_trade: true,
        bid: Some(if has_book { vec![(100.0, 7)] } else { vec![] }),
        ask: Some(vec![]),
        ..book("A", seq, ts)
    };
    let fill = |seq, ts| TradeRow {
        phase: "PERIODIC_AUCTION",
        ..trade("A", seq, ts, 100.0, 1)
    };
    let tree = Fixture::new(
        vec![auction(1, 10, false), auction(2, 10, true)],
        vec![fill(1, 10), fill(2, 10)],
    )
    .write("auction-empty");
    assert!(matches!(
        tree.messages()[0].body,
        MessageBody::Disclosure { trade: Some(_), .. }
    ));
}

#[test]
fn source_fields_absent_in_older_data_stay_absent() {
    // Before the delay flags existed in the source, both are null on every row.
    let tree = Fixture::new(
        vec![BookRow {
            phase: "CLOSE_AUCTION",
            indicative: true,
            delay_open: None,
            delay_close: None,
            ..book("A", 1, 10)
        }],
        vec![],
    )
    .write("older");
    let MessageBody::Indicative {
        matched,
        delay_open,
        delay_close,
        ..
    } = tree.messages()[0].body
    else {
        panic!();
    };
    assert_eq!((matched, delay_open, delay_close), (None, None, None));
}
