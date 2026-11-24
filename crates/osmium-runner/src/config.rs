//! The configuration of a run (`docs/reference/config.md`).

use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr};

use osmium_ledger::{
    ActionOverride, CashCheck, Eligibility, MissingTerms, Prepay, Uncovered,
    amount::{Rate, Rounding},
};
use osmium_matching::AuctionMarginal;
use osmium_md::{DEFAULT_MD_ROOT, MissingLotSize};
use osmium_strategy::{Params, Value};
use osmium_types::{CorporateActionKind, Date, Feed, Money, PerShare, Price, Symbol};
use serde::Deserialize;

/// A configuration that cannot be used: the text is not valid TOML, a key is unknown,
/// or a value is out of range. The message names the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn invalid(key: &str, problem: impl fmt::Display) -> ConfigError {
    ConfigError(format!("{key}: {problem}"))
}

/// What to do on a trading day that lacks one of the feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingDays {
    Error,
    Skip,
}

/// Microseconds between an event and its effect (`docs/reference/matching.md`
/// section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Latency {
    pub market_data: i64,
    pub order: i64,
    pub report: i64,
}

/// A validated run configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConfig {
    pub start: Date,
    pub end: Date,
    pub md_root: PathBuf,
    pub feeds: Vec<Feed>,
    /// The instruments to replay, sorted; `None` replays every instrument of the feeds.
    pub symbols: Option<Vec<Symbol>>,
    pub missing_days: MissingDays,
    pub missing_lot_size: MissingLotSize,
    pub latency: Latency,
    pub auction_marginal: AuctionMarginal,
    pub ledger: osmium_ledger::Config,
    pub strategy: String,
    pub params: Params,
}

