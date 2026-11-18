//! Whole runs over a synthetic marketdata tree.

use std::{fs, path::PathBuf};

use osmium_fixture::{BookRow, FeedRows, TradeRow, Tree, book, instrument, quote, trade};
use osmium_runner::{RunConfig, RunError, RunRequest, RunSummary, run};
use osmium_strategy::{Context, Strategy, TimerId};
use osmium_types::{
    Board, Date, Feed, Message, NewOrder, OrderEvent, OrderEventKind, OrderId, OrderPrice, Price,
    Shares, Side, Symbol, TimeInForce, TradeKind,
};

const SYMBOL: &str = "2330";

fn symbol() -> Symbol {
    Symbol::new(SYMBOL).unwrap()
}

fn price(text: &str) -> Price {
    text.parse().unwrap()
}

/// Microseconds of a Taiwan wall-clock time on `day`, plus `micros`.
fn at(day: &str, hour: u32, minute: u32, second: u32, micros: i64) -> i64 {
    let date: Date = day.parse().unwrap();
    date.at_taipei(hour, minute, second).micros() + micros
}

/// The opening uncross at 100 for 4 lots, leaving 100 × 7 against 100.5 × 3.
fn open(day: &str, seq: i64) -> (BookRow, TradeRow) {
    let ts = at(day, 9, 0, 0, 0);
    (
        BookRow {
            phase: "OPEN_AUCTION",
            has_trade: true,
            ..book(SYMBOL, seq, ts)
        },
        TradeRow {
            phase: "OPEN_AUCTION",
            ..trade(SYMBOL, seq, ts, 100.0, 4)
        },
    )
}

const DAYS: [&str; 6] = [
    "2026-10-14",
    "2026-10-15",
    "2026-10-16",
    "2026-10-19",
    "2026-10-20",
    "2026-10-21",
];

/// Two days of tick data (15th and 16th) and the reference tables of a week.
fn tree(name: &str) -> Tree {
    let tree = Tree::new(name);
    tree.calendar(&[
        (DAYS[0], true),
        (DAYS[1], true),
        (DAYS[2], true),
        ("2026-10-17", false),
        (DAYS[3], true),
        (DAYS[4], true),
        (DAYS[5], true),
        ("2026-10-22", true),
        ("2026-10-23", true),
    ]);
    let quotes: Vec<_> = DAYS.iter().map(|day| quote(day, SYMBOL, 100.0)).collect();
    tree.quotes(&quotes);
    tree.quotas("2026", &[]);
    for (day, seq) in [(DAYS[1], 10), (DAYS[2], 20)] {
        let (opening, opening_trade) = open(day, seq);
        tree.day(
            day,
            &[instrument(SYMBOL, Some(1_000))],
            &[],
            &[FeedRows {
                feed: Feed::TwseF6,
                books: vec![
                    opening,
                    book(SYMBOL, seq + 1, at(day, 9, 0, 5, 0)),
                    book(SYMBOL, seq + 2, at(day, 9, 0, 5, 2_000)),
                    book(SYMBOL, seq + 3, at(day, 9, 0, 6, 0)),
                ],
                trades: vec![opening_trade],
            }],
        );
    }
    tree
}

fn config(tree: &Tree, extra: &str) -> (String, RunConfig) {
    let text = format!(
        r#"
[run]
start = "2026-10-15"
end = "2026-10-21"

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

[daytrade]
uncovered = "force_cover"

[credit]
eligibility = "none"

[strategy]
name = "recorder"
{extra}"#,
        tree.path().display(),
        Feed::TwseF6.part(),
    );
    let config = RunConfig::parse(&text).unwrap();
    (text, config)
}

struct OutputDir(PathBuf);

impl OutputDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("osmium-runner-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn read(&self, file: &str) -> String {
        fs::read_to_string(self.0.join(file)).unwrap()
    }
}

impl Drop for OutputDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Sells a lot it does not have on the first continuous disclosure of the first day,
/// never buys it back, and writes down what it is shown.
#[derive(Default)]
struct Recorder {
    sold: bool,
    log: Vec<String>,
}

impl Recorder {
    fn note(&mut self, ctx: &Context<'_>, what: &str) {
        let account = ctx.account();
        let position = account.position(symbol());
        self.log.push(format!(
            "{} {what} cash={} available={} position={}/{}/{}",
            ctx.now(),
            account.cash(),
            account.available(),
            position.cash,
            position.margin,
            position.short,
        ));
    }
}

impl Strategy for Recorder {
    fn on_day_start(&mut self, ctx: &mut Context<'_>) {
        let equity = ctx.account().equity();
        self.note(ctx, &format!("day_start equity={equity}"));
    }

