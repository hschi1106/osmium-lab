//! The parameters of `docs/reference/accounting.md` section 15.

use osmium_types::{CorporateActionKind, Date, Money, PerShare, Price, Symbol};

use crate::amount::{Rate, Rounding};

/// When buying power is checked (section 7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CashCheck {
    /// An order is refused unless cash and pending settlements cover it.
    AtOrder,
    /// Orders are not checked; the run fails when cash is negative after a settlement.
    AtSettlement,
    /// Cash may go negative.
    None,
}

/// What happens to shares sold without inventory and not bought back (section 8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Uncovered {
    Error,
    ForceCover,
}

/// Prepayment during a disposition period (section 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Prepay {
    All,
    None,
}

/// Where margin and short eligibility comes from (section 9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Eligibility {
    MarketdataApprox,
    None,
}

/// What to do when marketdata lacks the terms of a capital reduction (section 13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissingTerms {
    Error,
    Approximate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountConfig {
    pub initial_cash: Money,
    pub cash_check: CashCheck,
    /// The most that may be bought and sold in one day, filled and working; `None`
    /// is no limit.
    pub daily_limit: Option<Money>,
    pub daytrade_enabled: bool,
    pub credit_offset_enabled: bool,
    pub nhi_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeConfig {
    pub rate: Rate,
    pub discount: Rate,
    pub rounding: Rounding,
    pub min_regular: Money,
    pub min_odd: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayTradeConfig {
    pub uncovered: Uncovered,
    pub borrow_fee_rate: Rate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditConfig {
    pub eligibility: Eligibility,
    pub margin_loan_ratio: Rate,
    /// Overrides the table of section 10.1 when set.
    pub short_margin_ratio: Option<Rate>,
    pub margin_interest_rate: Rate,
    pub short_credit_rate: Rate,
    pub short_fee_rate: Rate,
    pub interest_rounding: Rounding,
    pub call_threshold: Rate,
    pub cancel_threshold: Rate,
    pub notice_lag_days: u32,
    pub grace_days: u32,
    pub term_months: u32,
    pub max_extensions: u32,
}

/// Facts about one corporate action that marketdata does not have (section 13.1).
/// A field that is set replaces the default or the approximation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionOverride {
    pub symbol: Symbol,
    pub ex_date: Date,
    pub action: CorporateActionKind,
    /// The day the cash (dividend, refund or payment for a fraction) is paid.
    pub cash_date: Option<Date>,
    /// The day new shares are delivered.
    pub stock_date: Option<Date>,
    /// `(new, old)`: `old` shares become `new` shares.
    pub share_ratio: Option<(i64, i64)>,
    /// Cash returned per old share by a capital reduction.
    pub refund_per_share: Option<PerShare>,
    /// The par value, for a stock dividend of a share whose par value is not 10 yuan.
    pub par_value: Option<PerShare>,
    /// The price paid for a fraction of a share.
    pub fraction_price: Option<Price>,
    /// The cash dividend per share, replacing what marketdata has.
    pub cash_dividend: Option<PerShare>,
    /// The stock dividend in yuan of par value per share, replacing what marketdata
    /// has.
    pub stock_dividend: Option<PerShare>,
}

impl ActionOverride {
    #[must_use]
    pub const fn new(symbol: Symbol, ex_date: Date, action: CorporateActionKind) -> Self {
        Self {
            symbol,
            ex_date,
            action,
            cash_date: None,
            stock_date: None,
            share_ratio: None,
            refund_per_share: None,
            par_value: None,
            fraction_price: None,
            cash_dividend: None,
            stock_dividend: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorporateActionConfig {
    pub cash_pay_lag_days: u32,
    pub stock_credit_lag_days: u32,
    pub missing_terms: MissingTerms,
    pub overrides: Vec<ActionOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub account: AccountConfig,
    pub fee: FeeConfig,
    pub daytrade: DayTradeConfig,
    pub disposition_prepay: Prepay,
    pub credit: CreditConfig,
    pub corporate_actions: CorporateActionConfig,
}

impl Config {
    /// The defaults of section 15 for an account that starts with `initial_cash`.
    #[must_use]
    pub fn new(initial_cash: Money) -> Self {
        Self {
            account: AccountConfig {
                initial_cash,
                cash_check: CashCheck::AtOrder,
                daily_limit: None,
                daytrade_enabled: true,
                credit_offset_enabled: true,
                nhi_enabled: false,
            },
            fee: FeeConfig {
                rate: Rate::new(1_425, 1_000_000),
                discount: Rate::ONE,
                rounding: Rounding::Floor,
                min_regular: Money::new(20),
                min_odd: Money::new(1),
            },
            daytrade: DayTradeConfig {
                uncovered: Uncovered::Error,
                borrow_fee_rate: Rate::new(7, 100),
            },
            disposition_prepay: Prepay::All,
            credit: CreditConfig {
                eligibility: Eligibility::MarketdataApprox,
                margin_loan_ratio: Rate::new(60, 100),
                short_margin_ratio: None,
                margin_interest_rate: Rate::new(645, 10_000),
                short_credit_rate: Rate::new(2, 1_000),
                short_fee_rate: Rate::new(8, 10_000),
                interest_rounding: Rounding::HalfUp,
                call_threshold: Rate::new(130, 100),
                cancel_threshold: Rate::new(166, 100),
                notice_lag_days: 1,
                grace_days: 2,
                term_months: 6,
                max_extensions: 2,
            },
            corporate_actions: CorporateActionConfig {
                cash_pay_lag_days: 30,
                stock_credit_lag_days: 30,
                missing_terms: MissingTerms::Error,
                overrides: Vec::new(),
            },
        }
    }
}
