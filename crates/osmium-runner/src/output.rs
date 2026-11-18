//! The files a run writes (`docs/reference/artifacts.md`).

use std::{
    fmt,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use osmium_ledger::{DayClose, Entry, ForcedReason, Holding, Realized};
use osmium_types::{
    Date, NewOrder, OrderEvent, OrderEventKind, OrderId, OrderPrice, Price, Symbol, Timestamp,
};
use serde::Serialize;

use crate::RunError;

/// One written file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileSummary {
    pub name: String,
    /// Data rows, without the header.
    pub rows: u64,
    pub blake3: String,
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> RunError + '_ {
    move |source| RunError::Io {
        path: path.to_owned(),
        source,
    }
}

struct Table {
    name: &'static str,
    path: PathBuf,
    writer: BufWriter<File>,
    hasher: blake3::Hasher,
    rows: u64,
    line: String,
}

impl Table {
    fn create(dir: &Path, name: &'static str, header: &str) -> Result<Self, RunError> {
        let path = dir.join(name);
        let file = File::create(&path).map_err(io_error(&path))?;
        let mut table = Self {
            name,
            path,
            writer: BufWriter::new(file),
            hasher: blake3::Hasher::new(),
            rows: 0,
            line: String::new(),
        };
        table.line(format_args!("{header}"))?;
        table.rows = 0;
        Ok(table)
    }

    fn line(&mut self, line: fmt::Arguments<'_>) -> Result<(), RunError> {
        use fmt::Write as _;
        self.line.clear();
        self.line.write_fmt(line).expect("writing to a string");
        self.line.push('\n');
        self.hasher.update(self.line.as_bytes());
        self.writer
            .write_all(self.line.as_bytes())
            .map_err(io_error(&self.path))?;
        self.rows += 1;
        Ok(())
    }

    fn finish(mut self) -> Result<FileSummary, RunError> {
        self.writer.flush().map_err(io_error(&self.path))?;
        Ok(FileSummary {
            name: self.name.to_owned(),
            rows: self.rows,
            blake3: self.hasher.finalize().to_hex().to_string(),
        })
    }
}

/// An optional value as a CSV field: empty when absent.
struct Opt<T>(Option<T>);

impl<T: fmt::Display> fmt::Display for Opt<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(value) => value.fmt(formatter),
            None => Ok(()),
        }
    }
}

/// Free text as a CSV field.
struct Text<'a>(&'a str);

impl fmt::Display for Text<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.contains([',', '"', '\n', '\r']) {
            write!(formatter, "\"{}\"", self.0.replace('"', "\"\""))
        } else {
            formatter.write_str(self.0)
        }
    }
}

pub(crate) struct Output {
    dir: PathBuf,
    orders: Table,
    order_events: Table,
    entries: Table,
    realized: Table,
    daily: Table,
    positions: Table,
    diagnostics: Table,
}

impl Output {
    /// Creates the directory, which must not hold anything yet.
    pub(crate) fn create(dir: &Path) -> Result<Self, RunError> {
        fs::create_dir_all(dir).map_err(io_error(dir))?;
        if fs::read_dir(dir).map_err(io_error(dir))?.next().is_some() {
            return Err(RunError::OutputExists {
                path: dir.to_owned(),
            });
        }
        Ok(Self {
            dir: dir.to_owned(),
            orders: Table::create(
                dir,
                "orders.csv",
                "order_id,date,sent_time,arrival_time,symbol,board,side,kind,price,time_in_force,qty,forced",
            )?,
            order_events: Table::create(
                dir,
                "order_events.csv",
                "order_id,time,event,price,qty,remaining,basis,trigger_feed,trigger_seq,request,reason",
            )?,
            entries: Table::create(
                dir,
                "entries.csv",
                "date,settles,symbol,order_id,item,amount,approx",
            )?,
            realized: Table::create(
                dir,
                "realized.csv",
                "date,symbol,kind,shares,opened,proceeds,cost,interest,pnl,approx",
            )?,
            daily: Table::create(
                dir,
                "daily.csv",
                "date,settles,cash,pending,receivable,equity,maintenance,margin_call",
            )?,
            positions: Table::create(
                dir,
                "positions.csv",
                "date,symbol,cash_shares,margin_shares,short_shares,owed_shares,price",
            )?,
            diagnostics: Table::create(dir, "diagnostics.csv", "date,kind,symbol,detail")?,
        })
    }

    pub(crate) fn order(
        &mut self,
        id: OrderId,
        date: Date,
        sent: Timestamp,
        arrives: Timestamp,
        order: &NewOrder,
        forced: Option<ForcedReason>,
    ) -> Result<(), RunError> {
        let price = match order.price {
            OrderPrice::Limit(price) => price.to_string(),
            OrderPrice::Market => "market".to_owned(),
        };
        self.orders.line(format_args!(
            "{},{date},{sent},{arrives},{},{},{},{},{price},{},{},{}",
            id.0,
            order.symbol,
            order.board.as_str(),
            order.side.as_str(),
            order.kind.as_str(),
            order.time_in_force.as_str(),
            order.qty,
            Opt(forced.map(ForcedReason::as_str)),
        ))
    }

