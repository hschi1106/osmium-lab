//! The reference tables of marketdata that are not part of the tick stream: the
//! trading calendar, daily closing quotes, credit quotas, trading restrictions and
//! corporate actions.
//!
//! These tables are whole files or one file per year and have no manifest rows. Each
//! file is checked against schema version 1 through its footer and the types of the
//! columns that are read.

use std::path::Path;

use arrow_array::types::{Date32Type, Decimal128Type, Float64Type, Int64Type};
use osmium_types::{
    CorporateAction, CorporateActionKind, CreditQuota, DailyQuote, Date, Decimal8, MarketEvent,
    MarketEventKind, PerShare, Price, Symbol, Venue,
};

use crate::{
    MdError, MdRoot, SCHEMA_VERSION,
    parquet_io::{
        Column, ColumnType, ParquetFile, boolean, optional, optional_bool, optional_str,
        price_units, primitive, required, strings,
    },
};

/// Which file a reference table was read from, for the record of a run.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceFile {
    /// Relative to the marketdata root.
    pub path: String,
    pub rows: u64,
    /// The footer's `md.git_commit`.
    pub git_commit: String,
}

fn open(root: &MdRoot, relative: &str) -> Result<(ParquetFile, ReferenceFile), MdError> {
    let file = ParquetFile::open(&root.path().join(relative))?;
    let version = file.metadata_value("md.schema_version");
    if version != Some(SCHEMA_VERSION) {
        return Err(MdError::schema(
            file.path(),
            format!("md.schema_version {version:?} is not the supported version {SCHEMA_VERSION}"),
        ));
    }
    let git_commit = file
        .metadata_value("md.git_commit")
        .ok_or_else(|| MdError::schema(file.path(), "the footer has no md.git_commit".to_owned()))?
        .to_owned();
    let rows = u64::try_from(file.num_rows())
        .map_err(|_| MdError::inconsistent(file.path(), "a negative row count".to_owned()))?;
    let identity = ReferenceFile {
        path: relative.to_owned(),
        rows,
        git_commit,
    };
    Ok((file, identity))
}

fn date(path: &Path, value: Option<i32>, name: &str) -> Result<Date, MdError> {
    let days = required(path, value, name)?;
    Date::from_epoch_days(days)
        .map_err(|_| MdError::inconsistent(path, format!("{name} {days} is not a date")))
}

fn optional_date(path: &Path, value: Option<i32>, name: &str) -> Result<Option<Date>, MdError> {
    value.map(|days| date(path, Some(days), name)).transpose()
}

fn symbol(path: &Path, code: Option<&str>) -> Result<Symbol, MdError> {
    let code = required(path, code, "symbol")?;
    Symbol::new(code)
        .map_err(|error| MdError::inconsistent(path, format!("symbol {code}: {error}")))
}

/// A price cell that may be empty. Null and 0 both mean the source has no price.
fn optional_price(
    path: &Path,
    symbol: Symbol,
    name: &str,
    value: Option<f64>,
) -> Result<Option<Price>, MdError> {
    match value {
        None | Some(0.0) => Ok(None),
        Some(value) => price_units(value)
            .map(|units| Some(Price::from_units(i64::from(units))))
            .ok_or_else(|| {
                MdError::inconsistent(
                    path,
                    format!(
                        "symbol {symbol}: {name} {value} is not an exact positive 1e-4 decimal"
                    ),
                )
            }),
    }
}

/// Converts a float64 amount per share to exact units of 1e-10 yuan.
///
/// marketdata stores the nearest double to a decimal, so the conversion is accepted
/// only when it round-trips exactly.
fn per_share_units(value: f64) -> Option<i64> {
    if !value.is_finite() || !(0.0..=100_000.0).contains(&value) {
        return None;
    }
    let units = (value * 1e10).round();
    #[allow(clippy::float_cmp)]
    let exact = units / 1e10 == value;
    #[allow(clippy::cast_possible_truncation)]
    exact.then_some(units as i64)
}

/// The largest amount per share a decimal column may state, in yuan.
const MAX_PER_SHARE: i64 = 100_000;

