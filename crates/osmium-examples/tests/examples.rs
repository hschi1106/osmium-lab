//! The examples run over the sample market of `osmium-fixture`.

use std::{fs, path::PathBuf};

use osmium_fixture::sample;
use osmium_runner::{RunConfig, RunRequest, RunSummary, run};
use osmium_strategy::Registry;

struct OutputDir(PathBuf);

impl OutputDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("osmium-examples-{name}-{}", std::process::id()));
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

fn run_example(name: &str, params: &str) -> (RunSummary, OutputDir) {
    let mut registry = Registry::new();
    osmium_examples::register(&mut registry);
    let tree = sample::tree(name);
    let text = sample::config(&tree, name, params);
    let mut config = RunConfig::parse(&text).unwrap();
    let entry = registry.get(name).unwrap();
    let mut strategy = (entry.build)(&mut config.params).unwrap();
    config.params.finish().unwrap();
    let output = OutputDir::new(name);
    let summary = run(RunRequest {
        config: &config,
        config_text: &text,
        strategy: strategy.as_mut(),
        strategy_version: entry.version,
        output: &output.0,
        progress: &mut |_| {},
    })
    .unwrap();
    (summary, output)
}

const EVENTS_HEADER: &str =
    "order_id,time,event,price,qty,remaining,basis,trigger_feed,trigger_seq,request,reason\n";

#[test]
fn buy_and_hold_buys_once_at_the_open() {
    let (summary, output) = run_example("buy_and_hold", "symbol = \"2330\"\nlots = 2");
    // Sent before the open at the upper limit, filled at the opening price of 100.
    assert_eq!(
        output.read("order_events.csv"),
        format!(
            "{EVENTS_HEADER}\
             1,2026-10-15T08:30:00.001000+08:00,Accepted,,,,,,,,\n\
             1,2026-10-15T09:00:00.000000+08:00,Filled,100,2000,,AuctionBetter,TWSE_F6,100,,\n"
        )
    );
    // Nothing is ordered on the second day.
    assert_eq!((summary.orders, summary.fills), (1, 1));
    // 200,000 and a fee of 285 are paid; the shares are worth 100.5 each.
    assert_eq!(summary.final_cash.get(), 1_000_000 - 200_285);
    assert_eq!(summary.final_equity.get(), 1_000_000 - 200_285 + 201_000);
    assert!(
        output
            .read("positions.csv")
            .ends_with("2026-10-21,2330,2000,0,0,0,100.5\n")
    );
}

#[test]
fn open_to_close_is_flat_every_evening() {
    let (summary, output) = run_example("open_to_close", "symbol = \"2330\"");
    // Both orders are filled at the price of their auction.
    assert_eq!(
        output.read("order_events.csv"),
        format!(
            "{EVENTS_HEADER}\
             1,2026-10-15T08:30:00.001000+08:00,Accepted,,,,,,,,\n\
             1,2026-10-15T09:00:00.000000+08:00,Filled,100,1000,,AuctionBetter,TWSE_F6,100,,\n\
             2,2026-10-15T13:25:00.001000+08:00,Accepted,,,,,,,,\n\
             2,2026-10-15T13:30:00.000000+08:00,Filled,100.5,1000,,AuctionBetter,TWSE_F6,107,,\n\
             3,2026-10-16T08:30:00.001000+08:00,Accepted,,,,,,,,\n\
             3,2026-10-16T09:00:00.000000+08:00,Filled,100,1000,,AuctionBetter,TWSE_F6,200,,\n\
             4,2026-10-16T13:25:00.001000+08:00,Accepted,,,,,,,,\n\
             4,2026-10-16T13:30:00.000000+08:00,Filled,100.5,1000,,AuctionBetter,TWSE_F6,207,,\n"
        )
    );
    // No position is ever carried overnight.
    assert_eq!(
        output.read("positions.csv"),
        "date,symbol,cash_shares,margin_shares,short_shares,owed_shares,price\n"
    );
    // A day: 500 gained, fees of 142 and 143, and the halved tax of a day trade, 150.
    assert_eq!(summary.final_equity.get(), 1_000_000 + 2 * 65);
    assert_eq!(summary.final_cash, summary.final_equity);
}

#[test]
fn passive_quote_waits_for_the_queue() {
    let (summary, output) = run_example("passive_quote", "symbol = \"2330\"");
    // The bid joins 7 lots at 100. Four trade at 09:00:10, and of the five that trade
    // at 09:00:15 the first three are still ahead: the order is filled from the rest.
    // The offer at 100.5 stands behind 3 lots, which trade at 09:00:20; it is filled
    // when the market trades through it at 101.
    assert_eq!(
        output.read("order_events.csv"),
        format!(
            "{EVENTS_HEADER}\
             1,2026-10-15T09:00:00.001100+08:00,Accepted,,,,,,,,\n\
             1,2026-10-15T09:00:15.000000+08:00,Filled,100,1000,,PassiveQueue,TWSE_F6,104,,\n\
             2,2026-10-15T09:00:15.003000+08:00,Accepted,,,,,,,,\n\
             2,2026-10-15T09:00:25.000000+08:00,Filled,100.5,1000,,PassiveThrough,TWSE_F6,106,,\n\
             3,2026-10-16T09:00:00.001100+08:00,Accepted,,,,,,,,\n\
             3,2026-10-16T09:00:15.000000+08:00,Filled,100,1000,,PassiveQueue,TWSE_F6,204,,\n\
             4,2026-10-16T09:00:15.003000+08:00,Accepted,,,,,,,,\n\
             4,2026-10-16T09:00:25.000000+08:00,Filled,100.5,1000,,PassiveThrough,TWSE_F6,206,,\n"
        )
    );
    assert_eq!(summary.final_equity.get(), 1_000_000 + 2 * 65);
}

#[test]
fn parameters_are_checked() {
    let mut registry = Registry::new();
    osmium_examples::register(&mut registry);
    let names: Vec<&str> = registry.entries().iter().map(|entry| entry.name).collect();
    assert_eq!(names, ["buy_and_hold", "open_to_close", "passive_quote"]);

    let tree = sample::tree("parameters");
    for (name, params, problem) in [
        ("buy_and_hold", "", "strategy parameter `symbol` is missing"),
        (
            "buy_and_hold",
            "symbol = \"2330\"\nlots = 0",
            "strategy parameter `lots` must be positive",
        ),
        (
            "open_to_close",
            "symbol = 2330",
            "strategy parameter `symbol`",
        ),
    ] {
        let text = sample::config(&tree, name, params);
        let mut config = RunConfig::parse(&text).unwrap();
        let error = (registry.get(name).unwrap().build)(&mut config.params)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(problem), "{error}");
    }
}