    fn on_forced_order(&mut self, ctx: &mut Context<'_>, id: OrderId, order: &NewOrder) {
        self.note(
            ctx,
            &format!(
                "forced id={} {} {} {}",
                id.0,
                order.side.as_str(),
                order.qty,
                order.board.as_str()
            ),
        );
    }

    fn on_market(&mut self, ctx: &mut Context<'_>, message: &Message) {
        self.note(ctx, &format!("market seq={}", message.seq));
        if !self.sold && message.seq == 11 {
            self.sold = true;
            let id = ctx
                .submit(NewOrder {
                    symbol: symbol(),
                    board: Board::Regular,
                    side: Side::Sell,
                    kind: TradeKind::CashSellFirst,
                    price: OrderPrice::Limit(price("100")),
                    time_in_force: TimeInForce::Rod,
                    qty: Shares::new(1_000),
                })
                .unwrap();
            ctx.set_timer(ctx.date().at_taipei(14, 0, 0)).unwrap();
            self.log.push(format!("sent id={}", id.0));
        }
    }

    fn on_order_event(&mut self, ctx: &mut Context<'_>, event: &OrderEvent) {
        let kind = match event.kind {
            OrderEventKind::Accepted => "accepted".to_owned(),
            OrderEventKind::Filled(fill) => format!("filled {} at {}", fill.qty, fill.price),
            OrderEventKind::Rejected { reason, .. } => format!("rejected {}", reason.as_str()),
            other => format!("{other:?}"),
        };
        self.note(ctx, &format!("report id={} {kind}", event.order.0));
    }

    fn on_timer(&mut self, ctx: &mut Context<'_>, timer: TimerId) {
        // Long after the close: the exchange refuses it.
        let id = ctx.submit(NewOrder {
            symbol: symbol(),
            board: Board::Regular,
            side: Side::Buy,
            kind: TradeKind::Cash,
            price: OrderPrice::Limit(price("100")),
            time_in_force: TimeInForce::Rod,
            qty: Shares::new(1_000),
        });
        self.note(
            ctx,
            &format!("timer {} sent={:?}", timer.0, id.map(|id| id.0)),
        );
    }

    fn on_day_end(&mut self, ctx: &mut Context<'_>) {
        let equity = ctx.account().equity();
        self.note(ctx, &format!("day_end equity={equity}"));
        assert!(
            ctx.submit(NewOrder {
                symbol: symbol(),
                board: Board::Regular,
                side: Side::Buy,
                kind: TradeKind::Cash,
                price: OrderPrice::Limit(price("100")),
                time_in_force: TimeInForce::Rod,
                qty: Shares::new(1_000),
            })
            .is_none()
        );
        assert!(ctx.set_timer(ctx.now()).is_err());
    }
}

fn run_recorder(
    tree: &Tree,
    output: &OutputDir,
    extra: &str,
) -> (Result<RunSummary, RunError>, Recorder, Vec<Date>) {
    let (text, config) = config(tree, extra);
    let mut strategy = Recorder::default();
    let mut days = Vec::new();
    let result = run(RunRequest {
        config: &config,
        config_text: &text,
        strategy: &mut strategy,
        strategy_version: "test",
        output: &output.0,
        progress: &mut |day| days.push(day.date),
    });
    (result, strategy, days)
}

const FILES: [&str; 9] = [
    "orders.csv",
    "order_events.csv",
    "entries.csv",
    "realized.csv",
    "daily.csv",
    "positions.csv",
    "diagnostics.csv",
    "config.toml",
    "run.json",
];

