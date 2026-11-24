//! The configuration file (`docs/reference/config.md`).

use std::path::Path;

use osmium_ledger::{CashCheck, Eligibility, MissingTerms, Uncovered, amount::Rate};
use osmium_matching::AuctionMarginal;
use osmium_md::{DEFAULT_MD_ROOT, MissingLotSize};
use osmium_runner::{MissingDays, RunConfig};
use osmium_types::{CorporateActionKind, Feed, Money, Symbol};

const MINIMAL: &str = r#"
[run]
start = "2024-03-04"
end = "2024-03-08"

[data]
feeds = ["TWSE_F6"]

[latency]
market_data = 0
order = 0
report = 0

[account]
initial_cash = 1000000

[strategy]
name = "buy_and_hold"
"#;

fn problem(text: &str) -> String {
    RunConfig::parse(text).unwrap_err().to_string()
}

/// `MINIMAL` with `old` replaced by `new`.
fn changed(old: &str, new: &str) -> String {
    assert!(MINIMAL.contains(old), "{old}");
    MINIMAL.replace(old, new)
}

#[test]
fn the_required_keys_are_enough() {
    let config = RunConfig::parse(MINIMAL).unwrap();
    assert_eq!(config.start.to_string(), "2024-03-04");
    assert_eq!(config.end.to_string(), "2024-03-08");
    assert_eq!(config.md_root, Path::new(DEFAULT_MD_ROOT));
    assert_eq!(config.feeds, [Feed::TwseF6]);
    assert_eq!(config.symbols, None);
    assert_eq!(config.missing_days, MissingDays::Error);
    assert_eq!(config.missing_lot_size, MissingLotSize::Reject);
    assert_eq!(config.auction_marginal, AuctionMarginal::Conservative);
    assert_eq!(config.strategy, "buy_and_hold");
    // Everything else is the default of the accounting.
    assert_eq!(
        config.ledger,
        osmium_ledger::Config::new(Money::new(1_000_000))
    );
}

#[test]
fn every_section_is_read() {
    let text = r#"
[run]
start = "2024-03-04"
end = "2024-03-04"

[data]
md_root = "/tmp/md"
feeds = ["TPEX_F6", "TWSE_F23", "TWSE_F6"]
symbols = ["2330", "0050", "2330"]
missing_days = "skip"
assume_lot_size = 1000

[latency]
market_data = 150
order = 2000
report = 2500

[matching]
auction_marginal = "queue"

[account]
initial_cash = 5000000
cash_check = "at_settlement"
daily_limit = 3000000
daytrade_enabled = false
credit_offset_enabled = false
nhi_enabled = true

[fee]
rate = "0.001425"
discount = "0.28"
rounding = "half_up"
min_regular = 1
min_odd = 0

[daytrade]
uncovered = "force_cover"
borrow_fee_rate = "0.05"

[disposition]
prepay = "none"

[credit]
eligibility = "none"
margin_loan_ratio = "0.5"
short_margin_ratio = "0.9"
margin_interest_rate = "0.06"
short_credit_rate = "0.001"
short_fee_rate = "0.0008"
interest_rounding = "floor"
call_threshold = "1.3"
cancel_threshold = "1.66"
notice_lag_days = 1
grace_days = 2
term_months = 6
max_extensions = 2

[corporate_actions]
cash_pay_lag_days = 20
stock_credit_lag_days = 30
missing_terms = "approximate"

[[corporate_actions.overrides]]
symbol = "2330"
ex_date = "2024-03-18"
action = "dividend"
cash_date = "2024-04-11"
cash_dividend = "3.4999845"

[[corporate_actions.overrides]]
symbol = "3432"
ex_date = "2024-03-20"
action = "reduction_cash"
share_ratio = [7, 10]
refund_per_share = 3
fraction_price = "10"

[strategy]
name = "quoter"

[strategy.params]
symbol = "2330"
lots = 2
edge = "0.5"
aggressive = false
watch = ["2330", "0050"]
"#;
    let mut config = RunConfig::parse(text).unwrap();
    assert_eq!(config.md_root, Path::new("/tmp/md"));
    // In the order in which feeds are merged, whatever the order in the file.
    let mut by_rank = vec![Feed::TpexF6, Feed::TwseF23, Feed::TwseF6];
    by_rank.sort_unstable_by_key(|feed| feed.rank());
    assert_eq!(config.feeds, by_rank);
    assert_ne!(config.feeds[0], Feed::TpexF6);
    let symbol = |code: &str| Symbol::new(code).unwrap();
    assert_eq!(config.symbols, Some(vec![symbol("0050"), symbol("2330")]));
    assert_eq!(config.missing_days, MissingDays::Skip);
    assert_eq!(config.missing_lot_size, MissingLotSize::Assume(1_000));
    assert_eq!(
        (
            config.latency.market_data,
            config.latency.order,
            config.latency.report
        ),
        (150, 2_000, 2_500)
    );
    assert_eq!(config.auction_marginal, AuctionMarginal::Queue);

    let ledger = &config.ledger;
    assert_eq!(ledger.account.cash_check, CashCheck::AtSettlement);
    assert_eq!(ledger.account.daily_limit, Some(Money::new(3_000_000)));
    assert!(!ledger.account.daytrade_enabled);
    assert!(!ledger.account.credit_offset_enabled);
    assert!(ledger.account.nhi_enabled);
    assert_eq!(ledger.fee.discount, Rate::new(28, 100));
    assert_eq!(ledger.fee.min_odd, Money::new(0));
    assert_eq!(ledger.daytrade.uncovered, Uncovered::ForceCover);
    assert_eq!(ledger.credit.eligibility, Eligibility::None);
    assert_eq!(ledger.credit.short_margin_ratio, Some(Rate::new(9, 10)));
    assert_eq!(ledger.credit.cancel_threshold, Rate::new(166, 100));
    assert_eq!(ledger.credit.term_months, 6);
    assert_eq!(
        ledger.corporate_actions.missing_terms,
        MissingTerms::Approximate
    );
    let [dividend, reduction] = &ledger.corporate_actions.overrides[..] else {
        panic!("two overrides");
    };
    assert_eq!(dividend.action, CorporateActionKind::Dividend);
    assert_eq!(dividend.cash_date.unwrap().to_string(), "2024-04-11");
    assert_eq!(dividend.cash_dividend.unwrap().units(), 34_999_845_000);
    assert_eq!(reduction.share_ratio, Some((7, 10)));
    assert_eq!(reduction.refund_per_share.unwrap().units(), 30_000_000_000);
    assert_eq!(reduction.fraction_price.unwrap().to_string(), "10");

    // A strategy takes its parameters out one by one; what is left is unknown to it.
    let params = &mut config.params;
    assert_eq!(params.symbol("symbol").unwrap(), symbol("2330"));
    assert_eq!(params.integer("lots").unwrap(), 2);
    assert_eq!(params.price("edge").unwrap().to_string(), "0.5");
    assert!(!params.boolean_or("aggressive", true).unwrap());
    assert!(params.finish().is_err());
    assert_eq!(
        params.symbols("watch").unwrap(),
        [symbol("2330"), symbol("0050")]
    );
    params.finish().unwrap();
}