/// One row of marketdata `qc/calendar`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarDay {
    pub date: Date,
    pub twse_open: bool,
    pub tpex_open: bool,
}

/// marketdata `qc/calendar`: every weekday marketdata knows, open or closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradingCalendar {
    file: ReferenceFile,
    days: Vec<CalendarDay>,
}

impl TradingCalendar {
    pub const PATH: &'static str = "qc/calendar.parquet";

    pub fn load(root: &MdRoot) -> Result<Self, MdError> {
        const COLUMNS: &[Column] = &[
            ("trade_date", ColumnType::Date32),
            ("twse_open", ColumnType::Bool),
            ("tpex_open", ColumnType::Bool),
        ];
        let (file, identity) = open(root, Self::PATH)?;
        let mut days: Vec<CalendarDay> = Vec::new();
        file.read(COLUMNS, None, |path, batch| {
            let trade_date = primitive::<Date32Type>(path, batch, "trade_date")?;
            let twse = boolean(path, batch, "twse_open")?;
            let tpex = boolean(path, batch, "tpex_open")?;
            for row in 0..batch.num_rows() {
                let day = CalendarDay {
                    date: date(path, optional(trade_date, row), "trade_date")?,
                    twse_open: required(path, optional_bool(twse, row), "twse_open")?,
                    tpex_open: required(path, optional_bool(tpex, row), "tpex_open")?,
                };
                if days.last().is_some_and(|last| last.date >= day.date) {
                    return Err(MdError::inconsistent(
                        path,
                        format!("trade_date {} is not after the row before it", day.date),
                    ));
                }
                days.push(day);
            }
            Ok(())
        })?;
        Ok(Self {
            file: identity,
            days,
        })
    }

    #[must_use]
    pub const fn file(&self) -> &ReferenceFile {
        &self.file
    }

    /// Every row, in date order.
    #[must_use]
    pub fn days(&self) -> &[CalendarDay] {
        &self.days
    }

    /// The days `venue` is open, in order.
    pub fn trading_days(&self, venue: Venue) -> impl Iterator<Item = Date> + '_ {
        self.days
            .iter()
            .filter(move |day| match venue {
                Venue::Twse => day.twse_open,
                Venue::Tpex => day.tpex_open,
            })
            .map(|day| day.date)
    }
}

/// marketdata `corp_actions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorporateActions {
    file: ReferenceFile,
    rows: Vec<CorporateAction>,
}

impl CorporateActions {
    pub const PATH: &'static str = "corp_actions.parquet";