#[test]
fn a_run_replays_trades_and_settles() {
    let tree = tree("whole");
    let output = OutputDir::new("whole");
    let (result, strategy, days) = run_recorder(&tree, &output, "");
    let summary = result.unwrap();
    assert_eq!(days.len(), 5);
    assert_eq!(
        (
            summary.trading_days,
            summary.messages,
            summary.orders,
            summary.fills
        ),
        (5, 8, 3, 2)
    );
    assert_eq!(summary.final_cash.get(), 992_416);
    assert_eq!(summary.final_equity.get(), 992_416);

    // The strategy learns of a fill only when its report arrives: the message of
    // 09:00:05.002 reaches it after the fill at 09:00:05.0011 and before the report.
    // The order sent at 14:00 is refused, the uncovered sale is bought back by a
    // forced order in the next opening auction, and nothing can be sent after the
    // close.
    let first_two_days: Vec<&str> = strategy
        .log
        .iter()
        .map(String::as_str)
        .take_while(|line| !line.starts_with("2026-10-19"))
        .collect();
    assert_eq!(
        first_two_days,
        [
            "2026-10-15T08:30:00.000000+08:00 day_start equity=1000000 cash=1000000 available=1000000 position=0/0/0",
            "2026-10-15T09:00:00.000100+08:00 market seq=10 cash=1000000 available=1000000 position=0/0/0",
            "2026-10-15T09:00:05.000100+08:00 market seq=11 cash=1000000 available=1000000 position=0/0/0",
            "sent id=1",
            "2026-10-15T09:00:05.002100+08:00 market seq=12 cash=1000000 available=1000000 position=0/0/0",
            "2026-10-15T09:00:05.003100+08:00 report id=1 accepted cash=1000000 available=1000000 position=0/0/0",
            "2026-10-15T09:00:05.003100+08:00 report id=1 filled 1000 at 100 cash=1000000 available=1099558 position=-1000/0/0",
            "2026-10-15T09:00:06.000100+08:00 market seq=13 cash=1000000 available=1099558 position=-1000/0/0",
            "2026-10-15T14:00:00.000000+08:00 timer 1 sent=Some(2) cash=1000000 available=1099558 position=-1000/0/0",
            "2026-10-15T14:00:00.003000+08:00 report id=2 rejected MarketClosed cash=1000000 available=1099558 position=-1000/0/0",
            "2026-10-15T14:00:00.003000+08:00 day_end equity=992558 cash=1000000 available=1092558 position=-1000/0/0",
            "2026-10-16T08:30:00.000000+08:00 forced id=3 buy 1000 REG cash=1000000 available=1092558 position=-1000/0/0",
            "2026-10-16T08:30:00.000000+08:00 day_start equity=992558 cash=1000000 available=1092558 position=-1000/0/0",
            "2026-10-16T08:30:00.002000+08:00 report id=3 accepted cash=1000000 available=1092558 position=-1000/0/0",
            "2026-10-16T09:00:00.000100+08:00 market seq=20 cash=1000000 available=1092558 position=-1000/0/0",
            "2026-10-16T09:00:00.002000+08:00 report id=3 filled 1000 at 100 cash=1000000 available=992416 position=0/0/0",
            "2026-10-16T09:00:05.000100+08:00 market seq=21 cash=1000000 available=992416 position=0/0/0",
            "2026-10-16T09:00:05.002100+08:00 market seq=22 cash=1000000 available=992416 position=0/0/0",
            "2026-10-16T09:00:06.000100+08:00 market seq=23 cash=1000000 available=992416 position=0/0/0",
            "2026-10-16T09:00:06.000100+08:00 day_end equity=992416 cash=1000000 available=992416 position=0/0/0",
        ]
    );

    assert_eq!(
        output.read("orders.csv"),
        "order_id,date,sent_time,arrival_time,symbol,board,side,kind,price,time_in_force,qty,forced\n\
         1,2026-10-15,2026-10-15T09:00:05.000100+08:00,2026-10-15T09:00:05.001100+08:00,2330,REG,sell,CashSellFirst,100,ROD,1000,\n\
         2,2026-10-15,2026-10-15T14:00:00.000000+08:00,2026-10-15T14:00:00.001000+08:00,2330,REG,buy,Cash,100,ROD,1000,\n\
         3,2026-10-16,2026-10-16T08:30:00.000000+08:00,2026-10-16T08:30:00.000000+08:00,2330,REG,buy,Cash,110,ROD,1000,UncoveredSellFirst\n"
    );
    assert_eq!(
        output.read("order_events.csv"),
        "order_id,time,event,price,qty,remaining,basis,trigger_feed,trigger_seq,request,reason\n\
         1,2026-10-15T09:00:05.001100+08:00,Accepted,,,,,,,,\n\
         1,2026-10-15T09:00:05.001100+08:00,Filled,100,1000,,Aggressive,,,,\n\
         2,2026-10-15T14:00:00.001000+08:00,Rejected,,,,,,,New,MarketClosed\n\
         3,2026-10-16T08:30:00.000000+08:00,Accepted,,,,,,,,\n\
         3,2026-10-16T09:00:00.000000+08:00,Filled,100,1000,,AuctionBetter,TWSE_F6,20,,\n"
    );
    // Sold for 100,000 less a fee of 142 and a tax of 300; the shares borrowed for the
    // delivery cost 7 % of their value at the close; bought back for 100,000 and a
    // fee of 142.
    assert_eq!(
        output.read("entries.csv"),
        "date,settles,symbol,order_id,item,amount,approx\n\
         2026-10-15,2026-10-19,2330,1,TradeValue,100000,false\n\
         2026-10-15,2026-10-19,2330,1,Fee,-142,false\n\
         2026-10-15,2026-10-19,2330,1,Tax,-300,false\n\
         2026-10-15,2026-10-19,2330,,BorrowFee,-7000,true\n\
         2026-10-16,2026-10-20,2330,3,TradeValue,-100000,false\n\
         2026-10-16,2026-10-20,2330,3,Fee,-142,false\n"
    );
    assert_eq!(
        output.read("realized.csv"),
        "date,symbol,kind,shares,opened,proceeds,cost,interest,pnl,approx\n\
         2026-10-16,2330,SellFirstCover,1000,2026-10-15,99558,100142,0,-584,true\n"
    );
    // Each day's cash moves when its settlement is paid, two trading days later.
    assert_eq!(
        output.read("daily.csv"),
        "date,settles,cash,pending,receivable,equity,maintenance,margin_call\n\
         2026-10-15,2026-10-19,1000000,92558,0,992558,,false\n\
         2026-10-16,2026-10-20,1000000,-7584,0,992416,,false\n\
         2026-10-19,2026-10-21,1092558,-100142,0,992416,,false\n\
         2026-10-20,2026-10-22,992416,0,0,992416,,false\n\
         2026-10-21,2026-10-23,992416,0,0,992416,,false\n"
    );
    assert_eq!(
        output.read("positions.csv"),
        "date,symbol,cash_shares,margin_shares,short_shares,owed_shares,price\n\
         2026-10-15,2330,0,0,0,1000,100\n"
    );
    // The days without tick data are skipped because the configuration says so.
    assert_eq!(
        output.read("diagnostics.csv"),
        "date,kind,symbol,detail\n\
         2026-10-19,feed_skipped,,TWSE_F6\n\
         2026-10-20,feed_skipped,,TWSE_F6\n\
         2026-10-21,feed_skipped,,TWSE_F6\n"
    );

    // `run.json` names every other file with its digest.
    let record = output.read("run.json");
    for file in &summary.files {
        let bytes = fs::read(output.0.join(&file.name)).unwrap();
        assert_eq!(file.blake3, blake3::hash(&bytes).to_hex().to_string());
        assert!(record.contains(&file.blake3), "{}", file.name);
    }
    assert_eq!(summary.files.len() + 1, FILES.len());
    assert!(record.contains("\"output_version\": 1"));
    assert!(record.contains("\"path\": \"books/2026/2026-10-15/TWSE_F6.parquet\""));
    assert!(record.contains("\"path\": \"daily/2026.parquet\""));
}