#[test]
fn mistakes_are_named_by_their_key() {
    for (text, expected) in [
        (changed("[strategy]", "[strategy]\nnmae = 1"), "nmae"),
        (
            changed("[latency]", "[latency]\njitter = 3"),
            "unknown field `jitter`",
        ),
        (changed("[account]", "[acount]"), "acount"),
        (
            changed("end = \"2024-03-08\"", "end = \"2024-03-01\""),
            "run.end: is before run.start",
        ),
        (
            changed("start = \"2024-03-04\"", "start = \"2024-3-4\""),
            "run.start: `2024-3-4` is not a date",
        ),
        (
            changed("[\"TWSE_F6\"]", "[\"TWSE_F7\"]"),
            "data.feeds: `TWSE_F7` is not a feed",
        ),
        (
            changed("[\"TWSE_F6\"]", "[\"TWSE_F6\", \"TWSE_F6\"]"),
            "data.feeds: `TWSE_F6` is listed twice",
        ),
        (changed("[\"TWSE_F6\"]", "[]"), "data.feeds: must list"),
        (
            changed("[\"TWSE_F6\"]", "[\"TWSE_F6\"]\nsymbols = []"),
            "data.symbols: must not be empty",
        ),
        (
            changed("[\"TWSE_F6\"]", "[\"TWSE_F6\"]\nmissing_days = \"ignore\""),
            "data.missing_days: `ignore` is not one of \"error\", \"skip\"",
        ),
        (changed("order = 0", "order = -1"), "latency.order: must be"),
        (
            changed("initial_cash = 1000000", "initial_cash = -1"),
            "account.initial_cash: must not be negative",
        ),
        (
            changed("[strategy]", "[fee]\nrate = 0.001425\n\n[strategy]"),
            "expected a whole number, or a decimal number written as text",
        ),
        (
            changed(
                "[strategy]",
                "[credit]\nmargin_loan_ratio = \"1\"\n\n[strategy]",
            ),
            "credit.margin_loan_ratio: must be below 1",
        ),
        (
            changed(
                "[strategy]",
                "[[corporate_actions.overrides]]\nsymbol = \"2330\"\nex_date = \"2024-03-18\"\n\
                 action = \"dividend\"\ncash_date = \"2024-03-15\"\n\n[strategy]",
            ),
            "corporate_actions.overrides[0].cash_date: is before ex_date",
        ),
        (
            changed(
                "name = \"buy_and_hold\"",
                "name = \"buy_and_hold\"\n\n[strategy.params]\nedge = 0.5",
            ),
            "strategy.params.edge: a number with a fraction must be written as text",
        ),
    ] {
        let problem = problem(&text);
        assert!(problem.contains(expected), "{expected}: {problem}");
    }
}