    /// Reads the rows whose `ex_date` is on or after `from`. Rows before it are not
    /// converted, so old rows that cannot be represented do not stop a run.
    pub fn load(root: &MdRoot, from: Date) -> Result<Self, MdError> {
        const COLUMNS: &[Column] = &[
            ("symbol", ColumnType::String),
            ("ex_date", ColumnType::Date32),
            ("action", ColumnType::String),
            ("before_price", ColumnType::Float64),
            ("ref_price", ColumnType::Float64),
            ("after_price", ColumnType::Float64),
            ("cash_dividend", ColumnType::Float64),
            ("pay_date", ColumnType::Date32),
            ("stock_per_1000", ColumnType::Decimal18x8),
            ("value", ColumnType::Float64),
            ("shares_old", ColumnType::Decimal18x8),
            ("shares_new", ColumnType::Decimal18x8),
            ("refund_per_share", ColumnType::Decimal18x8),
        ];
        let (file, identity) = open(root, Self::PATH)?;
        let mut rows: Vec<CorporateAction> = Vec::new();
        file.read(COLUMNS, None, |path, batch| {
            let symbols = strings(path, batch, "symbol")?;
            let ex_date = primitive::<Date32Type>(path, batch, "ex_date")?;
            let action = strings(path, batch, "action")?;
            let before = primitive::<Float64Type>(path, batch, "before_price")?;
            let reference = primitive::<Float64Type>(path, batch, "ref_price")?;
            let after = primitive::<Float64Type>(path, batch, "after_price")?;
            let cash = primitive::<Float64Type>(path, batch, "cash_dividend")?;
            let pay_date = primitive::<Date32Type>(path, batch, "pay_date")?;
            let stock = primitive::<Decimal128Type>(path, batch, "stock_per_1000")?;
            let value = primitive::<Float64Type>(path, batch, "value")?;
            let shares_old = primitive::<Decimal128Type>(path, batch, "shares_old")?;
            let shares_new = primitive::<Decimal128Type>(path, batch, "shares_new")?;
            let refund = primitive::<Decimal128Type>(path, batch, "refund_per_share")?;
            for row in 0..batch.num_rows() {
                let ex_date = date(path, optional(ex_date, row), "ex_date")?;
                if ex_date < from {
                    continue;
                }
                let symbol = symbol(path, optional_str(symbols, row))?;
                let invalid = |reason: String| {
                    MdError::inconsistent(path, format!("symbol {symbol} on {ex_date}: {reason}"))
                };
                let action = required(path, optional_str(action, row), "action")?;
                let price = |name: &str, value: Option<f64>| {
                    optional_price(path, symbol, name, value)?
                        .ok_or_else(|| invalid(format!("{name} is missing")))
                };
                let per_share = |name: &str, value: Option<f64>| {
                    value
                        .map(|value| {
                            per_share_units(value)
                                .map(PerShare::from_units)
                                .ok_or_else(|| {
                                    invalid(format!("{name} {value} is not an exact 1e-10 decimal"))
                                })
                        })
                        .transpose()
                };
                // A positive decimal of at most `MAX_PER_SHARE`, or zero when `zero` is
                // allowed.
                let decimal = |name: &str, value: Option<i128>, zero: bool| {
                    value
                        .map(|units| {
                            i64::try_from(units)
                                .ok()
                                .filter(|&units| {
                                    (if zero { 0 } else { 1 }) <= units
                                        && units <= MAX_PER_SHARE * Decimal8::UNITS_PER_ONE
                                })
                                .map(Decimal8::from_units)
                                .ok_or_else(|| {
                                    invalid(format!("{name} {units}e-8 is out of range"))
                                })
                        })
                        .transpose()
                };
                let share_ratio = match (
                    decimal("shares_new", optional(shares_new, row), false)?,
                    decimal("shares_old", optional(shares_old, row), false)?,
                ) {
                    (Some(new), Some(old)) => Some((new, old)),
                    (None, None) => None,
                    _ => return Err(invalid("only one of shares_old and shares_new".to_owned())),
                };
                let refund = decimal("refund_per_share", optional(refund, row), true)?;
                rows.push(CorporateAction {
                    symbol,
                    ex_date,
                    kind: CorporateActionKind::from_marketdata(action)
                        .ok_or_else(|| invalid(format!("unknown action {action}")))?,
                    before_price: price("before_price", optional(before, row))?,
                    ref_price: price("ref_price", optional(reference, row))?,
                    after_price: optional_price(path, symbol, "after_price", optional(after, row))?,
                    cash_dividend: per_share("cash_dividend", optional(cash, row))?,
                    pay_date: optional_date(path, optional(pay_date, row), "pay_date")?,
                    stock_per_1000: decimal("stock_per_1000", optional(stock, row), false)?,
                    // Subscription rights can be worth less than nothing.
                    value: per_share("value", optional(value, row).filter(|value| *value > 0.0))?,
                    share_ratio,
                    // 1e-8 yuan is 100 units of 1e-10.
                    refund_per_share: refund
                        .map(|refund| PerShare::from_units(refund.units() * 100)),
                });
            }
            Ok(())
        })?;
        Ok(Self {
            file: identity,
            rows,
        })
    }

    #[must_use]
    pub const fn file(&self) -> &ReferenceFile {
        &self.file
    }

    /// The rows in file order.
    #[must_use]
    pub fn rows(&self) -> &[CorporateAction] {
        &self.rows
    }
}

/// marketdata `events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketEvents {
    file: ReferenceFile,
    rows: Vec<MarketEvent>,
}

impl MarketEvents {
    pub const PATH: &'static str = "events.parquet";

