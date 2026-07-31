use std::{error::Error, fmt};

use market_types::Decimal;
use strategy_api::OrderSide;

use crate::FillRecord;

pub const ACCOUNTING_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentEconomics {
    pub units_per_trading_unit: u64,
    pub multiplier: Decimal,
    pub provenance: Box<str>,
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
pub struct ChargeModel {
    pub rate: Decimal,
    pub sides: ChargeSides,
    pub minimum: Decimal,
    pub precision: u8,
    pub rounding: RoundingPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    initial_cash: Decimal,
    cash: Decimal,
    position: i128,
    average_cost: Option<Decimal>,
    realized_pnl: Decimal,
    total_fee: Decimal,
    total_tax: Decimal,
    fills: Vec<FillRecord>,
    economics: InstrumentEconomics,
    fee: ChargeModel,
    tax: ChargeModel,
}

impl Ledger {
    #[must_use]
    pub fn new(
        initial_cash: Decimal,
        economics: InstrumentEconomics,
        fee: ChargeModel,
        tax: ChargeModel,
    ) -> Self {
        Self {
            initial_cash,
            cash: initial_cash,
            position: 0,
            average_cost: None,
            realized_pnl: Decimal::ZERO,
            total_fee: Decimal::ZERO,
            total_tax: Decimal::ZERO,
            fills: Vec::new(),
            economics,
            fee,
            tax,
        }
    }

    pub fn apply_fill(&mut self, fill: FillRecord) -> Result<(), AccountingError> {
        validate_models(&self.economics, self.fee, self.tax)?;
        let quantity = i128::from(fill.quantity().value());
        let notional = notional(&fill, &self.economics)?;
        let fee = charge(notional, fill.side(), self.fee)?;
        let tax = charge(notional, fill.side(), self.tax)?;
        let signed_quantity = match fill.side() {
            OrderSide::Buy => quantity,
            OrderSide::Sell => -quantity,
        };
        let price = fill.price().as_decimal();
        let (position, average_cost, realized_delta) = transition_position(
            self.position,
            self.average_cost,
            signed_quantity,
            price,
            &self.economics,
        )?;
        let cash_delta = match fill.side() {
            OrderSide::Buy => checked_neg(checked_add(checked_add(notional, fee)?, tax)?)?,
            OrderSide::Sell => checked_sub(checked_sub(notional, fee)?, tax)?,
        };
        let next_cash = checked_add(self.cash, cash_delta)?;
        let next_fee = checked_add(self.total_fee, fee)?;
        let next_tax = checked_add(self.total_tax, tax)?;
        let next_realized = checked_add(self.realized_pnl, realized_delta)?;

        self.cash = next_cash;
        self.position = position;
        self.average_cost = average_cost;
        self.realized_pnl = next_realized;
        self.total_fee = next_fee;
        self.total_tax = next_tax;
        self.fills.push(fill);
        Ok(())
    }

    pub fn reconcile(&self) -> Result<(), AccountingError> {
        let mut rebuilt = Self::new(
            self.initial_cash,
            self.economics.clone(),
            self.fee,
            self.tax,
        );
        for fill in self.fills.clone() {
            rebuilt.apply_fill(fill)?;
        }
        if rebuilt.cash != self.cash
            || rebuilt.position != self.position
            || rebuilt.average_cost != self.average_cost
            || rebuilt.realized_pnl != self.realized_pnl
            || rebuilt.total_fee != self.total_fee
            || rebuilt.total_tax != self.total_tax
        {
            return Err(AccountingError::Reconciliation);
        }
        Ok(())
    }

    pub fn performance(
        &self,
        final_mark: Option<Decimal>,
    ) -> Result<PerformanceSummary, AccountingError> {
        let unrealized_pnl = match (self.position, self.average_cost, final_mark) {
            (0, _, _) => Some(Decimal::ZERO),
            (_, Some(cost), Some(mark)) => {
                let difference = if self.position > 0 {
                    checked_sub(mark, cost)?
                } else {
                    checked_sub(cost, mark)?
                };
                Some(scale_by_quantity(
                    difference,
                    self.position.unsigned_abs(),
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
    pub fn fills(&self) -> &[FillRecord] {
        &self.fills
    }
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
        || tax.rate.atoms() < 0
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
    let mut left = notional.atoms();
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

fn transition_position(
    position: i128,
    average: Option<Decimal>,
    delta: i128,
    price: Decimal,
    economics: &InstrumentEconomics,
) -> Result<(i128, Option<Decimal>, Decimal), AccountingError> {
    let next = position
        .checked_add(delta)
        .ok_or(AccountingError::Overflow)?;
    if position == 0 || position.signum() == delta.signum() {
        let previous_value = average
            .unwrap_or(Decimal::ZERO)
            .atoms()
            .checked_mul(position.unsigned_abs() as i128)
            .ok_or(AccountingError::Overflow)?;
        let added_value = price
            .atoms()
            .checked_mul(delta.unsigned_abs() as i128)
            .ok_or(AccountingError::Overflow)?;
        let total = previous_value
            .checked_add(added_value)
            .ok_or(AccountingError::Overflow)?;
        return Ok((
            next,
            Some(Decimal::from_atoms(total / next.unsigned_abs() as i128)),
            Decimal::ZERO,
        ));
    }
    let closed = position.unsigned_abs().min(delta.unsigned_abs());
    let average = average.ok_or(AccountingError::MissingCostBasis)?;
    let difference = if position > 0 {
        checked_sub(price, average)?
    } else {
        checked_sub(average, price)?
    };
    let realized = scale_by_quantity(difference, closed, economics)?;
    let next_average = if next == 0 {
        None
    } else if next.signum() != position.signum() {
        Some(price)
    } else {
        Some(average)
    };
    Ok((next, next_average, realized))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountingError {
    InvalidModel,
    Overflow,
    PrecisionLoss,
    MissingCostBasis,
    Reconciliation,
}

impl fmt::Display for AccountingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for AccountingError {}

#[cfg(test)]
mod tests {
    use market_types::{MatchTime, Price, Quantity, QuantityUnit};
    use strategy_api::OrderId;

    use super::*;

    fn model(sides: ChargeSides) -> ChargeModel {
        ChargeModel {
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
        FillRecord {
            order_id: OrderId::from_bytes([1; 32]),
            triggering_ordinal: 1,
            match_time: MatchTime::from_unix_microseconds(1),
            side,
            price: Price::parse(price).unwrap(),
            quantity: Quantity::new(quantity, QuantityUnit::TradingUnit).unwrap(),
        }
    }

    #[test]
    fn average_cost_realized_pnl_and_reconciliation_are_deterministic() {
        let mut ledger = ledger();
        ledger.apply_fill(fill(OrderSide::Buy, "100", 2)).unwrap();
        ledger.apply_fill(fill(OrderSide::Buy, "110", 2)).unwrap();
        ledger.apply_fill(fill(OrderSide::Sell, "120", 3)).unwrap();
        assert_eq!(ledger.position(), 1);
        assert_eq!(ledger.cash(), "940".parse().unwrap());
        assert_eq!(ledger.realized_pnl(), "45".parse().unwrap());
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
}