/// A decimal number written as text (`"0.001425"`) or as a whole number.
#[derive(Debug)]
enum Decimal {
    Whole(i64),
    Text(String),
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Decimal;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                // A TOML float is binary; the accounting takes exact decimals only.
                formatter.write_str(
                    "a whole number, or a decimal number written as text such as \"0.001425\"",
                )
            }

            fn visit_i64<E>(self, value: i64) -> Result<Decimal, E> {
                Ok(Decimal::Whole(value))
            }

            fn visit_str<E>(self, value: &str) -> Result<Decimal, E> {
                Ok(Decimal::Text(value.to_owned()))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl Decimal {
    fn text(&self) -> String {
        match self {
            Self::Whole(value) => value.to_string(),
            Self::Text(value) => value.clone(),
        }
    }

    fn rate(&self, key: &str) -> Result<Rate, ConfigError> {
        Rate::from_str(&self.text()).map_err(|error| invalid(key, error))
    }

    fn price(&self, key: &str) -> Result<Price, ConfigError> {
        let text = self.text();
        text.parse()
            .map_err(|_| invalid(key, format!("`{text}` is not a price")))
    }

    /// An amount per share with at most ten decimals.
    fn per_share(&self, key: &str) -> Result<PerShare, ConfigError> {
        let text = self.text();
        let error = || invalid(key, format!("`{text}` is not an amount per share"));
        let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
        let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
        if whole.is_empty() || !digits(whole) || !digits(fraction) || fraction.len() > 10 {
            return Err(error());
        }
        let whole: i64 = whole.parse().map_err(|_| error())?;
        let fraction: i64 = format!("{fraction:0<10}").parse().map_err(|_| error())?;
        whole
            .checked_mul(PerShare::UNITS_PER_YUAN)
            .and_then(|units| units.checked_add(fraction))
            .map(PerShare::from_units)
            .ok_or_else(error)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    run: RawRun,
    data: RawData,
    latency: RawLatency,
    #[serde(default)]
    matching: RawMatching,
    account: RawAccount,
    #[serde(default)]
    fee: RawFee,
    #[serde(default)]
    daytrade: RawDayTrade,
    #[serde(default)]
    disposition: RawDisposition,
    #[serde(default)]
    credit: RawCredit,
    #[serde(default)]
    corporate_actions: RawCorporateActions,
    strategy: RawStrategy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRun {
    start: String,
    end: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawData {
    md_root: Option<PathBuf>,
    feeds: Vec<String>,
    symbols: Option<Vec<String>>,
    missing_days: Option<String>,
    assume_lot_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLatency {
    market_data: i64,
    order: i64,
    report: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMatching {
    auction_marginal: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAccount {
    initial_cash: i64,
    cash_check: Option<String>,
    daily_limit: Option<i64>,
    daytrade_enabled: Option<bool>,
    credit_offset_enabled: Option<bool>,
    nhi_enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFee {
    rate: Option<Decimal>,
    discount: Option<Decimal>,
    rounding: Option<String>,
    min_regular: Option<i64>,
    min_odd: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDayTrade {
    uncovered: Option<String>,
    borrow_fee_rate: Option<Decimal>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDisposition {
    prepay: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCredit {
    eligibility: Option<String>,
    margin_loan_ratio: Option<Decimal>,
    short_margin_ratio: Option<Decimal>,
    margin_interest_rate: Option<Decimal>,
    short_credit_rate: Option<Decimal>,
    short_fee_rate: Option<Decimal>,
    interest_rounding: Option<String>,
    call_threshold: Option<Decimal>,
    cancel_threshold: Option<Decimal>,
    notice_lag_days: Option<u32>,
    grace_days: Option<u32>,
    term_months: Option<u32>,
    max_extensions: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCorporateActions {
    cash_pay_lag_days: Option<u32>,
    stock_credit_lag_days: Option<u32>,
    missing_terms: Option<String>,
    #[serde(default)]
    overrides: Vec<RawOverride>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOverride {
    symbol: String,
    ex_date: String,
    action: String,
    cash_date: Option<String>,
    stock_date: Option<String>,
    share_ratio: Option<(i64, i64)>,
    refund_per_share: Option<Decimal>,
    par_value: Option<Decimal>,
    fraction_price: Option<Decimal>,
    cash_dividend: Option<Decimal>,
    stock_dividend: Option<Decimal>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStrategy {
    name: String,
    #[serde(default)]
    params: BTreeMap<String, toml::Value>,
}

fn date(key: &str, text: &str) -> Result<Date, ConfigError> {
    text.parse()
        .map_err(|_| invalid(key, format!("`{text}` is not a date (YYYY-MM-DD)")))
}

fn symbol(key: &str, text: &str) -> Result<Symbol, ConfigError> {
    Symbol::new(text).map_err(|_| invalid(key, format!("`{text}` is not a symbol")))
}

fn rounding(key: &str, text: &str) -> Result<Rounding, ConfigError> {
    one_of(
        key,
        text,
        &[
            ("floor", Rounding::Floor),
            ("half_up", Rounding::HalfUp),
            ("ceil", Rounding::Ceil),
        ],
    )
}

fn one_of<T: Copy>(key: &str, text: &str, choices: &[(&str, T)]) -> Result<T, ConfigError> {
    choices
        .iter()
        .find(|(name, _)| *name == text)
        .map(|&(_, value)| value)
        .ok_or_else(|| {
            let names: Vec<String> = choices
                .iter()
                .map(|(name, _)| format!("\"{name}\""))
                .collect();
            invalid(key, format!("`{text}` is not one of {}", names.join(", ")))
        })
}

fn money(key: &str, value: i64) -> Result<Money, ConfigError> {
    if value < 0 {
        return Err(invalid(key, "must not be negative"));
    }
    Ok(Money::new(value))
}

fn param(key: &str, value: toml::Value) -> Result<Value, ConfigError> {
    Ok(match value {
        toml::Value::Integer(value) => Value::Integer(value),
        toml::Value::String(value) => Value::Text(value),
        toml::Value::Boolean(value) => Value::Boolean(value),
        toml::Value::Array(values) => Value::List(
            values
                .into_iter()
                .map(|value| param(key, value))
                .collect::<Result<_, _>>()?,
        ),
        toml::Value::Float(_) => {
            return Err(invalid(
                key,
                "a number with a fraction must be written as text, for example \"12.5\"",
            ));
        }
        toml::Value::Datetime(_) | toml::Value::Table(_) => {
            return Err(invalid(
                key,
                "must be an integer, text, a boolean or a list",
            ));
        }
    })
}

impl RunConfig {
    /// Parses and validates the text of a configuration file.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig =
            toml::from_str(text).map_err(|error| ConfigError(error.to_string()))?;
        Self::validate(raw)
    }

    #[allow(clippy::too_many_lines)]
    fn validate(raw: RawConfig) -> Result<Self, ConfigError> {
        let start = date("run.start", &raw.run.start)?;
        let end = date("run.end", &raw.run.end)?;
        if end < start {
            return Err(invalid("run.end", "is before run.start"));
        }

        let mut feeds = Vec::new();
        for part in &raw.data.feeds {
            let feed = Feed::from_part(part)
                .ok_or_else(|| invalid("data.feeds", format!("`{part}` is not a feed")))?;
            if feeds.contains(&feed) {
                return Err(invalid("data.feeds", format!("`{part}` is listed twice")));
            }
            feeds.push(feed);
        }
        if feeds.is_empty() {
            return Err(invalid("data.feeds", "must list at least one feed"));
        }
        feeds.sort_unstable_by_key(|feed| feed.rank());
        let symbols = raw
            .data
            .symbols
            .map(|symbols| {
                let mut symbols = symbols
                    .iter()
                    .map(|code| symbol("data.symbols", code))
                    .collect::<Result<Vec<_>, _>>()?;
                symbols.sort_unstable();
                symbols.dedup();
                if symbols.is_empty() {
                    return Err(invalid(
                        "data.symbols",
                        "must not be empty; leave it out to replay every instrument",
                    ));
                }
                Ok(symbols)
            })
            .transpose()?;
        let missing_days = one_of(
            "data.missing_days",
            raw.data.missing_days.as_deref().unwrap_or("error"),
            &[("error", MissingDays::Error), ("skip", MissingDays::Skip)],
        )?;
        let missing_lot_size = match raw.data.assume_lot_size {
            None => MissingLotSize::Reject,
            Some(0) => return Err(invalid("data.assume_lot_size", "must be positive")),
            Some(lot) => MissingLotSize::Assume(lot),
        };

        let latency = Latency {
            market_data: raw.latency.market_data,
            order: raw.latency.order,
            report: raw.latency.report,
        };
        for (key, value) in [
            ("latency.market_data", latency.market_data),
            ("latency.order", latency.order),
            ("latency.report", latency.report),
        ] {
            // A day is the most that makes sense; it also keeps the arithmetic far
            // from overflow.
            if !(0..=86_400_000_000).contains(&value) {
                return Err(invalid(
                    key,
                    "must be between 0 and 86,400,000,000 microseconds",
                ));
            }
        }
        let auction_marginal = one_of(
            "matching.auction_marginal",
            raw.matching
                .auction_marginal
                .as_deref()
                .unwrap_or("conservative"),
            &[
                ("conservative", AuctionMarginal::Conservative),
                ("queue", AuctionMarginal::Queue),
            ],
        )?;

        let mut ledger =
            osmium_ledger::Config::new(money("account.initial_cash", raw.account.initial_cash)?);
        let account = &mut ledger.account;
        if let Some(text) = &raw.account.cash_check {
            account.cash_check = one_of(
                "account.cash_check",
                text,
                &[
                    ("at_order", CashCheck::AtOrder),
                    ("at_settlement", CashCheck::AtSettlement),
                    ("none", CashCheck::None),
                ],
            )?;
        }
        account.daily_limit = match raw.account.daily_limit {
            None | Some(0) => None,
            Some(limit) => Some(money("account.daily_limit", limit)?),
        };
        account.daytrade_enabled = raw
            .account
            .daytrade_enabled
            .unwrap_or(account.daytrade_enabled);
        account.credit_offset_enabled = raw
            .account
            .credit_offset_enabled
            .unwrap_or(account.credit_offset_enabled);
        account.nhi_enabled = raw.account.nhi_enabled.unwrap_or(account.nhi_enabled);

        let set_rate = |target: &mut Rate, value: &Option<Decimal>, key: &str| {
            if let Some(value) = value {
                *target = value.rate(key)?;
            }
            Ok::<(), ConfigError>(())
        };
        set_rate(&mut ledger.fee.rate, &raw.fee.rate, "fee.rate")?;
        set_rate(&mut ledger.fee.discount, &raw.fee.discount, "fee.discount")?;
        if let Some(text) = &raw.fee.rounding {
            ledger.fee.rounding = rounding("fee.rounding", text)?;
        }
        if let Some(value) = raw.fee.min_regular {
            ledger.fee.min_regular = money("fee.min_regular", value)?;
        }
        if let Some(value) = raw.fee.min_odd {
            ledger.fee.min_odd = money("fee.min_odd", value)?;
        }

        if let Some(text) = &raw.daytrade.uncovered {
            ledger.daytrade.uncovered = one_of(
                "daytrade.uncovered",
                text,
                &[
                    ("error", Uncovered::Error),
                    ("force_cover", Uncovered::ForceCover),
                ],
            )?;
        }
        set_rate(
            &mut ledger.daytrade.borrow_fee_rate,
            &raw.daytrade.borrow_fee_rate,
            "daytrade.borrow_fee_rate",
        )?;
        if let Some(text) = &raw.disposition.prepay {
            ledger.disposition_prepay = one_of(
                "disposition.prepay",
                text,
                &[("all", Prepay::All), ("none", Prepay::None)],
            )?;
        }

        let credit = &mut ledger.credit;
        if let Some(text) = &raw.credit.eligibility {
            credit.eligibility = one_of(
                "credit.eligibility",
                text,
                &[
                    ("marketdata_approx", Eligibility::MarketdataApprox),
                    ("none", Eligibility::None),
                ],
            )?;
        }
        set_rate(
            &mut credit.margin_loan_ratio,
            &raw.credit.margin_loan_ratio,
            "credit.margin_loan_ratio",
        )?;
        if let Some(value) = &raw.credit.short_margin_ratio {
            credit.short_margin_ratio = Some(value.rate("credit.short_margin_ratio")?);
        }
        set_rate(
            &mut credit.margin_interest_rate,
            &raw.credit.margin_interest_rate,
            "credit.margin_interest_rate",
        )?;
        set_rate(
            &mut credit.short_credit_rate,
            &raw.credit.short_credit_rate,
            "credit.short_credit_rate",
        )?;
        set_rate(
            &mut credit.short_fee_rate,
            &raw.credit.short_fee_rate,
            "credit.short_fee_rate",
        )?;
        if let Some(text) = &raw.credit.interest_rounding {
            credit.interest_rounding = rounding("credit.interest_rounding", text)?;
        }
        set_rate(
            &mut credit.call_threshold,
            &raw.credit.call_threshold,
            "credit.call_threshold",
        )?;
        set_rate(
            &mut credit.cancel_threshold,
            &raw.credit.cancel_threshold,
            "credit.cancel_threshold",
        )?;
        if !credit.margin_loan_ratio.is_less_than(Rate::ONE) {
            return Err(invalid("credit.margin_loan_ratio", "must be below 1"));
        }
        if credit.cancel_threshold.is_less_than(credit.call_threshold) {
            return Err(invalid(
                "credit.cancel_threshold",
                "must not be below credit.call_threshold",
            ));
        }
        credit.notice_lag_days = raw.credit.notice_lag_days.unwrap_or(credit.notice_lag_days);
        credit.grace_days = raw.credit.grace_days.unwrap_or(credit.grace_days);
        if credit.grace_days == 0 {
            return Err(invalid("credit.grace_days", "must be positive"));
        }
        credit.term_months = raw.credit.term_months.unwrap_or(credit.term_months);
        if credit.term_months == 0 {
            return Err(invalid("credit.term_months", "must be positive"));
        }
        credit.max_extensions = raw.credit.max_extensions.unwrap_or(credit.max_extensions);

        let actions = &mut ledger.corporate_actions;
        actions.cash_pay_lag_days = raw
            .corporate_actions
            .cash_pay_lag_days
            .unwrap_or(actions.cash_pay_lag_days);
        actions.stock_credit_lag_days = raw
            .corporate_actions
            .stock_credit_lag_days
            .unwrap_or(actions.stock_credit_lag_days);
        if let Some(text) = &raw.corporate_actions.missing_terms {
            actions.missing_terms = one_of(
                "corporate_actions.missing_terms",
                text,
                &[
                    ("error", MissingTerms::Error),
                    ("approximate", MissingTerms::Approximate),
                ],
            )?;
        }
        for (index, terms) in raw.corporate_actions.overrides.iter().enumerate() {
            let key = |field: &str| format!("corporate_actions.overrides[{index}].{field}");
            let kind = CorporateActionKind::from_marketdata(&terms.action).ok_or_else(|| {
                invalid(
                    &key("action"),
                    format!("`{}` is not an action", terms.action),
                )
            })?;
            let mut item = ActionOverride::new(
                symbol(&key("symbol"), &terms.symbol)?,
                date(&key("ex_date"), &terms.ex_date)?,
                kind,
            );
            if actions.overrides.iter().any(|other| {
                (other.symbol, other.ex_date, other.action)
                    == (item.symbol, item.ex_date, item.action)
            }) {
                return Err(invalid(&key("symbol"), "the same action is listed twice"));
            }
            let optional_date = |field: &str, value: &Option<String>| {
                value
                    .as_deref()
                    .map(|text| date(&key(field), text))
                    .transpose()
            };
            item.cash_date = optional_date("cash_date", &terms.cash_date)?;
            item.stock_date = optional_date("stock_date", &terms.stock_date)?;
            for (field, day) in [
                ("cash_date", item.cash_date),
                ("stock_date", item.stock_date),
            ] {
                if day.is_some_and(|day| day < item.ex_date) {
                    return Err(invalid(&key(field), "is before ex_date"));
                }
            }
            if let Some((new, old)) = terms.share_ratio {
                if new <= 0 || old <= 0 {
                    return Err(invalid(
                        &key("share_ratio"),
                        "both numbers must be positive",
                    ));
                }
                item.share_ratio = Some((new, old));
            }
            let per_share = |field: &str, value: &Option<Decimal>| {
                value
                    .as_ref()
                    .map(|value| value.per_share(&key(field)))
                    .transpose()
            };
            item.refund_per_share = per_share("refund_per_share", &terms.refund_per_share)?;
            item.par_value = per_share("par_value", &terms.par_value)?;
            if item.par_value.is_some_and(|par| par.units() == 0) {
                return Err(invalid(&key("par_value"), "must be positive"));
            }
            item.cash_dividend = per_share("cash_dividend", &terms.cash_dividend)?;
            item.stock_dividend = per_share("stock_dividend", &terms.stock_dividend)?;
            item.fraction_price = terms
                .fraction_price
                .as_ref()
                .map(|value| value.price(&key("fraction_price")))
                .transpose()?;
            actions.overrides.push(item);
        }

        if raw.strategy.name.is_empty() {
            return Err(invalid("strategy.name", "must not be empty"));
        }
        let params = raw
            .strategy
            .params
            .into_iter()
            .map(|(name, value)| {
                let value = param(&format!("strategy.params.{name}"), value)?;
                Ok((name, value))
            })
            .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;

        Ok(Self {
            start,
            end,
            md_root: raw
                .data
                .md_root
                .unwrap_or_else(|| PathBuf::from(DEFAULT_MD_ROOT)),
            feeds,
            symbols,
            missing_days,
            missing_lot_size,
            latency,
            auction_marginal,
            ledger,
            strategy: raw.strategy.name,
            params: Params::new(params),
        })
    }
}
