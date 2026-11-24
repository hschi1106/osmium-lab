//! The reference tables: synthetic trees written by the tests, and a real marketdata
//! tree for the ignored tests:
//!
//! ```text
//! OSMIUM_MD_ROOT=/srv/md/v1 cargo test --release --locked -p osmium-md --test reference -- --ignored --nocapture
//! ```

use std::{fs, path::PathBuf, sync::Arc};

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int8Array, Int64Array,
    RecordBatch, StringArray,
};
use osmium_md::{
    CorporateActions, CreditQuotas, DEFAULT_MD_ROOT, DailyQuotes, MarketEvents, MdError, MdRoot,
    TradingCalendar,
};
use osmium_types::{
    CorporateActionKind, Date, Decimal8, MarketEventKind, PerShare, Price, Symbol, Venue,
};
use parquet::{arrow::ArrowWriter, file::metadata::KeyValue, file::properties::WriterProperties};

type Columns = Vec<(&'static str, ArrayRef)>;

fn day(text: &str) -> Date {
    text.parse().unwrap()
}

fn price(text: &str) -> Price {
    text.parse().unwrap()
}

fn symbol(code: &str) -> Symbol {
    Symbol::new(code).unwrap()
}

fn strings<'a>(values: impl IntoIterator<Item = &'a str>) -> ArrayRef {
    Arc::new(StringArray::from(values.into_iter().collect::<Vec<_>>()))
}

fn dates<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> ArrayRef {
    Arc::new(Date32Array::from_iter(
        values
            .into_iter()
            .map(|value| value.map(|text| day(text).epoch_days())),
    ))
}

fn floats(values: impl IntoIterator<Item = Option<f64>>) -> ArrayRef {
    Arc::new(Float64Array::from_iter(values))
}

