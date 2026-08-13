use std::{collections::BTreeMap, collections::VecDeque, error::Error, fmt};

use market_types::{Decimal, InstrumentId, MarketId, QuantityUnit, TradingDate};
use strategy_api::OrderSide;

use crate::FillRecord;

pub const ACCOUNTING_VERSION: u16 = 6;
pub const LEGACY_ACCOUNTING_VERSION: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AccountingModel {
    EquityV1 = 1,
    FuturesV1 = 2,
    /// Premium-paid/premium-received cash accounting for exchange options.
    OptionsV1 = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentEconomics {
    pub units_per_trading_unit: u64,
    pub multiplier: Decimal,
    pub provenance: Box<str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentLedgerConfig {
    instrument: InstrumentId,
    quantity_unit: QuantityUnit,
    model: AccountingModel,
    economics: InstrumentEconomics,
    fee: ChargeModel,
    tax: ChargeModel,
    day_trade_tax: Option<DayTradeTaxModel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeSides {
    Buy,
    Sell,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingPolicy {
    Down,
    HalfUp,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeBasis {
    NotionalRate,
    FixedPerUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargeModel {
    pub basis: ChargeBasis,
    pub rate: Decimal,
    pub sides: ChargeSides,
    pub minimum: Decimal,
    pub precision: u8,
    pub rounding: RoundingPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayTradeTaxModel {
    ordinary: ChargeModel,
    day_trade: ChargeModel,
    timezone_offset_minutes: i32,
    valid_through: TradingDate,
    eligible: bool,
    eligible_dates: Option<Box<[TradingDate]>>,
    provenance: Box<str>,
}

impl DayTradeTaxModel {
    pub fn new(
        ordinary: ChargeModel,
        day_trade: ChargeModel,
        timezone_offset_minutes: i32,
        valid_through: TradingDate,
        eligible: bool,
        provenance: impl Into<Box<str>>,
    ) -> Result<Self, AccountingError> {
        let provenance = provenance.into();
        if provenance.is_empty()
            || ordinary.basis != ChargeBasis::NotionalRate
            || day_trade.basis != ChargeBasis::NotionalRate
            || ordinary.sides != ChargeSides::Sell
            || day_trade.sides != ChargeSides::Sell
            || day_trade.rate > ordinary.rate
            || ordinary.rate.atoms() < 0
            || day_trade.rate.atoms() < 0
            || ordinary.minimum.atoms() < 0
            || day_trade.minimum.atoms() < 0
            || ordinary.precision > 18
            || day_trade.precision > 18
            || !(-1_439..=1_439).contains(&timezone_offset_minutes)
        {
            return Err(AccountingError::InvalidModel);
        }
        Ok(Self {
            ordinary,
            day_trade,
            timezone_offset_minutes,
            valid_through,
            eligible,
            eligible_dates: None,
            provenance,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_for_dates(
        ordinary: ChargeModel,
        day_trade: ChargeModel,
        timezone_offset_minutes: i32,
        valid_through: TradingDate,
        eligible_dates: impl IntoIterator<Item = TradingDate>,
        provenance: impl Into<Box<str>>,
    ) -> Result<Self, AccountingError> {
        let mut model = Self::new(
            ordinary,
            day_trade,
            timezone_offset_minutes,
            valid_through,
            false,
            provenance,
        )?;
        let dates = eligible_dates
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        model.eligible_dates = Some(dates.into_iter().collect());
        Ok(model)
    }

    #[must_use]
    pub const fn ordinary(&self) -> ChargeModel {
        self.ordinary
    }

    #[must_use]
    pub const fn day_trade(&self) -> ChargeModel {
        self.day_trade
    }

    #[must_use]
    pub const fn timezone_offset_minutes(&self) -> i32 {
        self.timezone_offset_minutes
    }

    #[must_use]
    pub const fn valid_through(&self) -> TradingDate {
        self.valid_through
    }

    #[must_use]
    pub const fn eligible(&self) -> bool {
        self.eligible
    }

    #[must_use]
    pub fn eligible_dates(&self) -> Option<&[TradingDate]> {
        self.eligible_dates.as_deref()
    }

    #[must_use]
    pub fn is_eligible(&self, date: TradingDate) -> bool {
        self.eligible
            || self
                .eligible_dates
                .as_ref()
                .is_some_and(|dates| dates.binary_search(&date).is_ok())
    }

    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

impl InstrumentLedgerConfig {
    #[must_use]
    pub fn new(
        instrument: InstrumentId,
        quantity_unit: QuantityUnit,
        model: AccountingModel,
        economics: InstrumentEconomics,
        fee: ChargeModel,
        tax: ChargeModel,
    ) -> Self {
        Self {
            instrument,
            quantity_unit,
            model,
            economics,
            fee,
            tax,
            day_trade_tax: None,
        }
    }

    #[must_use]
    pub fn with_day_trade_tax(mut self, model: DayTradeTaxModel) -> Self {
        self.tax = model.ordinary;
        self.day_trade_tax = Some(model);
        self
    }

    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn quantity_unit(&self) -> QuantityUnit {
        self.quantity_unit
    }

    #[must_use]
    pub const fn model(&self) -> AccountingModel {
        self.model
    }

    #[must_use]
    pub const fn economics(&self) -> &InstrumentEconomics {
        &self.economics
    }

    #[must_use]
    pub const fn fee(&self) -> ChargeModel {
        self.fee
    }

    #[must_use]
    pub const fn tax(&self) -> ChargeModel {
        self.tax
    }

    #[must_use]
    pub const fn day_trade_tax(&self) -> Option<&DayTradeTaxModel> {
        self.day_trade_tax.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    initial_cash: Decimal,
    cash: Decimal,
    position: i128,
    cost_basis_atoms: i128,
    average_cost: Option<Decimal>,
    realized_pnl: Decimal,
    total_fee: Decimal,
    total_tax: Decimal,
    fills: Vec<FillRecord>,
    economics: InstrumentEconomics,
    fee: ChargeModel,
    tax: ChargeModel,
    accounting_model: AccountingModel,
    day_trade_tax: Option<DayTradeTaxModel>,
    day_trade_buys: VecDeque<DayTradeBuy>,
    day_trade_sells: VecDeque<DayTradeSell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DayTradeBuy {
    date: TradingDate,
    remaining: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DayTradeSell {
    date: TradingDate,
    price: Decimal,
    quantity: u64,
    matched: u64,
    assessed_tax: Decimal,
}

impl Ledger {
    #[must_use]
    pub fn new(
        initial_cash: Decimal,
        economics: InstrumentEconomics,
        fee: ChargeModel,
        tax: ChargeModel,
    ) -> Self {
        Self::new_with_model(initial_cash, economics, fee, tax, AccountingModel::EquityV1)
    }

    #[must_use]
    pub fn new_with_model(
        initial_cash: Decimal,
        economics: InstrumentEconomics,
        fee: ChargeModel,
        tax: ChargeModel,
        accounting_model: AccountingModel,
    ) -> Self {
        Self {
            initial_cash,
            cash: initial_cash,
            position: 0,
            cost_basis_atoms: 0,
            average_cost: None,
            realized_pnl: Decimal::ZERO,
            total_fee: Decimal::ZERO,
            total_tax: Decimal::ZERO,
            fills: Vec::new(),
            economics,
            fee,
            tax,
            accounting_model,
            day_trade_tax: None,
            day_trade_buys: VecDeque::new(),
            day_trade_sells: VecDeque::new(),
        }
    }

    #[must_use]
    pub fn with_day_trade_tax(mut self, model: DayTradeTaxModel) -> Self {
        self.tax = model.ordinary;
        self.day_trade_tax = Some(model);
        self
    }

    pub fn apply_fill(&mut self, fill: FillRecord) -> Result<(), AccountingError> {
        validate_models(&self.economics, self.fee, self.tax)?;
        if let Some(model) = &self.day_trade_tax {
            validate_models(&self.economics, self.fee, model.day_trade)?;
            if self.accounting_model != AccountingModel::EquityV1 {
                return Err(AccountingError::InvalidModel);
            }
        }
        let quantity = i128::from(fill.quantity().value());
        let notional = notional(&fill, &self.economics)?;
        let fee = charge(notional, fill.quantity().value(), fill.side(), self.fee)?;
        let mut day_trade_buys = self.day_trade_buys.clone();
        let mut day_trade_sells = self.day_trade_sells.clone();
        let tax = self.tax_for_fill(&fill, notional, &mut day_trade_buys, &mut day_trade_sells)?;
        let signed_quantity = match fill.side() {
            OrderSide::Buy => quantity,
            OrderSide::Sell => -quantity,
        };
        let price = fill.price().as_decimal();
        let (position, cost_basis_atoms, average_cost, realized_delta) = transition_position(
            self.position,
            self.cost_basis_atoms,
            signed_quantity,
            price,
            &self.economics,
        )?;
        let cash_delta = match self.accounting_model {
            AccountingModel::EquityV1 => match fill.side() {
                OrderSide::Buy => checked_neg(checked_add(checked_add(notional, fee)?, tax)?)?,
                OrderSide::Sell => checked_sub(checked_sub(notional, fee)?, tax)?,
            },
            AccountingModel::FuturesV1 => checked_sub(checked_sub(realized_delta, fee)?, tax)?,
            AccountingModel::OptionsV1 => match fill.side() {
                OrderSide::Buy => checked_neg(checked_add(checked_add(notional, fee)?, tax)?)?,
                OrderSide::Sell => checked_sub(checked_sub(notional, fee)?, tax)?,
            },
        };
        let next_cash = checked_add(self.cash, cash_delta)?;
        let next_fee = checked_add(self.total_fee, fee)?;
        let next_tax = checked_add(self.total_tax, tax)?;
        let next_realized = checked_add(self.realized_pnl, realized_delta)?;

        self.cash = next_cash;
        self.position = position;
        self.cost_basis_atoms = cost_basis_atoms;
        self.average_cost = average_cost;
        self.realized_pnl = next_realized;
        self.total_fee = next_fee;
        self.total_tax = next_tax;
        self.day_trade_buys = day_trade_buys;
        self.day_trade_sells = day_trade_sells;
        self.fills.push(fill);
        Ok(())
    }

    pub fn reconcile(&self) -> Result<(), AccountingError> {
        let mut rebuilt = Self::new_with_model(
            self.initial_cash,
            self.economics.clone(),
            self.fee,
            self.tax,
            self.accounting_model,
        )
        .with_optional_day_trade_tax(self.day_trade_tax.clone());
        for fill in self.fills.clone() {
            rebuilt.apply_fill(fill)?;
        }
        if rebuilt.cash != self.cash
            || rebuilt.position != self.position
            || rebuilt.cost_basis_atoms != self.cost_basis_atoms
            || rebuilt.average_cost != self.average_cost
            || rebuilt.realized_pnl != self.realized_pnl
            || rebuilt.total_fee != self.total_fee
            || rebuilt.total_tax != self.total_tax
            || rebuilt.day_trade_buys != self.day_trade_buys
            || rebuilt.day_trade_sells != self.day_trade_sells
        {
            return Err(AccountingError::Reconciliation);
        }
        Ok(())
    }

    pub fn performance(
        &self,
        final_mark: Option<Decimal>,
    ) -> Result<PerformanceSummary, AccountingError> {
        let unrealized_pnl = match (self.position, final_mark) {
            (0, _) => Some(Decimal::ZERO),
            (_, Some(mark)) => {
                let marked_value = mark
                    .atoms()
                    .checked_mul(self.position.unsigned_abs() as i128)
                    .ok_or(AccountingError::Overflow)?;
                let difference_atoms = if self.position > 0 {
                    marked_value
                        .checked_sub(self.cost_basis_atoms)
                        .ok_or(AccountingError::Overflow)?
                } else {
                    self.cost_basis_atoms
                        .checked_sub(marked_value)
                        .ok_or(AccountingError::Overflow)?
                };
                Some(scale_by_quantity(
                    Decimal::from_atoms(difference_atoms),
                    1,
                    &self.economics,
                )?)
            }
            _ => None,
        };
        Ok(PerformanceSummary {
            initial_cash: self.initial_cash,
            final_cash: self.cash,
            position: self.position,
            realized_pnl: self.realized_pnl,
            unrealized_pnl,
            total_fee: self.total_fee,
            total_tax: self.total_tax,
            fill_count: self.fills.len() as u64,
        })
    }

    #[must_use]
    pub const fn cash(&self) -> Decimal {
        self.cash
    }
    #[must_use]
    pub const fn position(&self) -> i128 {
        self.position
    }
    #[must_use]
    pub const fn realized_pnl(&self) -> Decimal {
        self.realized_pnl
    }
    #[must_use]
    pub const fn average_cost(&self) -> Option<Decimal> {
        self.average_cost
    }
    #[must_use]
    pub const fn economics(&self) -> &InstrumentEconomics {
        &self.economics
    }
    #[must_use]
    pub fn fills(&self) -> &[FillRecord] {
        &self.fills
    }

    #[must_use]
    pub const fn accounting_model(&self) -> AccountingModel {
        self.accounting_model
    }

    #[must_use]
    pub const fn accounting_version(&self) -> u16 {
        ACCOUNTING_VERSION
    }

    fn with_optional_day_trade_tax(mut self, model: Option<DayTradeTaxModel>) -> Self {
        self.day_trade_tax = model;
        self
    }

    fn tax_for_fill(
        &self,
        fill: &FillRecord,
        notional: Decimal,
        buys: &mut VecDeque<DayTradeBuy>,
        sells: &mut VecDeque<DayTradeSell>,
    ) -> Result<Decimal, AccountingError> {
        let Some(model) = &self.day_trade_tax else {
            return charge(notional, fill.quantity().value(), fill.side(), self.tax);
        };
        let date = trading_date(fill.match_time(), model.timezone_offset_minutes)?;
        if !model.is_eligible(date) || date > model.valid_through {
            return charge(
                notional,
                fill.quantity().value(),
                fill.side(),
                model.ordinary,
            );
        }
        match fill.side() {
            OrderSide::Buy => {
                let mut remaining = fill.quantity().value();
                let mut adjustment = Decimal::ZERO;
                for sell in sells
                    .iter_mut()
                    .filter(|sell| sell.date == date && sell.matched < sell.quantity)
                {
                    if remaining == 0 {
                        break;
                    }
                    let matched = remaining.min(sell.quantity - sell.matched);
                    let previous = sell.assessed_tax;
                    sell.matched += matched;
                    sell.assessed_tax = split_sell_tax(
                        sell.price,
                        sell.quantity,
                        sell.matched,
                        &self.economics,
                        model,
                    )?;
                    adjustment =
                        checked_add(adjustment, checked_sub(sell.assessed_tax, previous)?)?;
                    remaining -= matched;
                }
                if remaining > 0 {
                    buys.push_back(DayTradeBuy { date, remaining });
                }
                Ok(adjustment)
            }
            OrderSide::Sell => {
                let mut remaining = fill.quantity().value();
                for buy in buys
                    .iter_mut()
                    .filter(|buy| buy.date == date && buy.remaining > 0)
                {
                    if remaining == 0 {
                        break;
                    }
                    let matched = remaining.min(buy.remaining);
                    buy.remaining -= matched;
                    remaining -= matched;
                }
                let matched = fill.quantity().value() - remaining;
                let assessed_tax = split_sell_tax(
                    fill.price().as_decimal(),
                    fill.quantity().value(),
                    matched,
                    &self.economics,
                    model,
                )?;
                if remaining > 0 {
                    sells.push_back(DayTradeSell {
                        date,
                        price: fill.price().as_decimal(),
                        quantity: fill.quantity().value(),
                        matched,
                        assessed_tax,
                    });
                }
                Ok(assessed_tax)
            }
        }
    }
}

fn trading_date(
    match_time: market_types::MatchTime,
    timezone_offset_minutes: i32,
) -> Result<TradingDate, AccountingError> {
    const MICROS_PER_MINUTE: i64 = 60 * 1_000_000;
    const MICROS_PER_DAY: i64 = 24 * 60 * MICROS_PER_MINUTE;
    let offset = i64::from(timezone_offset_minutes)
        .checked_mul(MICROS_PER_MINUTE)
        .ok_or(AccountingError::InvalidTradingDate)?;
    let local = match_time
        .as_unix_microseconds()
        .checked_add(offset)
        .ok_or(AccountingError::InvalidTradingDate)?;
    let epoch_days = i32::try_from(local.div_euclid(MICROS_PER_DAY))
        .map_err(|_| AccountingError::InvalidTradingDate)?;
    Ok(TradingDate::from_epoch_days(epoch_days))
}

fn split_sell_tax(
    price: Decimal,
    quantity: u64,
    day_trade_quantity: u64,
    economics: &InstrumentEconomics,
    model: &DayTradeTaxModel,
) -> Result<Decimal, AccountingError> {
    let ordinary_quantity = quantity
        .checked_sub(day_trade_quantity)
        .ok_or(AccountingError::Overflow)?;
    let day_trade_tax = if day_trade_quantity == 0 {
        Decimal::ZERO
    } else {
        let notional = scale_by_quantity(price, u128::from(day_trade_quantity), economics)?;
        charge(
            notional,
            day_trade_quantity,
            OrderSide::Sell,
            model.day_trade,
        )?
    };
    let ordinary_tax = if ordinary_quantity == 0 {
        Decimal::ZERO
    } else {
        let notional = scale_by_quantity(price, u128::from(ordinary_quantity), economics)?;
        charge(notional, ordinary_quantity, OrderSide::Sell, model.ordinary)?
    };
    checked_add(day_trade_tax, ordinary_tax)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerformanceSummary {
    pub initial_cash: Decimal,
    pub final_cash: Decimal,
    pub position: i128,
    pub realized_pnl: Decimal,
    pub unrealized_pnl: Option<Decimal>,
    pub total_fee: Decimal,
    pub total_tax: Decimal,
    pub fill_count: u64,
}

fn validate_models(
    economics: &InstrumentEconomics,
    fee: ChargeModel,
    tax: ChargeModel,
) -> Result<(), AccountingError> {
    if economics.units_per_trading_unit == 0
        || economics.multiplier.atoms() <= 0
        || economics.provenance.is_empty()
        || fee.rate.atoms() < 0
        || fee.minimum.atoms() < 0
        || tax.rate.atoms() < 0
        || tax.minimum.atoms() < 0
        || fee.precision > 18
        || tax.precision > 18
    {
        return Err(AccountingError::InvalidModel);
    }
    Ok(())
}

fn notional(
    fill: &FillRecord,
    economics: &InstrumentEconomics,
) -> Result<Decimal, AccountingError> {
    scale_by_quantity(
        fill.price().as_decimal(),
        u128::from(fill.quantity().value()),
        economics,
    )
}

fn scale_by_quantity(
    value: Decimal,
    quantity: u128,
    economics: &InstrumentEconomics,
) -> Result<Decimal, AccountingError> {
    let quantity = i128::try_from(quantity).map_err(|_| AccountingError::Overflow)?;
    let units = i128::from(economics.units_per_trading_unit);
    let scaled = multiply_decimal_atoms(value.atoms(), economics.multiplier.atoms())?;
    let product = scaled
        .checked_mul(quantity)
        .and_then(|value| value.checked_mul(units))
        .ok_or(AccountingError::Overflow)?;
    Ok(Decimal::from_atoms(product))
}

fn multiply_decimal_atoms(left: i128, right: i128) -> Result<i128, AccountingError> {
    let mut left = left;
    let mut right = right;
    let mut denominator = Decimal::SCALE_FACTOR;
    let first = gcd(left.unsigned_abs(), denominator as u128) as i128;
    left /= first;
    denominator /= first;
    let second = gcd(right.unsigned_abs(), denominator as u128) as i128;
    right /= second;
    denominator /= second;
    let product = left.checked_mul(right).ok_or(AccountingError::Overflow)?;
    if product % denominator != 0 {
        return Err(AccountingError::PrecisionLoss);
    }
    Ok(product / denominator)
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn charge(
    notional: Decimal,
    quantity: u64,
    side: OrderSide,
    model: ChargeModel,
) -> Result<Decimal, AccountingError> {
    let applies = matches!(
        (model.sides, side),
        (ChargeSides::Both, _)
            | (ChargeSides::Buy, OrderSide::Buy)
            | (ChargeSides::Sell, OrderSide::Sell)
    );
    if !applies {
        return Ok(Decimal::ZERO);
    }
    let mut left = match model.basis {
        ChargeBasis::NotionalRate => notional.atoms(),
        ChargeBasis::FixedPerUnit => i128::from(quantity)
            .checked_mul(Decimal::SCALE_FACTOR)
            .ok_or(AccountingError::Overflow)?,
    };
    let mut right = model.rate.atoms();
    let mut denominator = Decimal::SCALE_FACTOR;
    let first = gcd(left.unsigned_abs(), denominator as u128) as i128;
    left /= first;
    denominator /= first;
    let second = gcd(right.unsigned_abs(), denominator as u128) as i128;
    right /= second;
    denominator /= second;
    let product = left.checked_mul(right).ok_or(AccountingError::Overflow)?;
    let raw = divide_round(product, denominator, model.rounding)?;
    let quantum = 10_i128
        .checked_pow(u32::from(18 - model.precision))
        .ok_or(AccountingError::Overflow)?;
    let rounded = divide_round(raw, quantum, model.rounding)?
        .checked_mul(quantum)
        .ok_or(AccountingError::Overflow)?;
    Ok(Decimal::from_atoms(rounded.max(model.minimum.atoms())))
}

/// Assesses one configured charge against one actual execution fill.
///
/// This is the same exact-decimal path used by `Ledger`; research strategies that need
/// round-trip attribution can use it without duplicating fee or tax rounding rules.
pub fn assess_fill_charge(
    price: market_types::Price,
    quantity: market_types::Quantity,
    side: OrderSide,
    economics: &InstrumentEconomics,
    model: ChargeModel,
) -> Result<Decimal, AccountingError> {
    validate_models(economics, model, model)?;
    let notional = scale_by_quantity(price.as_decimal(), u128::from(quantity.value()), economics)?;
    charge(notional, quantity.value(), side, model)
}

fn transition_position(
    position: i128,
    cost_basis_atoms: i128,
    delta: i128,
    price: Decimal,
    economics: &InstrumentEconomics,
) -> Result<(i128, i128, Option<Decimal>, Decimal), AccountingError> {
    let next = position
        .checked_add(delta)
        .ok_or(AccountingError::Overflow)?;
    if position == 0 || position.signum() == delta.signum() {
        let added_value = price
            .atoms()
            .checked_mul(delta.unsigned_abs() as i128)
            .ok_or(AccountingError::Overflow)?;
        let next_cost_basis_atoms = cost_basis_atoms
            .checked_add(added_value)
            .ok_or(AccountingError::Overflow)?;
        return Ok((
            next,
            next_cost_basis_atoms,
            average_cost(next_cost_basis_atoms, next)?,
            Decimal::ZERO,
        ));
    }
    let closed = position.unsigned_abs().min(delta.unsigned_abs());
    let position_size =
        i128::try_from(position.unsigned_abs()).map_err(|_| AccountingError::Overflow)?;
    let closed = i128::try_from(closed).map_err(|_| AccountingError::Overflow)?;
    let allocated_basis = if closed == position_size {
        cost_basis_atoms
    } else {
        let numerator = cost_basis_atoms
            .checked_mul(closed)
            .ok_or(AccountingError::Overflow)?;
        divide_round(numerator, position_size, RoundingPolicy::HalfUp)?
    };
    let closed_value = price
        .atoms()
        .checked_mul(closed)
        .ok_or(AccountingError::Overflow)?;
    let difference_atoms = if position > 0 {
        closed_value
            .checked_sub(allocated_basis)
            .ok_or(AccountingError::Overflow)?
    } else {
        allocated_basis
            .checked_sub(closed_value)
            .ok_or(AccountingError::Overflow)?
    };
    let realized = scale_by_quantity(Decimal::from_atoms(difference_atoms), 1, economics)?;
    let next_cost_basis_atoms = if next == 0 {
        0
    } else if next.signum() != position.signum() {
        price
            .atoms()
            .checked_mul(next.unsigned_abs() as i128)
            .ok_or(AccountingError::Overflow)?
    } else {
        cost_basis_atoms
            .checked_sub(allocated_basis)
            .ok_or(AccountingError::Overflow)?
    };
    Ok((
        next,
        next_cost_basis_atoms,
        average_cost(next_cost_basis_atoms, next)?,
        realized,
    ))
}

fn average_cost(
    cost_basis_atoms: i128,
    position: i128,
) -> Result<Option<Decimal>, AccountingError> {
    if position == 0 {
        return Ok(None);
    }
    let divisor = i128::try_from(position.unsigned_abs()).map_err(|_| AccountingError::Overflow)?;
    Ok(Some(Decimal::from_atoms(divide_round(
        cost_basis_atoms,
        divisor,
        RoundingPolicy::HalfUp,
    )?)))
}

fn divide_round(
    numerator: i128,
    denominator: i128,
    policy: RoundingPolicy,
) -> Result<i128, AccountingError> {
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder == 0 {
        return Ok(quotient);
    }
    Ok(match policy {
        RoundingPolicy::Down => quotient,
        RoundingPolicy::Up => quotient.checked_add(1).ok_or(AccountingError::Overflow)?,
        RoundingPolicy::HalfUp if remainder.abs() * 2 >= denominator.abs() => {
            quotient.checked_add(1).ok_or(AccountingError::Overflow)?
        }
        RoundingPolicy::HalfUp => quotient,
    })
}

fn checked_add(left: Decimal, right: Decimal) -> Result<Decimal, AccountingError> {
    left.checked_add(right)
        .map_err(|_| AccountingError::Overflow)
}
fn checked_sub(left: Decimal, right: Decimal) -> Result<Decimal, AccountingError> {
    left.checked_sub(right)
        .map_err(|_| AccountingError::Overflow)
}
fn checked_neg(value: Decimal) -> Result<Decimal, AccountingError> {
    value.checked_neg().map_err(|_| AccountingError::Overflow)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentPerformance {
    instrument: InstrumentId,
    quantity_unit: QuantityUnit,
    accounting_model: AccountingModel,
    economics: InstrumentEconomics,
    average_cost: Option<Decimal>,
    summary: PerformanceSummary,
}

impl InstrumentPerformance {
    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn accounting_model(&self) -> AccountingModel {
        self.accounting_model
    }

    #[must_use]
    pub const fn quantity_unit(&self) -> QuantityUnit {
        self.quantity_unit
    }

    #[must_use]
    pub const fn economics(&self) -> &InstrumentEconomics {
        &self.economics
    }

    #[must_use]
    pub const fn average_cost(&self) -> Option<Decimal> {
        self.average_cost
    }

    #[must_use]
    pub const fn summary(&self) -> PerformanceSummary {
        self.summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiPerformanceSummary {
    initial_cash: Decimal,
    final_cash: Decimal,
    realized_pnl: Decimal,
    unrealized_pnl: Decimal,
    total_fee: Decimal,
    total_tax: Decimal,
    fill_count: u64,
    instruments: Box<[InstrumentPerformance]>,
}

impl MultiPerformanceSummary {
    #[must_use]
    pub const fn initial_cash(&self) -> Decimal {
        self.initial_cash
    }

    #[must_use]
    pub const fn final_cash(&self) -> Decimal {
        self.final_cash
    }

    #[must_use]
    pub const fn realized_pnl(&self) -> Decimal {
        self.realized_pnl
    }

    #[must_use]
    pub const fn unrealized_pnl(&self) -> Decimal {
        self.unrealized_pnl
    }

    #[must_use]
    pub const fn total_fee(&self) -> Decimal {
        self.total_fee
    }

    #[must_use]
    pub const fn total_tax(&self) -> Decimal {
        self.total_tax
    }

    #[must_use]
    pub const fn fill_count(&self) -> u64 {
        self.fill_count
    }

    #[must_use]
    pub const fn instruments(&self) -> &[InstrumentPerformance] {
        &self.instruments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstrumentLedger {
    quantity_unit: QuantityUnit,
    model: AccountingModel,
    ledger: Ledger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiLedger {
    initial_cash: Decimal,
    cash: Decimal,
    ledgers: BTreeMap<InstrumentId, InstrumentLedger>,
}

impl MultiLedger {
    pub fn new(
        initial_cash: Decimal,
        configs: impl IntoIterator<Item = InstrumentLedgerConfig>,
    ) -> Result<Self, AccountingError> {
        if initial_cash.atoms() < 0 {
            return Err(AccountingError::InvalidModel);
        }
        let mut ledgers = BTreeMap::new();
        for config in configs {
            if !model_matches_market(config.instrument(), config.model())
                || config.quantity_unit() == QuantityUnit::SourceUnit
                || (config.day_trade_tax().is_some() && config.model() != AccountingModel::EquityV1)
            {
                return Err(AccountingError::AccountingModelMismatch(
                    config.instrument().clone(),
                ));
            }
            validate_models(config.economics(), config.fee(), config.tax())?;
            if let Some(day_trade) = config.day_trade_tax() {
                validate_models(config.economics(), config.fee(), day_trade.day_trade())?;
            }
            let instrument = config.instrument().clone();
            let entry = InstrumentLedger {
                quantity_unit: config.quantity_unit(),
                model: config.model(),
                ledger: Ledger::new_with_model(
                    Decimal::ZERO,
                    config.economics().clone(),
                    config.fee(),
                    config.tax(),
                    config.model(),
                )
                .with_optional_day_trade_tax(config.day_trade_tax().cloned()),
            };
            if ledgers.insert(instrument.clone(), entry).is_some() {
                return Err(AccountingError::DuplicateInstrument(instrument));
            }
        }
        if ledgers.is_empty() {
            return Err(AccountingError::EmptyUniverse);
        }
        Ok(Self {
            initial_cash,
            cash: initial_cash,
            ledgers,
        })
    }

    pub fn apply_fill(
        &mut self,
        instrument: &InstrumentId,
        fill: FillRecord,
    ) -> Result<(), AccountingError> {
        let entry = self
            .ledgers
            .get_mut(instrument)
            .ok_or_else(|| AccountingError::UnknownInstrument(instrument.clone()))?;
        if fill.quantity().unit() != entry.quantity_unit {
            return Err(AccountingError::QuantityUnitMismatch {
                expected: entry.quantity_unit,
                actual: fill.quantity().unit(),
            });
        }
        let mut next = entry.ledger.clone();
        next.apply_fill(fill)?;
        let cash_delta = checked_sub(next.cash(), entry.ledger.cash())?;
        let next_cash = checked_add(self.cash, cash_delta)?;
        entry.ledger = next;
        self.cash = next_cash;
        Ok(())
    }

    pub fn reconcile(&self) -> Result<(), AccountingError> {
        let mut rebuilt_cash = self.initial_cash;
        for entry in self.ledgers.values() {
            if entry.ledger.accounting_model() != entry.model {
                return Err(AccountingError::Reconciliation);
            }
            entry.ledger.reconcile()?;
            rebuilt_cash = checked_add(rebuilt_cash, entry.ledger.cash())?;
        }
        if rebuilt_cash != self.cash {
            return Err(AccountingError::Reconciliation);
        }
        Ok(())
    }

    pub fn performance(
        &self,
        final_marks: &BTreeMap<InstrumentId, Option<Decimal>>,
    ) -> Result<MultiPerformanceSummary, AccountingError> {
        let mut realized_pnl = Decimal::ZERO;
        let mut unrealized_pnl = Decimal::ZERO;
        let mut total_fee = Decimal::ZERO;
        let mut total_tax = Decimal::ZERO;
        let mut fill_count = 0_u64;
        let mut instruments = Vec::with_capacity(self.ledgers.len());
        for (instrument, entry) in &self.ledgers {
            let mark = final_marks.get(instrument).copied().flatten();
            let summary = entry.ledger.performance(mark)?;
            if entry.ledger.position() != 0 && summary.unrealized_pnl.is_none() {
                return Err(AccountingError::MissingFinalMark(instrument.clone()));
            }
            realized_pnl = checked_add(realized_pnl, summary.realized_pnl)?;
            unrealized_pnl = checked_add(
                unrealized_pnl,
                summary.unrealized_pnl.unwrap_or(Decimal::ZERO),
            )?;
            total_fee = checked_add(total_fee, summary.total_fee)?;
            total_tax = checked_add(total_tax, summary.total_tax)?;
            fill_count = fill_count
                .checked_add(summary.fill_count)
                .ok_or(AccountingError::Overflow)?;
            instruments.push(InstrumentPerformance {
                instrument: instrument.clone(),
                quantity_unit: entry.quantity_unit,
                accounting_model: entry.model,
                economics: entry.ledger.economics().clone(),
                average_cost: entry.ledger.average_cost(),
                summary,
            });
        }
        Ok(MultiPerformanceSummary {
            initial_cash: self.initial_cash,
            final_cash: self.cash,
            realized_pnl,
            unrealized_pnl,
            total_fee,
            total_tax,
            fill_count,
            instruments: instruments.into_boxed_slice(),
        })
    }

    #[must_use]
    pub const fn initial_cash(&self) -> Decimal {
        self.initial_cash
    }

    #[must_use]
    pub const fn cash(&self) -> Decimal {
        self.cash
    }

    pub fn instruments(&self) -> impl Iterator<Item = &InstrumentId> {
        self.ledgers.keys()
    }

    #[must_use]
    pub fn accounting_version(&self) -> u16 {
        self.ledgers
            .values()
            .map(|entry| entry.ledger.accounting_version())
            .max()
            .unwrap_or(ACCOUNTING_VERSION)
    }

    #[must_use]
    pub fn ledger(&self, instrument: &InstrumentId) -> Option<&Ledger> {
        self.ledgers.get(instrument).map(|entry| &entry.ledger)
    }

    /// Returns the amount added to shared cash to mark one open position to market.
    pub fn mark_to_market_adjustment(
        &self,
        instrument: &InstrumentId,
        mark: Decimal,
    ) -> Result<Decimal, AccountingError> {
        let entry = self
            .ledgers
            .get(instrument)
            .ok_or_else(|| AccountingError::UnknownInstrument(instrument.clone()))?;
        let position = entry.ledger.position();
        if position == 0 {
            return Ok(Decimal::ZERO);
        }
        if entry.model == AccountingModel::FuturesV1 {
            return entry
                .ledger
                .performance(Some(mark))?
                .unrealized_pnl
                .ok_or_else(|| AccountingError::MissingFinalMark(instrument.clone()));
        }
        let marked_atoms = mark
            .atoms()
            .checked_mul(position.unsigned_abs() as i128)
            .ok_or(AccountingError::Overflow)?;
        let marked_value = scale_by_quantity(
            Decimal::from_atoms(marked_atoms),
            1,
            entry.ledger.economics(),
        )?;
        if position > 0 {
            Ok(marked_value)
        } else {
            checked_neg(marked_value)
        }
    }
}

fn model_matches_market(instrument: &InstrumentId, model: AccountingModel) -> bool {
    match instrument.market() {
        MarketId::Taifex => {
            matches!(
                model,
                AccountingModel::FuturesV1 | AccountingModel::OptionsV1
            )
        }
        MarketId::Twse | MarketId::Tpex => model == AccountingModel::EquityV1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountingError {
    InvalidModel,
    Overflow,
    PrecisionLoss,
    MissingCostBasis,
    Reconciliation,
    EmptyUniverse,
    DuplicateInstrument(InstrumentId),
    UnknownInstrument(InstrumentId),
    QuantityUnitMismatch {
        expected: QuantityUnit,
        actual: QuantityUnit,
    },
    AccountingModelMismatch(InstrumentId),
    MissingFinalMark(InstrumentId),
    InvalidTradingDate,
}

impl fmt::Display for AccountingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for AccountingError {}

#[cfg(test)]
mod tests {
    use market_types::{InstrumentId, MarketId, MatchTime, Price, Quantity, QuantityUnit, Symbol};
    use strategy_api::OrderId;

    use super::*;

    fn model(sides: ChargeSides) -> ChargeModel {
        ChargeModel {
            basis: ChargeBasis::NotionalRate,
            rate: Decimal::ZERO,
            sides,
            minimum: Decimal::ZERO,
            precision: 0,
            rounding: RoundingPolicy::Down,
        }
    }

    fn ledger() -> Ledger {
        Ledger::new(
            "1000".parse().unwrap(),
            InstrumentEconomics {
                units_per_trading_unit: 1,
                multiplier: "1".parse().unwrap(),
                provenance: "test".into(),
            },
            model(ChargeSides::Both),
            model(ChargeSides::Sell),
        )
    }

    fn fill(side: OrderSide, price: &str, quantity: u64) -> FillRecord {
        fill_in(side, price, quantity, QuantityUnit::TradingUnit)
    }

    fn fill_in(side: OrderSide, price: &str, quantity: u64, unit: QuantityUnit) -> FillRecord {
        fill_at(
            side,
            price,
            quantity,
            unit,
            MatchTime::from_unix_microseconds(1),
        )
    }

    fn fill_at(
        side: OrderSide,
        price: &str,
        quantity: u64,
        unit: QuantityUnit,
        match_time: MatchTime,
    ) -> FillRecord {
        FillRecord::from_market_event(
            OrderId::from_bytes([1; 32]),
            1,
            match_time,
            side,
            Price::parse(price).unwrap(),
            Quantity::new(quantity, unit).unwrap(),
        )
    }

    fn taiwan_day_trade_ledger(eligible: bool, valid_through: &str) -> Ledger {
        let tax = |rate: &str| ChargeModel {
            basis: ChargeBasis::NotionalRate,
            rate: Decimal::parse(rate).unwrap(),
            sides: ChargeSides::Sell,
            minimum: Decimal::ZERO,
            precision: 0,
            rounding: RoundingPolicy::Down,
        };
        Ledger::new(
            Decimal::parse("1000000").unwrap(),
            InstrumentEconomics {
                units_per_trading_unit: 1000,
                multiplier: Decimal::parse("1").unwrap(),
                provenance: "TWSE fixture".into(),
            },
            model(ChargeSides::Both),
            tax("0.003"),
        )
        .with_day_trade_tax(
            DayTradeTaxModel::new(
                tax("0.003"),
                tax("0.0015"),
                480,
                TradingDate::parse(valid_through).unwrap(),
                eligible,
                "MOF:SecuritiesTransactionTaxAct-2025",
            )
            .unwrap(),
        )
    }

    fn taiwan_day_trade_ledger_for_dates(dates: &[&str]) -> Ledger {
        let tax = |rate: &str| ChargeModel {
            basis: ChargeBasis::NotionalRate,
            rate: Decimal::parse(rate).unwrap(),
            sides: ChargeSides::Sell,
            minimum: Decimal::ZERO,
            precision: 0,
            rounding: RoundingPolicy::Down,
        };
        Ledger::new(
            Decimal::parse("1000000").unwrap(),
            InstrumentEconomics {
                units_per_trading_unit: 1000,
                multiplier: Decimal::parse("1").unwrap(),
                provenance: "TWSE fixture".into(),
            },
            model(ChargeSides::Both),
            tax("0.003"),
        )
        .with_day_trade_tax(
            DayTradeTaxModel::new_for_dates(
                tax("0.003"),
                tax("0.0015"),
                480,
                TradingDate::parse("2027-12-31").unwrap(),
                dates.iter().map(|date| TradingDate::parse(date).unwrap()),
                "TWSE:day-trading-eligibility",
            )
            .unwrap(),
        )
    }

    fn taiwan_time(value: &str) -> MatchTime {
        MatchTime::parse(value).unwrap()
    }

    #[test]
    fn fixed_per_unit_fee_is_charged_for_each_filled_contract() {
        let mut ledger = Ledger::new(
            Decimal::parse("1000").unwrap(),
            InstrumentEconomics {
                units_per_trading_unit: 1,
                multiplier: Decimal::parse("2000").unwrap(),
                provenance: "stock futures fixture".into(),
            },
            ChargeModel {
                basis: ChargeBasis::FixedPerUnit,
                rate: Decimal::parse("100").unwrap(),
                sides: ChargeSides::Both,
                minimum: Decimal::ZERO,
                precision: 0,
                rounding: RoundingPolicy::Down,
            },
            model(ChargeSides::Sell),
        );

        ledger
            .apply_fill(fill_in(OrderSide::Sell, "2500", 3, QuantityUnit::Contract))
            .unwrap();

        assert_eq!(
            ledger.performance(None).unwrap().total_fee,
            Decimal::parse("300").unwrap()
        );
    }

    #[test]
    fn buy_then_sell_same_day_uses_reduced_tax_for_matched_quantity() {
        let mut ledger = taiwan_day_trade_ledger(true, "2027-12-31");
        assert_eq!(ledger.accounting_version(), ACCOUNTING_VERSION);
        let time = taiwan_time("2026-06-23T09:00:00+08:00");
        ledger
            .apply_fill(fill_at(
                OrderSide::Buy,
                "100",
                1,
                QuantityUnit::TradingUnit,
                time,
            ))
            .unwrap();
        ledger
            .apply_fill(fill_at(
                OrderSide::Sell,
                "100",
                1,
                QuantityUnit::TradingUnit,
                time,
            ))
            .unwrap();

        assert_eq!(
            ledger.performance(None).unwrap().total_tax,
            Decimal::parse("150").unwrap()
        );
        assert_eq!(ledger.cash(), Decimal::parse("999850").unwrap());
        ledger.reconcile().unwrap();
    }

    #[test]
    fn sell_then_buy_same_day_reprices_prior_sell_tax() {
        let mut ledger = taiwan_day_trade_ledger(true, "2027-12-31");
        let sell_time = taiwan_time("2026-06-23T09:00:00+08:00");
        let buy_time = taiwan_time("2026-06-23T09:01:00+08:00");
        ledger
            .apply_fill(fill_at(
                OrderSide::Sell,
                "100",
                1,
                QuantityUnit::TradingUnit,
                sell_time,
            ))
            .unwrap();
        assert_eq!(
            ledger
                .performance(Some(Decimal::parse("100").unwrap()))
                .unwrap()
                .total_tax,
            Decimal::parse("300").unwrap()
        );
        ledger
            .apply_fill(fill_at(
                OrderSide::Buy,
                "100",
                1,
                QuantityUnit::TradingUnit,
                buy_time,
            ))
            .unwrap();

        assert_eq!(
            ledger.performance(None).unwrap().total_tax,
            Decimal::parse("150").unwrap()
        );
        assert_eq!(ledger.cash(), Decimal::parse("999850").unwrap());
        ledger.reconcile().unwrap();
    }

    #[test]
    fn unmatched_or_ineligible_quantity_keeps_ordinary_tax() {
        let time = taiwan_time("2026-06-23T09:00:00+08:00");
        let mut partial = taiwan_day_trade_ledger(true, "2027-12-31");
        partial
            .apply_fill(fill_at(
                OrderSide::Buy,
                "100",
                1,
                QuantityUnit::TradingUnit,
                time,
            ))
            .unwrap();
        partial
            .apply_fill(fill_at(
                OrderSide::Sell,
                "100",
                2,
                QuantityUnit::TradingUnit,
                time,
            ))
            .unwrap();
        assert_eq!(
            partial
                .performance(Some(Decimal::parse("100").unwrap()))
                .unwrap()
                .total_tax,
            Decimal::parse("450").unwrap()
        );

        for mut ledger in [
            taiwan_day_trade_ledger(false, "2027-12-31"),
            taiwan_day_trade_ledger(true, "2025-12-31"),
        ] {
            ledger
                .apply_fill(fill_at(
                    OrderSide::Buy,
                    "100",
                    1,
                    QuantityUnit::TradingUnit,
                    time,
                ))
                .unwrap();
            ledger
                .apply_fill(fill_at(
                    OrderSide::Sell,
                    "100",
                    1,
                    QuantityUnit::TradingUnit,
                    time,
                ))
                .unwrap();
            assert_eq!(
                ledger.performance(None).unwrap().total_tax,
                Decimal::parse("300").unwrap()
            );
        }
    }

    #[test]
    fn date_scoped_eligibility_does_not_leak_to_adjacent_date() {
        let mut ledger = taiwan_day_trade_ledger_for_dates(&["2026-06-23"]);
        for date in ["2026-06-23", "2026-06-24"] {
            let time = taiwan_time(&format!("{date}T09:00:00+08:00"));
            ledger
                .apply_fill(fill_at(
                    OrderSide::Buy,
                    "100",
                    1,
                    QuantityUnit::TradingUnit,
                    time,
                ))
                .unwrap();
            ledger
                .apply_fill(fill_at(
                    OrderSide::Sell,
                    "100",
                    1,
                    QuantityUnit::TradingUnit,
                    time,
                ))
                .unwrap();
        }
        assert_eq!(
            ledger.performance(None).unwrap().total_tax,
            Decimal::parse("450").unwrap()
        );
        ledger.reconcile().unwrap();
    }

    #[test]
    fn average_cost_realized_pnl_and_reconciliation_are_deterministic() {
        let mut ledger = ledger();
        assert_eq!(ledger.accounting_version(), ACCOUNTING_VERSION);
        ledger.apply_fill(fill(OrderSide::Buy, "100", 2)).unwrap();
        ledger.apply_fill(fill(OrderSide::Buy, "110", 2)).unwrap();
        ledger.apply_fill(fill(OrderSide::Sell, "120", 3)).unwrap();
        assert_eq!(ledger.position(), 1);
        assert_eq!(ledger.cash(), "940".parse().unwrap());
        assert_eq!(ledger.realized_pnl(), "45".parse().unwrap());
        ledger.reconcile().unwrap();
    }

    #[test]
    fn non_divisible_average_cost_preserves_exact_total_basis() {
        let mut ledger = ledger();
        ledger.apply_fill(fill(OrderSide::Buy, "100", 1)).unwrap();
        ledger.apply_fill(fill(OrderSide::Buy, "101", 2)).unwrap();

        assert_eq!(
            ledger.average_cost(),
            Some("100.666666666666666667".parse().unwrap())
        );

        ledger.apply_fill(fill(OrderSide::Sell, "101", 1)).unwrap();
        ledger.apply_fill(fill(OrderSide::Sell, "101", 2)).unwrap();

        assert_eq!(ledger.position(), 0);
        assert_eq!(ledger.cash(), "1001".parse().unwrap());
        assert_eq!(ledger.realized_pnl(), "1".parse().unwrap());
        ledger.reconcile().unwrap();
    }

    #[test]
    fn signed_position_reversal_resets_average_cost() {
        let mut ledger = ledger();
        ledger.apply_fill(fill(OrderSide::Sell, "100", 2)).unwrap();
        ledger.apply_fill(fill(OrderSide::Buy, "90", 3)).unwrap();
        assert_eq!(ledger.position(), 1);
        assert_eq!(ledger.realized_pnl(), "20".parse().unwrap());
        assert_eq!(
            ledger
                .performance(Some("95".parse().unwrap()))
                .unwrap()
                .unrealized_pnl,
            Some("5".parse().unwrap())
        );
    }

    #[test]
    fn realized_pnl_uses_economic_quantity() {
        let mut ledger = Ledger::new(
            "10000000".parse().unwrap(),
            InstrumentEconomics {
                units_per_trading_unit: 1000,
                multiplier: "1".parse().unwrap(),
                provenance: "TWSE trading unit regression".into(),
            },
            model(ChargeSides::Both),
            model(ChargeSides::Sell),
        );
        ledger.apply_fill(fill(OrderSide::Buy, "2335", 1)).unwrap();
        ledger.apply_fill(fill(OrderSide::Sell, "2330", 1)).unwrap();

        assert_eq!(ledger.position(), 0);
        assert_eq!(ledger.cash(), "9995000".parse().unwrap());
        assert_eq!(ledger.realized_pnl(), "-5000".parse().unwrap());
        ledger.reconcile().unwrap();
    }

    #[test]
    fn futures_cash_moves_only_on_realized_pnl_and_costs() {
        let zero = model(ChargeSides::Both);
        let mut ledger = Ledger::new_with_model(
            Decimal::ZERO,
            InstrumentEconomics {
                units_per_trading_unit: 1,
                multiplier: "200".parse().unwrap(),
                provenance: "TAIFEX contract multiplier fixture".into(),
            },
            zero,
            zero,
            AccountingModel::FuturesV1,
        );

        ledger
            .apply_fill(fill_in(OrderSide::Buy, "100", 1, QuantityUnit::Contract))
            .unwrap();
        assert_eq!(ledger.cash(), Decimal::ZERO);
        assert_eq!(ledger.position(), 1);
        assert_eq!(ledger.realized_pnl(), Decimal::ZERO);
        assert_eq!(
            ledger
                .performance(Some("105".parse().unwrap()))
                .unwrap()
                .unrealized_pnl,
            Some("1000".parse().unwrap())
        );

        ledger
            .apply_fill(fill_in(OrderSide::Sell, "105", 1, QuantityUnit::Contract))
            .unwrap();
        assert_eq!(ledger.cash(), "1000".parse().unwrap());
        assert_eq!(ledger.realized_pnl(), "1000".parse().unwrap());
        assert_eq!(ledger.position(), 0);
        assert_eq!(
            ledger.performance(None).unwrap().unrealized_pnl,
            Some(Decimal::ZERO)
        );
        ledger.reconcile().unwrap();
    }

    #[test]
    fn futures_short_close_uses_signed_closed_quantity() {
        let zero = model(ChargeSides::Both);
        let mut ledger = Ledger::new_with_model(
            Decimal::ZERO,
            InstrumentEconomics {
                units_per_trading_unit: 1,
                multiplier: "200".parse().unwrap(),
                provenance: "TAIFEX contract multiplier fixture".into(),
            },
            zero,
            zero,
            AccountingModel::FuturesV1,
        );
        ledger
            .apply_fill(fill_in(OrderSide::Sell, "100", 1, QuantityUnit::Contract))
            .unwrap();
        ledger
            .apply_fill(fill_in(OrderSide::Buy, "95", 1, QuantityUnit::Contract))
            .unwrap();
        assert_eq!(ledger.cash(), "1000".parse().unwrap());
        assert_eq!(ledger.realized_pnl(), "1000".parse().unwrap());
        assert_eq!(ledger.position(), 0);
    }

    #[test]
    fn options_v1_moves_premium_cash_with_contract_multiplier() {
        let zero = model(ChargeSides::Both);
        let mut ledger = Ledger::new_with_model(
            Decimal::ZERO,
            InstrumentEconomics {
                units_per_trading_unit: 1,
                multiplier: "50".parse().unwrap(),
                provenance: "TAIFEX TXO option contract fixture".into(),
            },
            zero,
            zero,
            AccountingModel::OptionsV1,
        );

        ledger
            .apply_fill(fill_in(OrderSide::Buy, "30", 1, QuantityUnit::Contract))
            .unwrap();
        assert_eq!(ledger.cash(), "-1500".parse().unwrap());
        assert_eq!(ledger.realized_pnl(), Decimal::ZERO);

        ledger
            .apply_fill(fill_in(OrderSide::Sell, "35", 1, QuantityUnit::Contract))
            .unwrap();
        assert_eq!(ledger.cash(), "250".parse().unwrap());
        assert_eq!(ledger.realized_pnl(), "250".parse().unwrap());
        assert_eq!(ledger.position(), 0);
        ledger.reconcile().unwrap();
    }

    #[test]
    fn multi_ledger_reconciles_equity_and_futures_cash_separately() {
        let twse = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let taifex = InstrumentId::new(MarketId::Taifex, Symbol::new("TXFH6").unwrap());
        let zero = model(ChargeSides::Both);
        let mut ledger = MultiLedger::new(
            "1000000".parse().unwrap(),
            [
                InstrumentLedgerConfig::new(
                    twse.clone(),
                    QuantityUnit::TradingUnit,
                    AccountingModel::EquityV1,
                    InstrumentEconomics {
                        units_per_trading_unit: 1000,
                        multiplier: "1".parse().unwrap(),
                        provenance: "TWSE trading unit fixture".into(),
                    },
                    zero,
                    zero,
                ),
                InstrumentLedgerConfig::new(
                    taifex.clone(),
                    QuantityUnit::Contract,
                    AccountingModel::FuturesV1,
                    InstrumentEconomics {
                        units_per_trading_unit: 1,
                        multiplier: "200".parse().unwrap(),
                        provenance: "TAIFEX contract multiplier fixture".into(),
                    },
                    zero,
                    zero,
                ),
            ],
        )
        .unwrap();
        ledger
            .apply_fill(
                &twse,
                fill_in(OrderSide::Buy, "100", 1, QuantityUnit::TradingUnit),
            )
            .unwrap();
        ledger
            .apply_fill(
                &taifex,
                fill_in(OrderSide::Buy, "100", 1, QuantityUnit::Contract),
            )
            .unwrap();
        ledger
            .apply_fill(
                &taifex,
                fill_in(OrderSide::Sell, "105", 1, QuantityUnit::Contract),
            )
            .unwrap();
        ledger
            .apply_fill(
                &twse,
                fill_in(OrderSide::Sell, "110", 1, QuantityUnit::TradingUnit),
            )
            .unwrap();

        assert_eq!(ledger.cash(), "1011000".parse().unwrap());
        ledger.reconcile().unwrap();
        let marks = BTreeMap::from([(twse.clone(), None), (taifex.clone(), None)]);
        let performance = ledger.performance(&marks).unwrap();
        assert_eq!(performance.final_cash(), ledger.cash());
        assert_eq!(performance.realized_pnl(), "11000".parse().unwrap());
        assert_eq!(performance.instruments().len(), 2);
        assert_eq!(performance.instruments()[0].instrument(), &twse);
        assert_eq!(
            performance.instruments()[0].accounting_model(),
            AccountingModel::EquityV1
        );
        assert_eq!(performance.instruments()[1].instrument(), &taifex);
        assert_eq!(
            performance.instruments()[1].accounting_model(),
            AccountingModel::FuturesV1
        );
    }

    #[test]
    fn multi_ledger_requires_a_mark_for_open_positions() {
        let instrument = InstrumentId::new(MarketId::Taifex, Symbol::new("TXFH6").unwrap());
        let zero = model(ChargeSides::Both);
        let mut ledger = MultiLedger::new(
            Decimal::ZERO,
            [InstrumentLedgerConfig::new(
                instrument.clone(),
                QuantityUnit::Contract,
                AccountingModel::FuturesV1,
                InstrumentEconomics {
                    units_per_trading_unit: 1,
                    multiplier: "200".parse().unwrap(),
                    provenance: "TAIFEX contract multiplier fixture".into(),
                },
                zero,
                zero,
            )],
        )
        .unwrap();
        ledger
            .apply_fill(
                &instrument,
                fill_in(OrderSide::Buy, "100", 1, QuantityUnit::Contract),
            )
            .unwrap();
        let marks = BTreeMap::from([(instrument.clone(), None)]);
        assert!(matches!(
            ledger.performance(&marks),
            Err(AccountingError::MissingFinalMark(actual)) if actual == instrument
        ));
    }

    #[test]
    fn mark_to_market_adjustments_reconcile_shared_cash_to_current_equity() {
        let stock = InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap());
        let futures = InstrumentId::new(MarketId::Taifex, Symbol::new("CDFH6").unwrap());
        let zero = model(ChargeSides::Both);
        let mut ledger = MultiLedger::new(
            "1000000".parse().unwrap(),
            [
                InstrumentLedgerConfig::new(
                    stock.clone(),
                    QuantityUnit::TradingUnit,
                    AccountingModel::EquityV1,
                    InstrumentEconomics {
                        units_per_trading_unit: 1000,
                        multiplier: "1".parse().unwrap(),
                        provenance: "TWSE fixture".into(),
                    },
                    zero,
                    zero,
                ),
                InstrumentLedgerConfig::new(
                    futures.clone(),
                    QuantityUnit::Contract,
                    AccountingModel::FuturesV1,
                    InstrumentEconomics {
                        units_per_trading_unit: 1,
                        multiplier: "200".parse().unwrap(),
                        provenance: "TAIFEX fixture".into(),
                    },
                    zero,
                    zero,
                ),
            ],
        )
        .unwrap();
        ledger
            .apply_fill(
                &stock,
                fill_in(OrderSide::Buy, "100", 1, QuantityUnit::TradingUnit),
            )
            .unwrap();
        ledger
            .apply_fill(
                &futures,
                fill_in(OrderSide::Sell, "100", 1, QuantityUnit::Contract),
            )
            .unwrap();

        let stock_adjustment = ledger
            .mark_to_market_adjustment(&stock, "110".parse().unwrap())
            .unwrap();
        let futures_adjustment = ledger
            .mark_to_market_adjustment(&futures, "95".parse().unwrap())
            .unwrap();
        let current_equity = ledger
            .cash()
            .checked_add(stock_adjustment)
            .and_then(|value| value.checked_add(futures_adjustment))
            .unwrap();

        assert_eq!(stock_adjustment, "110000".parse().unwrap());
        assert_eq!(futures_adjustment, "1000".parse().unwrap());
        assert_eq!(current_equity, "1011000".parse().unwrap());
    }
}