    /// Reads the events that can still be in force on or after `from`: those that end
    /// on or after it, or have no end date and start within a year before it.
    pub fn load(root: &MdRoot, from: Date) -> Result<Self, MdError> {
        const COLUMNS: &[Column] = &[
            ("symbol", ColumnType::String),
            ("event", ColumnType::String),
            ("start_date", ColumnType::Date32),
            ("end_date", ColumnType::Date32),
        ];
        let (file, identity) = open(root, Self::PATH)?;
        let earliest_open_ended = from.epoch_days().saturating_sub(366);
        let mut rows: Vec<MarketEvent> = Vec::new();
        file.read(COLUMNS, None, |path, batch| {
            let symbols = strings(path, batch, "symbol")?;
            let event = strings(path, batch, "event")?;
            let start = primitive::<Date32Type>(path, batch, "start_date")?;
            let end = primitive::<Date32Type>(path, batch, "end_date")?;
            for row in 0..batch.num_rows() {
                let start = date(path, optional(start, row), "start_date")?;
                let end = optional_date(path, optional(end, row), "end_date")?;
                let relevant = match end {
                    Some(end) => end >= from,
                    None => start.epoch_days() >= earliest_open_ended,
                };
                if !relevant {
                    continue;
                }
                let symbol = symbol(path, optional_str(symbols, row))?;
                let invalid = |reason: String| {
                    MdError::inconsistent(path, format!("symbol {symbol} from {start}: {reason}"))
                };
                let event = required(path, optional_str(event, row), "event")?;
                if end.is_some_and(|end| end < start) {
                    return Err(invalid("end_date is before start_date".to_owned()));
                }
                rows.push(MarketEvent {
                    symbol,
                    kind: MarketEventKind::from_marketdata(event)
                        .ok_or_else(|| invalid(format!("unknown event {event}")))?,
                    start,
                    end,
                });
            }
            Ok(())
        })?;
        Ok(Self {
            file: identity,
            rows,
        })
    }

    #[must_use]
    pub const fn file(&self) -> &ReferenceFile {
        &self.file
    }

    /// The rows in file order.
    #[must_use]
    pub fn rows(&self) -> &[MarketEvent] {
        &self.rows
    }
}

fn year_file(table: &str, date: Date) -> String {
    format!("{table}/{}.parquet", date.ymd().0)
}

/// Reads the rows of one trade date from a table stored as one file per year, sorted
/// by `(trade_date, symbol)`. `row` is called with the symbol and the row index.
fn read_day<T>(
    root: &MdRoot,
    table: &str,
    on: Date,
    columns: &[Column],
    mut row: impl FnMut(&Path, &arrow_array::RecordBatch, usize, Symbol) -> Result<T, MdError>,
    symbol_of: fn(&T) -> Symbol,
) -> Result<(ReferenceFile, Vec<T>), MdError> {
    let (file, identity) = open(root, &year_file(table, on))?;
    let groups = file.row_groups_for_date(on.epoch_days());
    let mut rows: Vec<T> = Vec::new();
    file.read(columns, Some(groups), |path, batch| {
        let trade_date = primitive::<Date32Type>(path, batch, "trade_date")?;
        let symbols = strings(path, batch, "symbol")?;
        for index in 0..batch.num_rows() {
            if required(path, optional(trade_date, index), "trade_date")? != on.epoch_days() {
                continue;
            }
            let symbol = symbol(path, optional_str(symbols, index))?;
            if rows.last().is_some_and(|last| symbol_of(last) >= symbol) {
                return Err(MdError::inconsistent(
                    path,
                    format!("{on}: symbol {symbol} is not after the row before it"),
                ));
            }
            rows.push(row(path, batch, index, symbol)?);
        }
        Ok(())
    })?;
    Ok((identity, rows))
}

/// The marketdata `daily` rows of one trade date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyQuotes {
    date: Date,
    file: ReferenceFile,
    /// Sorted by symbol.
    rows: Vec<DailyQuote>,
}

