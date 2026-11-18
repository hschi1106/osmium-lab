//! Prints one feed day as text, one line per message, for comparison against another reader.
//!
//! ```text
//! cargo run --release -p osmium-md --example dump -- <md-root> <date> <feed> [symbol...]
//! ```
//!
//! Columns are tab-separated: `seq`, `symbol`, `exch_ts` and `observed_time` (µs since the Unix
//! epoch), `phase`, kind, then `key=value` fields. A price is in units of 1e-4 元 and a quantity in
//! shares; a level is `price*qty`. A regular-board symbol without a lot size is read with 1,000
//! shares per lot.

use std::{
    env,
    fmt::Write as _,
    io::{self, BufWriter, Write as _},
    process::ExitCode,
};

use osmium_md::{DayInstruments, DayManifest, FeedDay, MdRoot, MissingLotSize};
use osmium_types::{Book, BookSide, DelayDirection, Feed, Level, MessageBody, Symbol, Trade};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let [root, date, feed, symbols @ ..] = arguments.as_slice() else {
        return Err("usage: dump <md-root> <date> <feed> [symbol...]".into());
    };
    let root = MdRoot::new(root);
    let feed = Feed::from_part(feed).ok_or_else(|| format!("unknown feed {feed}"))?;
    let symbols: Vec<Symbol> = symbols
        .iter()
        .map(|code| Symbol::new(code))
        .collect::<Result<_, _>>()?;

    let manifest = DayManifest::load(&root, date.parse()?)?;
    let instruments = DayInstruments::load(&root, &manifest, feed.venue())?;
    let day = FeedDay::load(
        &root,
        &manifest,
        feed,
        &instruments,
        (!symbols.is_empty()).then_some(symbols.as_slice()),
        MissingLotSize::Assume(1_000),
    )?;

    let mut out = BufWriter::new(io::stdout().lock());
    let mut line = String::new();
    for message in day.messages() {
        line.clear();
        write!(
            line,
            "{}\t{}\t{}\t{}\t{}\t",
            message.seq,
            message.symbol,
            message.exch_ts.micros(),
            message.observed_time.micros(),
            message.phase.as_str(),
        )?;
        match message.body {
            MessageBody::Disclosure { book, trade } => {
                line.push_str("disclosure");
                push_book(&mut line, &book)?;
                if let Some(trade) = trade {
                    push_trade(&mut line, trade)?;
                }
            }
            MessageBody::IntermediateTrade(trade) => {
                line.push_str("trade");
                push_trade(&mut line, trade)?;
            }
            MessageBody::Indicative {
                book,
                matched,
                delay_open,
                delay_close,
            } => {
                line.push_str("indicative");
                push_book(&mut line, &book)?;
                if let Some(matched) = matched {
                    line.push_str("\tmatched=");
                    push_level(&mut line, matched)?;
                }
                for (name, flag) in [("delay_open", delay_open), ("delay_close", delay_close)] {
                    if let Some(flag) = flag {
                        write!(line, "\t{name}={}", u8::from(flag))?;
                    }
                }
            }
            MessageBody::Delay(DelayDirection::Up) => line.push_str("delay\tdirection=up"),
            MessageBody::Delay(DelayDirection::Down) => line.push_str("delay\tdirection=down"),
        }
        writeln!(out, "{line}")?;
    }
    out.flush()?;
    Ok(())
}

fn push_level(line: &mut String, level: Level) -> std::fmt::Result {
    write!(line, "{}*{}", level.price.units(), level.qty.get())
}

fn push_side(line: &mut String, name: &str, side: &BookSide) -> std::fmt::Result {
    write!(line, "\t{name}=")?;
    for (index, level) in side.levels().iter().enumerate() {
        if index > 0 {
            line.push(',');
        }
        push_level(line, *level)?;
    }
    write!(line, "|{}", side.market_qty().get())
}

fn push_book(line: &mut String, book: &Book) -> std::fmt::Result {
    push_side(line, "bid", book.bid())?;
    push_side(line, "ask", book.ask())
}

fn push_trade(line: &mut String, trade: Trade) -> std::fmt::Result {
    write!(line, "\ttrade={}*{}", trade.price.units(), trade.qty.get())?;
    if let Some(cum_qty) = trade.cum_qty {
        write!(line, "\tcum_qty={}", cum_qty.get())?;
    }
    Ok(())
}