    pub(crate) fn event(&mut self, event: &OrderEvent) -> Result<(), RunError> {
        let (id, time) = (event.order.0, event.time);
        let table = &mut self.order_events;
        match event.kind {
            OrderEventKind::Accepted => table.line(format_args!("{id},{time},Accepted,,,,,,,,")),
            OrderEventKind::Rejected { request, reason } => table.line(format_args!(
                "{id},{time},Rejected,,,,,,,{},{}",
                request.as_str(),
                reason.as_str()
            )),
            OrderEventKind::Filled(fill) => table.line(format_args!(
                "{id},{time},Filled,{},{},,{},{},{},,",
                fill.price,
                fill.qty,
                fill.basis.as_str(),
                Opt(fill.trigger.map(|(feed, _)| feed.part())),
                Opt(fill.trigger.map(|(_, seq)| seq)),
            )),
            OrderEventKind::Cancelled { qty, reason } => table.line(format_args!(
                "{id},{time},Cancelled,,{qty},,,,,,{}",
                reason.as_str()
            )),
            OrderEventKind::Reduced { by, remaining } => {
                table.line(format_args!("{id},{time},Reduced,,{by},{remaining},,,,,"))
            }
            OrderEventKind::Repriced { price } => {
                table.line(format_args!("{id},{time},Repriced,{price},,,,,,,"))
            }
            OrderEventKind::Expired { qty } => {
                table.line(format_args!("{id},{time},Expired,,{qty},,,,,,"))
            }
            OrderEventKind::PriceBandLimited => {
                table.line(format_args!("{id},{time},PriceBandLimited,,,,,,,,"))
            }
        }
    }

    pub(crate) fn entry(&mut self, entry: &Entry) -> Result<(), RunError> {
        self.entries.line(format_args!(
            "{},{},{},{},{},{},{}",
            entry.date,
            entry.settles,
            entry.symbol,
            Opt(entry.order.map(|order| order.0)),
            entry.item.as_str(),
            entry.amount,
            entry.approx,
        ))
    }

    pub(crate) fn realized(&mut self, realized: &Realized) -> Result<(), RunError> {
        self.realized.line(format_args!(
            "{},{},{},{},{},{},{},{},{},{}",
            realized.date,
            realized.symbol,
            realized.kind.as_str(),
            realized.shares,
            realized.opened,
            realized.proceeds,
            realized.cost,
            realized.interest,
            realized.pnl,
            realized.approx,
        ))
    }

    pub(crate) fn day(&mut self, close: &DayClose) -> Result<(), RunError> {
        // The ratio to six decimals, rounded down.
        let maintenance = close.maintenance.map(|ratio| {
            let scaled = ratio.numerator * 1_000_000 / ratio.denominator.max(1);
            format!("{}.{:06}", scaled / 1_000_000, scaled % 1_000_000)
        });
        self.daily.line(format_args!(
            "{},{},{},{},{},{},{},{}",
            close.date,
            close.settles,
            close.cash,
            close.pending,
            close.receivable,
            close.equity,
            Opt(maintenance),
            close.call,
        ))
    }

    pub(crate) fn position(
        &mut self,
        date: Date,
        symbol: Symbol,
        holding: &Holding,
        price: Option<Price>,
    ) -> Result<(), RunError> {
        self.positions.line(format_args!(
            "{date},{symbol},{},{},{},{},{}",
            holding.cash_shares(),
            holding.margin_shares(),
            holding.short_shares(),
            holding.owed_shares(),
            Opt(price),
        ))
    }

    pub(crate) fn diagnostic(
        &mut self,
        date: Date,
        kind: &str,
        symbol: Option<Symbol>,
        detail: &str,
    ) -> Result<(), RunError> {
        self.diagnostics.line(format_args!(
            "{date},{kind},{},{}",
            Opt(symbol),
            Text(detail)
        ))
    }

    /// Writes a file that is not a table and returns its summary.
    pub(crate) fn file(&self, name: &str, contents: &[u8]) -> Result<FileSummary, RunError> {
        let path = self.dir.join(name);
        fs::write(&path, contents).map_err(io_error(&path))?;
        Ok(FileSummary {
            name: name.to_owned(),
            rows: 0,
            blake3: blake3::hash(contents).to_hex().to_string(),
        })
    }

    /// Flushes every table and returns their summaries in a fixed order.
    pub(crate) fn finish(self) -> Result<(PathBuf, Vec<FileSummary>), RunError> {
        let tables = [
            self.orders,
            self.order_events,
            self.entries,
            self.realized,
            self.daily,
            self.positions,
            self.diagnostics,
        ];
        let files = tables
            .into_iter()
            .map(Table::finish)
            .collect::<Result<Vec<_>, _>>()?;
        Ok((self.dir, files))
    }
}
