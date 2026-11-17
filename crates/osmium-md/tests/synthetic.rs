//! End-to-end reads of a synthetic marketdata tree written by the test itself.

use std::{fs, path::PathBuf, sync::Arc};

use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    RecordBatch, StringArray, TimestampMicrosecondArray,
};
use market_types::{Symbol, TradingDate};
use osmium_md::{Feed, FeedDay, MdError, MdRoot, Table};
use parquet::{arrow::ArrowWriter, file::metadata::KeyValue, file::properties::WriterProperties};
use replay_engine::EventStream;

const DATE: &str = "2026-10-15";
const COMMIT: &str = "c0ffee";

struct Tree {
    root: PathBuf,
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write(path: &PathBuf, columns: Vec<(&str, ArrayRef)>) -> i64 {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
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

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn timestamps(values: &[i64]) -> ArrayRef {
    Arc::new(TimestampMicrosecondArray::from(values.to_vec()).with_timezone("UTC"))
}

/// Books for symbols A (seq 1, 3) and B (seq 2). B's message at seq 2 carries a later
/// `exch_ts` than A's next message, as a staggered close-auction result would.
fn books() -> Vec<(&'static str, ArrayRef)> {
    let n = 3;
    let mut columns: Vec<(&str, ArrayRef)> = vec![
        ("symbol", strings(&["A", "A", "B"])),
        ("exch_ts", timestamps(&[10, 30, 50])),
        ("seq", Arc::new(Int64Array::from(vec![1, 3, 2]))),
        ("phase", strings(&["CONTINUOUS"; 3])),
        (
            "has_trade",
            Arc::new(BooleanArray::from(vec![false, true, false])),
        ),
        ("n_bid", Arc::new(Int8Array::from(vec![1; n]))),
        ("n_ask", Arc::new(Int8Array::from(vec![0; n]))),
        ("bid_mkt_qty", Arc::new(Int64Array::from(vec![None; n]))),
        ("ask_mkt_qty", Arc::new(Int64Array::from(vec![None; n]))),
        (
            "is_indicative",
            Arc::new(BooleanArray::from(vec![false; n])),
        ),
        ("indicative_px", Arc::new(Float64Array::from(vec![None; n]))),
        ("indicative_qty", Arc::new(Int64Array::from(vec![None; n]))),
        (
            "delay_open",
            Arc::new(BooleanArray::from(vec![Some(false); n])),
        ),
        (
            "delay_close",
            Arc::new(BooleanArray::from(vec![Some(false); n])),
        ),
        ("delay_state", Arc::new(Int8Array::from(vec![0; n]))),
    ];
    for side in ["bid", "ask"] {
        for level in 1..=5 {
            let (price, quantity) = if side == "bid" && level == 1 {
                (vec![Some(100.5); n], vec![Some(7); n])
            } else {
                (vec![None; n], vec![None; n])
            };
            let price_name: &'static str = format!("{side}_px_{level}").leak();
            let quantity_name: &'static str = format!("{side}_qty_{level}").leak();
            columns.push((price_name, Arc::new(Float64Array::from(price))));
            columns.push((quantity_name, Arc::new(Int64Array::from(quantity))));
        }
    }
    columns
}

fn trades() -> Vec<(&'static str, ArrayRef)> {
    vec![
        ("symbol", strings(&["A"])),
        ("exch_ts", timestamps(&[30])),
        ("seq", Arc::new(Int64Array::from(vec![3]))),
        ("sub_seq", Arc::new(Int16Array::from(vec![0]))),
        ("price", Arc::new(Float64Array::from(vec![100.5]))),
        ("qty", Arc::new(Int64Array::from(vec![2]))),
        ("cum_qty", Arc::new(Int64Array::from(vec![Some(2)]))),
        ("phase", strings(&["CONTINUOUS"])),
    ]
}

fn instruments() -> Vec<(&'static str, ArrayRef)> {
    vec![
        ("symbol", strings(&["A", "B"])),
        ("instrument_type", strings(&["stock", "stock"])),
        (
            "shares_per_lot",
            Arc::new(Int32Array::from(vec![Some(1000), None])),
        ),
        (
            "match_interval_s",
            Arc::new(Int32Array::from(vec![Some(0), Some(0)])),
        ),
        ("derived", Arc::new(BooleanArray::from(vec![false, false]))),
    ]
}

/// Writes the tree; `books_rows_override` corrupts the manifest row count for books.
fn tree(name: &str, books_rows_override: Option<i64>) -> Tree {
    let root = std::env::temp_dir().join(format!("osmium-md-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let md = MdRoot::new(&root);
    let date = TradingDate::parse(DATE).unwrap();
    let mut rows = Vec::new();
    for (table, part, columns) in [
        (Table::Books, "TWSE_F6", books()),
        (Table::Trades, "TWSE_F6", trades()),
        (Table::Instruments, "TWSE", instruments()),
    ] {
        let count = write(&md.file(table, date, part), columns);
        rows.push((table, part, count));
    }
    let len = rows.len();
    let column = |values: Vec<&str>| strings(&values);
    write(
        &md.manifest_dir(date).join("tick.parquet"),
        vec![
            ("job", column(vec!["tick"; len])),
            (
                "table",
                column(rows.iter().map(|row| row.0.as_str()).collect()),
            ),
            ("part", column(rows.iter().map(|row| row.1).collect())),
            (
                "path",
                strings(
                    &rows
                        .iter()
                        .map(|row| MdRoot::relative_file(row.0, date, row.1))
                        .collect::<Vec<_>>()
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                ),
            ),
            (
                "rows",
                Arc::new(Int64Array::from(
                    rows.iter()
                        .map(|row| match (row.0, books_rows_override) {
                            (Table::Books, Some(rows)) => rows,
                            _ => row.2,
                        })
                        .collect::<Vec<_>>(),
                )),
            ),
            ("partial", Arc::new(BooleanArray::from(vec![false; len]))),
            ("seq_gaps", Arc::new(Int64Array::from(vec![Some(0); len]))),
            ("seq_dup", Arc::new(Int64Array::from(vec![Some(0); len]))),
            ("schema_version", column(vec!["1"; len])),
            ("git_commit", column(vec![COMMIT; len])),
            ("code_hash", column(vec!["code"; len])),
            ("inputs_hash", column(vec!["inputs"; len])),
        ],
    );
    Tree { root }
}

fn load(tree: &Tree, symbols: &[&str]) -> Result<FeedDay, MdError> {
    let symbols: Vec<Symbol> = symbols.iter().map(|s| Symbol::new(s).unwrap()).collect();
    FeedDay::load(
        &MdRoot::new(&tree.root),
        TradingDate::parse(DATE).unwrap(),
        Feed::TwseF6,
        &symbols,
    )
}

#[test]
fn observed_time_spans_the_whole_feed_regardless_of_selection() {
    let tree = tree("observed", None);
    let a = Symbol::new("A").unwrap();
    let mut day = load(&tree, &["A"]).unwrap();
    assert_eq!(day.identity().books.rows, 3);
    let mut stream = day.take_stream(&a).unwrap();

    let first = stream.next_event().unwrap().unwrap();
    assert_eq!(first.source_sequence(), Some(1));
    assert_eq!(first.observed_time().as_unix_microseconds(), 10);

    let second = stream.next_event().unwrap().unwrap();
    assert_eq!(second.source_sequence(), Some(3));
    assert_eq!(second.match_time().as_unix_microseconds(), 30);
    // B's seq-2 message (exch_ts 50) was disseminated first, though B is not selected.
    assert_eq!(second.observed_time().as_unix_microseconds(), 50);
    assert!(stream.next_event().unwrap().is_none());

    let mut both = load(&tree, &["B", "A"]).unwrap();
    let mut again = both.take_stream(&a).unwrap();
    again.next_event().unwrap();
    assert_eq!(again.next_event().unwrap().unwrap(), second);
}

#[test]
fn unknown_symbols_get_empty_streams() {
    let tree = tree("empty", None);
    let missing = Symbol::new("Z").unwrap();
    let mut day = load(&tree, &["Z"]).unwrap();
    assert!(
        day.take_stream(&missing)
            .unwrap()
            .next_event()
            .unwrap()
            .is_none()
    );
}

#[test]
fn files_must_match_their_manifest_rows() {
    let tree = tree("mismatch", Some(4));
    assert!(matches!(
        load(&tree, &["A"]),
        Err(MdError::Inconsistent { .. })
    ));
}

#[test]
fn a_day_without_manifest_has_no_data() {
    let tree = tree("nomanifest", None);
    fs::remove_dir_all(MdRoot::new(&tree.root).manifest_dir(TradingDate::parse(DATE).unwrap()))
        .unwrap();
    assert!(matches!(
        load(&tree, &["A"]),
        Err(MdError::MissingManifest { .. })
    ));
}