#[test]
fn the_same_inputs_give_the_same_bytes() {
    let tree = tree("same");
    let (first, second) = (OutputDir::new("same-1"), OutputDir::new("same-2"));
    run_recorder(&tree, &first, "").0.unwrap();
    run_recorder(&tree, &second, "").0.unwrap();
    for file in FILES {
        assert_eq!(
            fs::read(first.0.join(file)).unwrap(),
            fs::read(second.0.join(file)).unwrap(),
            "{file}"
        );
    }
}

#[test]
fn a_run_does_not_write_over_existing_results() {
    let tree = tree("existing");
    let output = OutputDir::new("existing");
    run_recorder(&tree, &output, "").0.unwrap();
    let before = output.read("run.json");
    assert!(matches!(
        run_recorder(&tree, &output, "").0,
        Err(RunError::OutputExists { .. })
    ));
    assert_eq!(output.read("run.json"), before);
}

#[test]
fn a_day_without_its_feed_stops_the_run_unless_skipped() {
    let tree = tree("missing");
    let output = OutputDir::new("missing");
    let (text, _) = config(&tree, "");
    let text = text.replace("missing_days = \"skip\"\n", "");
    let config = RunConfig::parse(&text).unwrap();
    let mut days = Vec::new();
    let result = run(RunRequest {
        config: &config,
        config_text: &text,
        strategy: &mut Recorder::default(),
        strategy_version: "test",
        output: &output.0,
        progress: &mut |day| days.push(day.date),
    });
    // The 19th has no manifest.
    assert!(matches!(result, Err(RunError::Md(_))), "{result:?}");
    assert_eq!(days.len(), 2);
    // An unfinished run has no `run.json`.
    assert!(!output.0.join("run.json").exists());
}

#[test]
fn a_range_without_a_trading_day_is_an_error() {
    let tree = tree("weekend");
    let output = OutputDir::new("weekend");
    let (text, _) = config(&tree, "");
    let text = text
        .replace("start = \"2026-10-15\"", "start = \"2026-10-17\"")
        .replace("end = \"2026-10-21\"", "end = \"2026-10-18\"");
    let config = RunConfig::parse(&text).unwrap();
    let result = run(RunRequest {
        config: &config,
        config_text: &text,
        strategy: &mut Recorder::default(),
        strategy_version: "test",
        output: &output.0,
        progress: &mut |_| {},
    });
    assert!(
        matches!(result, Err(RunError::NoTradingDay { .. })),
        "{result:?}"
    );
    assert!(!output.0.exists());
}