struct Tree(PathBuf);

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Tree {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("osmium-md-ref-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        Self(root)
    }

    fn root(&self) -> MdRoot {
        MdRoot::new(&self.0)
    }

    fn write_as(&self, relative: &str, schema_version: &str, columns: Columns) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let batch = RecordBatch::try_from_iter(columns).unwrap();
        let properties = WriterProperties::builder()
            .set_key_value_metadata(Some(vec![
                KeyValue::new("md.schema_version".to_owned(), schema_version.to_owned()),
                KeyValue::new("md.git_commit".to_owned(), "c0ffee".to_owned()),
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
    }

    fn write(&self, relative: &str, columns: Columns) {
        self.write_as(relative, "1", columns);
    }
}

fn calendar_columns(rows: &[(&'static str, bool, bool)]) -> Columns {
    vec![
        ("trade_date", dates(rows.iter().map(|row| Some(row.0)))),
        (
            "twse_open",
            Arc::new(BooleanArray::from_iter(rows.iter().map(|row| Some(row.1)))),
        ),
        (
            "tpex_open",
            Arc::new(BooleanArray::from_iter(rows.iter().map(|row| Some(row.2)))),
        ),
    ]
}

#[test]
fn the_calendar_lists_open_and_closed_days() {
    let tree = Tree::new("calendar");
    tree.write(
        TradingCalendar::PATH,
        calendar_columns(&[
            ("2024-02-05", true, true),
            ("2024-02-06", false, false),
            ("2024-02-15", true, false),
        ]),
    );
    let calendar = TradingCalendar::load(&tree.root()).unwrap();
    assert_eq!(calendar.days().len(), 3);
    assert_eq!(calendar.file().path, "qc/calendar.parquet");
    assert_eq!(calendar.file().rows, 3);
    assert_eq!(calendar.file().git_commit, "c0ffee");
    assert_eq!(
        calendar.trading_days(Venue::Twse).collect::<Vec<_>>(),
        [day("2024-02-05"), day("2024-02-15")]
    );
    assert_eq!(
        calendar.trading_days(Venue::Tpex).collect::<Vec<_>>(),
        [day("2024-02-05")]
    );

    let tree = Tree::new("calendar-order");
    tree.write(
        TradingCalendar::PATH,
        calendar_columns(&[("2024-02-06", true, true), ("2024-02-05", true, true)]),
    );
    assert!(matches!(
        TradingCalendar::load(&tree.root()),
        Err(MdError::Inconsistent { .. })
    ));
}

#[test]
fn only_schema_version_1_with_the_documented_types_is_read() {
    let tree = Tree::new("version");
    tree.write_as(
        TradingCalendar::PATH,
        "2",
        calendar_columns(&[("2024-02-05", true, true)]),
    );
    assert!(matches!(
        TradingCalendar::load(&tree.root()),
        Err(MdError::Schema { .. })
    ));

    let tree = Tree::new("types");
    tree.write(
        TradingCalendar::PATH,
        vec![
            ("trade_date", strings(["2024-02-05"])),
            ("twse_open", Arc::new(BooleanArray::from(vec![true]))),
            ("tpex_open", Arc::new(BooleanArray::from(vec![true]))),
        ],
    );
    assert!(matches!(
        TradingCalendar::load(&tree.root()),
        Err(MdError::Schema { .. })
    ));

    assert!(matches!(
        TradingCalendar::load(&Tree::new("missing").root()),
        Err(MdError::Io { .. })
    ));
}

struct ActionRow {
    symbol: &'static str,
    ex_date: &'static str,
    action: &'static str,
    before: Option<f64>,
    reference: Option<f64>,
    after: Option<f64>,
    cash: Option<f64>,
    pay_date: Option<&'static str>,
    /// Decimals in units of 1e-8.
    stock_per_1000: Option<i128>,
    value: Option<f64>,
    shares_old: Option<i128>,
    shares_new: Option<i128>,
    refund: Option<i128>,
}

fn dividend(symbol: &'static str, ex_date: &'static str) -> ActionRow {
    ActionRow {
        symbol,
        ex_date,
        action: "dividend",
        before: Some(100.0),
        reference: Some(97.25),
        after: Some(97.25),
        cash: Some(2.75),
        pay_date: Some("2024-07-26"),
        stock_per_1000: None,
        value: Some(2.75),
        shares_old: None,
        shares_new: None,
        refund: None,
    }
}

fn decimals(values: impl IntoIterator<Item = Option<i128>>) -> ArrayRef {
    Arc::new(
        Decimal128Array::from_iter(values)
            .with_precision_and_scale(18, 8)
            .unwrap(),
    )
}

fn action_columns(rows: &[ActionRow]) -> Columns {
    vec![
        ("symbol", strings(rows.iter().map(|row| row.symbol))),
        ("ex_date", dates(rows.iter().map(|row| Some(row.ex_date)))),
        ("action", strings(rows.iter().map(|row| row.action))),
        ("before_price", floats(rows.iter().map(|row| row.before))),
        ("ref_price", floats(rows.iter().map(|row| row.reference))),
        ("after_price", floats(rows.iter().map(|row| row.after))),
        ("cash_dividend", floats(rows.iter().map(|row| row.cash))),
        ("pay_date", dates(rows.iter().map(|row| row.pay_date))),
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
    ]
}

#[test]
fn corporate_actions_are_exact_decimals() {
    let tree = Tree::new("actions");
    tree.write(
        CorporateActions::PATH,
        action_columns(&[
            // Too old to matter: it is not converted, so its bad value does not count.
            ActionRow {
                cash: Some(0.123_456_789_012_3),
                ..dividend("1101", "2019-07-01")
            },
            ActionRow {
                after: Some(96.8),
                stock_per_1000: Some(2_001_302_638),
                ..dividend("1101", "2024-07-01")
            },
            ActionRow {
                action: "reduction_cash",
                before: Some(27.7),
                reference: Some(39.71),
                after: None,
                cash: None,
                pay_date: None,
                value: None,
                shares_old: Some(100_000_000_000),
                shares_new: Some(59_568_140_000),
                refund: Some(404_318_600),
                ..dividend("4930", "2025-06-18")
            },
            // The value of a subscription right that is worth less than nothing.
            ActionRow {
                reference: Some(100.0),
                cash: None,
                value: Some(-0.170_421),
                ..dividend("1312A", "2026-06-11")
            },
        ]),
    );
    let actions = CorporateActions::load(&tree.root(), day("2022-01-03")).unwrap();
    assert_eq!(actions.file().rows, 4);
    let [first, second, third] = actions.rows() else {
        panic!("three rows from 2022 on");
    };
    assert_eq!(first.symbol, symbol("1101"));
    assert_eq!(first.ex_date, day("2024-07-01"));
    assert_eq!(first.kind, CorporateActionKind::Dividend);
    assert_eq!(first.before_price, price("100"));
    assert_eq!(first.ref_price, price("97.25"));
    assert_eq!(first.after_price, Some(price("96.8")));
    assert_eq!(
        first.cash_dividend,
        Some(PerShare::from_units(27_500_000_000))
    );
    assert_eq!(first.pay_date, Some(day("2024-07-26")));
    assert_eq!(
        first.stock_per_1000,
        Some(Decimal8::from_units(2_001_302_638))
    );
    assert_eq!(first.value, Some(PerShare::from_units(27_500_000_000)));
    assert_eq!((first.share_ratio, first.refund_per_share), (None, None));
    assert_eq!(second.kind, CorporateActionKind::ReductionCash);
    assert_eq!(
        (second.after_price, second.cash_dividend, second.pay_date),
        (None, None, None)
    );
    assert_eq!(
        second.share_ratio,
        Some((
            Decimal8::from_units(59_568_140_000),
            Decimal8::from_units(100_000_000_000)
        ))
    );
    // 4.043186 yuan.
    assert_eq!(
        second.refund_per_share,
        Some(PerShare::from_units(40_431_860_000))
    );
    assert_eq!((second.value, third.value), (None, None));

    for (name, row) in [
        (
            "inexact",
            ActionRow {
                cash: Some(0.123_456_789_012_3),
                ..dividend("1101", "2024-07-01")
            },
        ),
        (
            "action",
            ActionRow {
                action: "merger",
                ..dividend("1101", "2024-07-01")
            },
        ),
        (
            "price",
            ActionRow {
                reference: None,
                ..dividend("1101", "2024-07-01")
            },
        ),
        (
            "half-ratio",
            ActionRow {
                shares_new: Some(80_000_000_000),
                ..dividend("1101", "2024-07-01")
            },
        ),
        (
            "zero-ratio",
            ActionRow {
                shares_old: Some(100_000_000_000),
                shares_new: Some(0),
                ..dividend("1101", "2024-07-01")
            },
        ),
        (
            "negative-refund",
            ActionRow {
                refund: Some(-1),
                ..dividend("1101", "2024-07-01")
            },
        ),
    ] {
        let tree = Tree::new(&format!("actions-{name}"));
        tree.write(CorporateActions::PATH, action_columns(&[row]));
        assert!(
            matches!(
                CorporateActions::load(&tree.root(), day("2022-01-03")),
                Err(MdError::Inconsistent { .. })
            ),
            "{name}"
        );
    }
}

type EventRow = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<i8>,
);

fn event_columns(rows: &[EventRow]) -> Columns {
    vec![
        ("symbol", strings(rows.iter().map(|row| row.0))),
        ("event", strings(rows.iter().map(|row| row.1))),
        ("start_date", dates(rows.iter().map(|row| Some(row.2)))),
        ("end_date", dates(rows.iter().map(|row| row.3))),
        (
            "disp_nth",
            Arc::new(Int8Array::from_iter(rows.iter().map(|row| row.4))),
        ),
    ]
}

#[test]
fn events_that_can_still_be_in_force_are_read() {
    let tree = Tree::new("events");
    tree.write(
        MarketEvents::PATH,
        event_columns(&[
            (
                "1101",
                "disposition",
                "2023-12-20",
                Some("2024-01-05"),
                Some(2),
            ),
            (
                "1101",
                "disposition",
                "2023-11-01",
                Some("2023-11-14"),
                Some(1),
            ),
            ("1102", "short_suspended", "2023-12-28", None, None),
            ("1102", "halt", "2020-03-02", None, None),
            (
                "1103",
                "daytrade_suspended",
                "2024-03-01",
                Some("2024-03-01"),
                None,
            ),
        ]),
    );
    let events = MarketEvents::load(&tree.root(), day("2024-01-02")).unwrap();
    let kinds: Vec<_> = events
        .rows()
        .iter()
        .map(|event| (event.symbol, event.kind, event.start, event.end, event.nth))
        .collect();
    assert_eq!(
        kinds,
        [
            (
                symbol("1101"),
                MarketEventKind::Disposition,
                day("2023-12-20"),
                Some(day("2024-01-05")),
                Some(2)
            ),
            (
                symbol("1102"),
                MarketEventKind::ShortSuspended,
                day("2023-12-28"),
                None,
                None
            ),
            (
                symbol("1103"),
                MarketEventKind::DayTradeSuspended,
                day("2024-03-01"),
                Some(day("2024-03-01")),
                None
            ),
        ]
    );

    for (name, row) in [
        ("kind", ("1101", "delisted", "2024-03-01", None, None)),
        (
            "order",
            ("1101", "halt", "2024-03-05", Some("2024-03-01"), None),
        ),
        ("nth", ("1101", "disposition", "2024-03-01", None, Some(0))),
        ("nth-kind", ("1101", "halt", "2024-03-01", None, Some(1))),
    ] {
        let tree = Tree::new(&format!("events-{name}"));
        tree.write(MarketEvents::PATH, event_columns(&[row]));
        assert!(
            matches!(
                MarketEvents::load(&tree.root(), day("2024-01-02")),
                Err(MdError::Inconsistent { .. })
            ),
            "{name}"
        );
    }
}

struct QuoteRow {
    date: &'static str,
    symbol: &'static str,
    reference: Option<f64>,
    limit_down: Option<f64>,
    close: Option<f64>,
    bid: Option<f64>,
    ask: Option<f64>,
}

fn quote(date: &'static str, symbol: &'static str, close: Option<f64>) -> QuoteRow {
    QuoteRow {
        date,
        symbol,
        reference: Some(100.0),
        limit_down: Some(90.0),
        close,
        bid: Some(99.5),
        ask: Some(100.5),
    }
}

fn quote_columns(rows: &[QuoteRow]) -> Columns {
    vec![
        ("trade_date", dates(rows.iter().map(|row| Some(row.date)))),
        ("symbol", strings(rows.iter().map(|row| row.symbol))),
        ("ref_price", floats(rows.iter().map(|row| row.reference))),
        ("limit_down", floats(rows.iter().map(|row| row.limit_down))),
        ("close", floats(rows.iter().map(|row| row.close))),
        ("close_bid_px", floats(rows.iter().map(|row| row.bid))),
        ("close_ask_px", floats(rows.iter().map(|row| row.ask))),
    ]
}

#[test]
fn daily_quotes_are_the_rows_of_one_date() {
    let tree = Tree::new("daily");
    tree.write(
        "daily/2024.parquet",
        quote_columns(&[
            quote("2024-03-04", "1101", Some(101.5)),
            quote("2024-03-05", "1101", Some(102.0)),
            QuoteRow {
                reference: Some(0.0),
                limit_down: None,
                bid: None,
                ask: Some(0.0),
                ..quote("2024-03-05", "1102", None)
            },
            quote("2024-03-06", "1101", Some(103.0)),
        ]),
    );
    let quotes = DailyQuotes::load(&tree.root(), day("2024-03-05")).unwrap();
    assert_eq!(quotes.date(), day("2024-03-05"));
    assert_eq!(quotes.file().path, "daily/2024.parquet");
    assert_eq!(quotes.rows().len(), 2);
    let traded = quotes.get(symbol("1101")).unwrap();
    assert_eq!(traded.close, Some(price("102")));
    assert_eq!(traded.close_bid, Some(price("99.5")));
    assert_eq!(traded.close_ask, Some(price("100.5")));
    assert_eq!(traded.ref_price, Some(price("100")));
    assert_eq!(traded.limit_down, Some(price("90")));
    // Null and 0 both mean there is no price.
    let idle = quotes.get(symbol("1102")).unwrap();
    assert_eq!(
        (
            idle.close,
            idle.close_bid,
            idle.close_ask,
            idle.ref_price,
            idle.limit_down
        ),
        (None, None, None, None, None)
    );
    assert!(quotes.get(symbol("2330")).is_none());
    assert!(
        DailyQuotes::load(&tree.root(), day("2024-03-07"))
            .unwrap()
            .rows()
            .is_empty()
    );
    // Another year is another file.
    assert!(matches!(
        DailyQuotes::load(&tree.root(), day("2025-03-05")),
        Err(MdError::Io { .. })
    ));

    for (name, rows) in [
        ("inexact", vec![quote("2024-03-05", "1101", Some(1.234_56))]),
        (
            "order",
            vec![
                quote("2024-03-05", "1102", Some(10.0)),
                quote("2024-03-05", "1101", Some(10.0)),
            ],
        ),
        (
            "duplicate",
            vec![
                quote("2024-03-05", "1101", Some(10.0)),
                quote("2024-03-05", "1101", Some(10.0)),
            ],
        ),
    ] {
        let tree = Tree::new(&format!("daily-{name}"));
        tree.write("daily/2024.parquet", quote_columns(&rows));
        assert!(
            matches!(
                DailyQuotes::load(&tree.root(), day("2024-03-05")),
                Err(MdError::Inconsistent { .. })
            ),
            "{name}"
        );
    }
}

#[test]
fn credit_quotas_are_the_rows_of_one_date() {
    let quota_columns =
        |rows: &[(&'static str, &'static str, Option<i64>, Option<i64>)]| -> Columns {
            vec![
                ("trade_date", dates(rows.iter().map(|row| Some(row.0)))),
                ("symbol", strings(rows.iter().map(|row| row.1))),
                (
                    "margin_limit",
                    Arc::new(Int64Array::from_iter(rows.iter().map(|row| row.2))),
                ),
                (
                    "short_limit",
                    Arc::new(Int64Array::from_iter(rows.iter().map(|row| row.3))),
                ),
            ]
        };
    let tree = Tree::new("quotas");
    tree.write(
        "daily_ext/2024.parquet",
        quota_columns(&[
            ("2024-03-04", "1101", Some(1), Some(1)),
            ("2024-03-05", "1101", Some(1_500_000), Some(0)),
            ("2024-03-05", "1102", None, None),
        ]),
    );
    let quotas = CreditQuotas::load(&tree.root(), day("2024-03-05")).unwrap();
    assert_eq!(quotas.rows().len(), 2);
    let listed = quotas.get(symbol("1101")).unwrap();
    assert_eq!(
        (listed.margin_limit, listed.short_limit),
        (Some(1_500_000), Some(0))
    );
    let unlisted = quotas.get(symbol("1102")).unwrap();
    assert_eq!((unlisted.margin_limit, unlisted.short_limit), (None, None));

    let tree = Tree::new("quotas-negative");
    tree.write(
        "daily_ext/2024.parquet",
        quota_columns(&[("2024-03-05", "1101", Some(-1), Some(0))]),
    );
    assert!(matches!(
        CreditQuotas::load(&tree.root(), day("2024-03-05")),
        Err(MdError::Inconsistent { .. })
    ));
}

fn real_root() -> MdRoot {
    MdRoot::new(std::env::var("OSMIUM_MD_ROOT").unwrap_or_else(|_| DEFAULT_MD_ROOT.to_owned()))
}

#[test]
#[ignore = "needs a marketdata tree"]
fn real_reference_tables_read_exactly() {
    let root = real_root();
    let calendar = TradingCalendar::load(&root).unwrap();
    let trading: Vec<Date> = calendar.trading_days(Venue::Twse).collect();
    let (first, last) = (trading[0], *trading.last().unwrap());
    println!(
        "calendar: {} rows, {} trading days, {first} to {last}",
        calendar.days().len(),
        trading.len()
    );
    assert!(
        calendar
            .days()
            .iter()
            .all(|day| day.twse_open == day.tpex_open)
    );

    let actions = CorporateActions::load(&root, first).unwrap();
    let mut by_kind = std::collections::BTreeMap::new();
    for action in actions.rows() {
        *by_kind.entry(action.kind.as_str()).or_insert(0_usize) += 1;
    }
    println!("corp_actions from {first}: {by_kind:?}");
    assert!(actions.rows().len() > 10_000);

    let events = MarketEvents::load(&root, first).unwrap();
    println!("events in force from {first}: {}", events.rows().len());
    assert!(events.rows().len() > 10_000);

    // Every twentieth trading day, and the first and the last.
    let mut quotes_read = 0;
    for (index, &date) in trading.iter().enumerate() {
        if index % 20 != 0 && date != last {
            continue;
        }
        let quotes = DailyQuotes::load(&root, date).unwrap();
        let quotas = CreditQuotas::load(&root, date).unwrap();
        assert!(quotes.rows().len() > 1_000, "{date}");
        assert!(
            quotas
                .rows()
                .iter()
                .filter(|quota| quota.margin_limit.is_some())
                .count()
                > 1_000,
            "{date}"
        );
        quotes_read += quotes.rows().len();
    }
    println!("daily quotes read: {quotes_read}");
}