impl DailyQuotes {
    /// Reads `daily/{YYYY}.parquet` for `date`. A date without rows gives no quotes.
    pub fn load(root: &MdRoot, date: Date) -> Result<Self, MdError> {
        const COLUMNS: &[Column] = &[
            ("trade_date", ColumnType::Date32),
            ("symbol", ColumnType::String),
            ("ref_price", ColumnType::Float64),
            ("limit_down", ColumnType::Float64),
            ("close", ColumnType::Float64),
            ("close_bid_px", ColumnType::Float64),
            ("close_ask_px", ColumnType::Float64),
        ];
        let (file, rows) = read_day(
            root,
            "daily",
            date,
            COLUMNS,
            |path, batch, index, symbol| {
                let price = |name: &str| {
                    let column = primitive::<Float64Type>(path, batch, name)?;
                    optional_price(path, symbol, name, optional(column, index))
                };
                Ok(DailyQuote {
                    symbol,
                    close: price("close")?,
                    close_bid: price("close_bid_px")?,
                    close_ask: price("close_ask_px")?,
                    ref_price: price("ref_price")?,
                    limit_down: price("limit_down")?,
                })
            },
            |quote| quote.symbol,
        )?;
        Ok(Self { date, file, rows })
    }

    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    #[must_use]
    pub const fn file(&self) -> &ReferenceFile {
        &self.file
    }

    #[must_use]
    pub fn get(&self, symbol: Symbol) -> Option<&DailyQuote> {
        self.rows
            .binary_search_by_key(&symbol, |quote| quote.symbol)
            .ok()
            .map(|index| &self.rows[index])
    }

    /// Every row of the date, in symbol order.
    #[must_use]
    pub fn rows(&self) -> &[DailyQuote] {
        &self.rows
    }
}

/// The marketdata `daily_ext` margin and short quotas of one trade date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditQuotas {
    date: Date,
    file: ReferenceFile,
    /// Sorted by symbol.
    rows: Vec<CreditQuota>,
}

impl CreditQuotas {
    /// Reads `daily_ext/{YYYY}.parquet` for `date`. A date without rows gives no
    /// quotas.
    pub fn load(root: &MdRoot, date: Date) -> Result<Self, MdError> {
        const COLUMNS: &[Column] = &[
            ("trade_date", ColumnType::Date32),
            ("symbol", ColumnType::String),
            ("margin_limit", ColumnType::Int64),
            ("short_limit", ColumnType::Int64),
        ];
        let (file, rows) = read_day(
            root,
            "daily_ext",
            date,
            COLUMNS,
            |path, batch, index, symbol| {
                let limit = |name: &str| {
                    let column = primitive::<Int64Type>(path, batch, name)?;
                    match optional(column, index) {
                        Some(value) if value < 0 => Err(MdError::inconsistent(
                            path,
                            format!("symbol {symbol}: {name} {value} is negative"),
                        )),
                        value => Ok(value),
                    }
                };
                Ok(CreditQuota {
                    symbol,
                    margin_limit: limit("margin_limit")?,
                    short_limit: limit("short_limit")?,
                })
            },
            |quota| quota.symbol,
        )?;
        Ok(Self { date, file, rows })
    }

    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    #[must_use]
    pub const fn file(&self) -> &ReferenceFile {
        &self.file
    }

    #[must_use]
    pub fn get(&self, symbol: Symbol) -> Option<&CreditQuota> {
        self.rows
            .binary_search_by_key(&symbol, |quota| quota.symbol)
            .ok()
            .map(|index| &self.rows[index])
    }

    /// Every row of the date, in symbol order.
    #[must_use]
    pub fn rows(&self) -> &[CreditQuota] {
        &self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::per_share_units;

    #[test]
    fn amounts_per_share_convert_only_when_exact() {
        assert_eq!(per_share_units(2.75), Some(27_500_000_000));
        assert_eq!(per_share_units(0.200_130_263_8), Some(2_001_302_638));
        assert_eq!(per_share_units(144.391_544_97), Some(1_443_915_449_700));
        assert_eq!(per_share_units(0.0), Some(0));
        assert_eq!(per_share_units(0.123_456_789_012), None);
        assert_eq!(per_share_units(-1.0), None);
        assert_eq!(per_share_units(f64::NAN), None);
        assert_eq!(per_share_units(100_000.5), None);
    }
}
