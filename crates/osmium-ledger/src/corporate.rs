//! Corporate actions (`docs/reference/accounting.md` section 13).

use osmium_types::{CorporateAction, CorporateActionKind, Date, Money, PerShare, Price};

use crate::{
    ActionOverride, Entry, Item, LedgerError, MissingTerms, Realized, RealizedKind, SecurityFacts,
    amount::{Rate, Rounding, allocate, divide},
    calendar::add_days,
    ledger::{DayOpen, Delivery, Ledger, Receivable},
};

/// The par value marketdata's dividend figures assume, in units of 1e-10 yuan.
const PAR_UNITS: i128 = 10 * PerShare::UNITS_PER_YUAN as i128;
const TEN_YUAN: i64 = 10 * Price::UNITS_PER_YUAN;

/// The supplementary premium applies to payments from this amount (section 13.2).
const NHI_FLOOR: i64 = 20_000;
const NHI_CAP: i64 = 10_000_000;
const NHI_RATE: Rate = Rate::new(211, 10_000);

/// `old` shares become `new` shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ratio {
    new: i64,
    old: i64,
}

/// The whole-number ratio a split or a reverse split shows in its prices, or `None`
/// when the prices are more than 1 % away from every whole number.
fn infer_ratio(kind: CorporateActionKind, before: Price, reference: Price) -> Option<Ratio> {
    let (before, reference) = (i128::from(before.units()), i128::from(reference.units()));
    if before <= 0 || reference <= 0 {
        return None;
    }
    let splits = before >= reference;
    let allowed = match kind {
        CorporateActionKind::Split => splits,
        CorporateActionKind::ReverseSplit => !splits,
        _ => true,
    };
    let (large, small) = if splits {
        (before, reference)
    } else {
        (reference, before)
    };
    let n = (2 * large + small) / (2 * small);
    if !allowed || n < 2 || (n * small - large).abs() * 100 > large {
        return None;
    }
    let n = i64::try_from(n).ok()?;
    Some(if splits {
        Ratio { new: n, old: 1 }
    } else {
        Ratio { new: 1, old: n }
    })
}

/// The amount for `shares` shares at `per_share`.
fn per_share_amount(shares: i64, per_share: PerShare, rounding: Rounding) -> Money {
    Money::new(divide(
        i128::from(shares) * i128::from(per_share.units()),
        i128::from(PerShare::UNITS_PER_YUAN),
        rounding,
    ))
}

struct Context<'a> {
    action: &'a CorporateAction,
    terms: Option<&'a ActionOverride>,
    cash_date: Date,
    stock_date: Date,
}

impl Context<'_> {
    fn missing(&self, missing: &'static str) -> LedgerError {
        LedgerError::MissingActionTerms {
            symbol: self.action.symbol,
            ex_date: self.action.ex_date,
            action: self.action.kind,
            missing,
        }
    }

    fn receivable(&self, not_before: Date, item: Item, amount: Money, approx: bool) -> Receivable {
        Receivable {
            not_before,
            entry: Entry {
                date: self.action.ex_date,
                settles: not_before,
                symbol: self.action.symbol,
                order: None,
                item,
                amount,
                approx,
            },
        }
    }

    /// Whether the day cash is paid is an approximation.
    fn cash_date_approx(&self) -> bool {
        self.terms.is_none_or(|terms| terms.cash_date.is_none())
    }
}

impl Ledger {
    /// Applies one corporate action to the positions held at the previous close.
    /// Actions on instruments without a position are skipped.
    pub(crate) fn apply_action(
        &mut self,
        action: &CorporateAction,
        facts: Option<&SecurityFacts>,
        open: &mut DayOpen,
    ) -> Result<(), LedgerError> {
        let symbol = action.symbol;
        let awaited = self
            .deliveries
            .iter()
            .any(|delivery| delivery.symbol == symbol);
        let held = self
            .holdings
            .get(&symbol)
            .is_some_and(|holding| !holding.is_empty());
        if action.kind == CorporateActionKind::Rights || !(held || awaited) {
            return Ok(());
        }
        let config = self.config.corporate_actions.clone();
        let terms = config.overrides.iter().find(|terms| {
            terms.symbol == symbol && terms.ex_date == action.ex_date && terms.action == action.kind
        });
        let context = Context {
            action,
            terms,
            cash_date: terms
                .and_then(|terms| terms.cash_date)
                .unwrap_or_else(|| add_days(action.ex_date, config.cash_pay_lag_days)),
            stock_date: terms
                .and_then(|terms| terms.stock_date)
                .unwrap_or_else(|| add_days(action.ex_date, config.stock_credit_lag_days)),
        };
        let given = terms
            .and_then(|terms| terms.share_ratio)
            .map(|(new, old)| Ratio { new, old });
        let approximate = config.missing_terms == MissingTerms::Approximate;
        let (before, reference) = (action.before_price.units(), action.ref_price.units());
        match action.kind {
            CorporateActionKind::Rights => {}
            CorporateActionKind::Dividend => {
                if held {
                    self.dividend(&context, facts, approximate)?;
                }
            }
            CorporateActionKind::Split
            | CorporateActionKind::ParChange
            | CorporateActionKind::ReverseSplit => {
                let ratio = given
                    .or_else(|| infer_ratio(action.kind, action.before_price, action.ref_price))
                    .ok_or_else(|| context.missing("share_ratio"))?;
                self.rescale(&context, ratio, given.is_none(), open)?;
            }
            CorporateActionKind::ReductionLoss => {
                let inferred =
                    (approximate && 0 < before && before <= reference).then_some(Ratio {
                        new: before,
                        old: reference,
                    });
                let ratio = given
                    .or(inferred)
                    .ok_or_else(|| context.missing("share_ratio"))?;
                self.rescale(&context, ratio, given.is_none(), open)?;
            }
            CorporateActionKind::ReductionCash => {
                let refund = terms.and_then(|terms| terms.refund_per_share);
                let inferable = approximate && TEN_YUAN <= before && before < reference;
                let (ratio, refund, approx) = match (given, refund) {
                    (Some(ratio), Some(refund)) => (ratio, Refund::PerShare(refund), false),
                    (None, None) if inferable => (
                        Ratio {
                            new: before - TEN_YUAN,
                            old: reference - TEN_YUAN,
                        },
                        Refund::Inferred {
                            numerator: i128::from(reference - before) * 10,
                            denominator: i128::from(reference - TEN_YUAN),
                        },
                        true,
                    ),
                    (None, _) => return Err(context.missing("share_ratio")),
                    (Some(_), None) => return Err(context.missing("refund_per_share")),
                };
                if held {
                    self.refund(&context, refund, approx, open)?;
                }
                self.rescale(&context, ratio, approx, open)?;
            }
        }
        if held {
            self.valuation.insert(symbol, action.ref_price);
        }
        Ok(())
    }

    fn short_across(&self, action: &CorporateAction) -> Result<(), LedgerError> {
        let short = self
            .holdings
            .get(&action.symbol)
            .is_some_and(|holding| holding.short_shares() + holding.owed_shares() > 0);
        if short {
            return Err(LedgerError::ShortAcrossExDate {
                symbol: action.symbol,
                ex_date: action.ex_date,
            });
        }
        Ok(())
    }

    fn dividend(
        &mut self,
        context: &Context<'_>,
        facts: Option<&SecurityFacts>,
        approximate: bool,
    ) -> Result<(), LedgerError> {
        let action = context.action;
        let stated = |given: Option<PerShare>, known: Option<PerShare>| {
            given.or(known).filter(|amount| amount.units() > 0)
        };
        let mut cash_dividend = stated(
            context.terms.and_then(|terms| terms.cash_dividend),
            action.cash_dividend,
        );
        let stock_dividend = stated(
            context.terms.and_then(|terms| terms.stock_dividend),
            action.stock_dividend,
        );
        // marketdata has the amounts of a dividend only for the issuers its source
        // lists; for the others it has what the ex-date takes off the price.
        let mut unstated = false;
        if cash_dividend.is_none() && stock_dividend.is_none() {
            if action.ref_price == action.before_price {
                // The exchange did not lower the price: nothing is paid on this row.
                return Ok(());
            }
            cash_dividend = action.value.filter(|_| approximate);
            if cash_dividend.is_none() {
                return Err(context.missing("cash_dividend"));
            }
            unstated = true;
        }
        let holding = &self.holdings[&action.symbol];
        let shares = holding.cash_shares() + holding.margin_shares();
        let borrowed = holding.short_shares() + holding.owed_shares();
        let mut payment = Money::ZERO;
        let mut premium_date = context.stock_date;

        if let Some(per_share) = cash_dividend {
            premium_date = context.cash_date;
            let approx = unstated || context.cash_date_approx();
            let received = per_share_amount(shares, per_share, Rounding::Floor);
            if received != Money::ZERO {
                payment += received;
                self.receivables.push(context.receivable(
                    context.cash_date,
                    Item::CashDividend,
                    received,
                    approx,
                ));
            }
            // Whoever has borrowed shares over the ex-date pays the dividend to the
            // lender.
            let paid = per_share_amount(borrowed, per_share, Rounding::Ceil);
            if paid != Money::ZERO {
                self.receivables.push(context.receivable(
                    context.cash_date,
                    Item::CashDividend,
                    -paid,
                    true,
                ));
            }
        }

        if let Some(per_share) = stock_dividend {
            self.short_across(action)?;
            let par = match (
                context.terms.and_then(|terms| terms.par_value),
                facts.and_then(|facts| facts.non_10_par),
            ) {
                (Some(par), _) => i128::from(par.units()),
                (None, Some(true)) => return Err(context.missing("par_value")),
                (None, _) => PAR_UNITS,
            };
            if par <= 0 {
                return Err(context.missing("par_value"));
            }
            let total = i128::from(shares) * i128::from(per_share.units());
            let new_shares = divide(total, par, Rounding::Floor);
            let fraction = total % par;
            // Ten yuan of dividend at par is one share whatever the par value is.
            payment += Money::new(divide(total * 10, par, Rounding::Floor));
            if new_shares > 0 {
                self.deliveries.push(Delivery {
                    not_before: context.stock_date,
                    symbol: action.symbol,
                    shares: new_shares,
                });
            }
            let price = context.terms.and_then(|terms| terms.fraction_price);
            let cash = match price {
                // At par.
                None => divide(
                    fraction,
                    i128::from(PerShare::UNITS_PER_YUAN),
                    Rounding::Floor,
                ),
                Some(price) => divide(
                    fraction * i128::from(price.units()),
                    par * i128::from(Price::UNITS_PER_YUAN),
                    Rounding::Floor,
                ),
            };
            if cash > 0 {
                let approx = context.terms.is_none_or(|terms| terms.stock_date.is_none());
                self.receivables.push(context.receivable(
                    context.stock_date,
                    Item::FractionCash,
                    Money::new(cash),
                    approx,
                ));
            }
        }

        if self.config.account.nhi_enabled && payment.get() >= NHI_FLOOR {
            let premium = NHI_RATE.of(Money::new(payment.get().min(NHI_CAP)), Rounding::HalfUp);
            self.receivables.push(context.receivable(
                premium_date,
                Item::NhiPremium,
                -premium,
                unstated || context.cash_date_approx(),
            ));
        }
        Ok(())
    }

    /// Cash returned by a capital reduction: it lowers the cost of the lots, and what
    /// exceeds a lot's cost is a realised gain.
    fn refund(
        &mut self,
        context: &Context<'_>,
        refund: Refund,
        approx: bool,
        open: &mut DayOpen,
    ) -> Result<(), LedgerError> {
        let action = context.action;
        self.short_across(action)?;
        let holding = self
            .holdings
            .get_mut(&action.symbol)
            .expect("the caller checked the position");
        let shares = holding.cash_shares() + holding.margin_shares();
        let total = match refund {
            Refund::PerShare(per_share) => per_share_amount(shares, per_share, Rounding::Floor),
            Refund::Inferred {
                numerator,
                denominator,
            } => Money::new(divide(
                i128::from(shares) * numerator,
                denominator,
                Rounding::Floor,
            )),
        };
        if total == Money::ZERO {
            return Ok(());
        }
        let lots = holding
            .cash
            .iter_mut()
            .map(|lot| (lot.shares, &mut lot.cost, lot.acquired))
            .chain(
                holding
                    .margin
                    .iter_mut()
                    .map(|lot| (lot.shares, &mut lot.cost, lot.opened)),
            );
        let lots: Vec<(i64, &mut Money, Date)> = lots.collect();
        let weights: Vec<i128> = lots
            .iter()
            .map(|&(shares, ..)| i128::from(shares))
            .collect();
        for ((shares, cost, opened), part) in lots.into_iter().zip(allocate(total, &weights)) {
            let gain = (part - *cost).max(Money::ZERO);
            *cost -= part - gain;
            if gain != Money::ZERO {
                open.realized.push(Realized {
                    date: action.ex_date,
                    symbol: action.symbol,
                    kind: RealizedKind::CorporateAction,
                    shares,
                    opened,
                    proceeds: gain,
                    cost: Money::ZERO,
                    interest: Money::ZERO,
                    pnl: gain,
                    approx,
                });
            }
        }
        self.receivables.push(context.receivable(
            context.cash_date,
            Item::CapitalRefund,
            total,
            approx || context.cash_date_approx(),
        ));
        Ok(())
    }

    /// Changes the number of shares of every lot. The total cost stays; a fraction of
    /// a share is paid in cash.
    fn rescale(
        &mut self,
        context: &Context<'_>,
        ratio: Ratio,
        approx: bool,
        open: &mut DayOpen,
    ) -> Result<(), LedgerError> {
        let action = context.action;
        if ratio.new <= 0 || ratio.old <= 0 {
            return Err(context.missing("share_ratio"));
        }
        self.short_across(action)?;
        let (new, old) = (i128::from(ratio.new), i128::from(ratio.old));
        let scaled = |shares: i64| divide(i128::from(shares) * new, old, Rounding::Floor);
        for delivery in &mut self.deliveries {
            if delivery.symbol == action.symbol {
                delivery.shares = scaled(delivery.shares);
            }
        }
        self.deliveries.retain(|delivery| delivery.shares > 0);
        let Some(holding) = self.holdings.get_mut(&action.symbol) else {
            return Ok(());
        };

        // Lots share the rounding: the running total is rounded, not each lot.
        let (mut before, mut after) = (0_i64, 0_i64);
        let shares = holding
            .cash
            .iter_mut()
            .map(|lot| &mut lot.shares)
            .chain(holding.margin.iter_mut().map(|lot| &mut lot.shares));
        for shares in shares {
            before += *shares;
            let target = scaled(before);
            *shares = target - after;
            after = target;
        }
        let fraction = i128::from(before) * new % old;
        let price = context.terms.and_then(|terms| terms.fraction_price);
        let cash = divide(
            fraction * i128::from(price.unwrap_or(action.ref_price).units()),
            old * i128::from(Price::UNITS_PER_YUAN),
            Rounding::Floor,
        );

        // A lot left without a share passes its cost on; with no lot left the cost is
        // lost.
        let mut lost = Vec::new();
        let orphaned: Money = holding
            .cash
            .iter()
            .filter(|lot| lot.shares == 0)
            .map(|lot| lot.cost)
            .sum();
        let first_opened = holding.cash.first().map(|lot| lot.acquired);
        holding.cash.retain(|lot| lot.shares > 0);
        match (holding.cash.first_mut(), first_opened) {
            (Some(lot), _) => lot.cost += orphaned,
            (None, Some(opened)) => lost.push((opened, orphaned)),
            (None, None) => {}
        }
        let mut repaid = Money::ZERO;
        for lot in holding.margin.iter().filter(|lot| lot.shares == 0) {
            lost.push((lot.opened, lot.cost));
            repaid += lot.loan;
        }
        holding.margin.retain(|lot| lot.shares > 0);
        if holding.is_empty() {
            self.holdings.remove(&action.symbol);
        }
        for (opened, cost) in lost {
            open.realized.push(Realized {
                date: action.ex_date,
                symbol: action.symbol,
                kind: RealizedKind::CorporateAction,
                shares: 0,
                opened,
                proceeds: Money::ZERO,
                cost,
                interest: Money::ZERO,
                pnl: -cost,
                approx: true,
            });
        }
        if repaid != Money::ZERO {
            self.receivables.push(context.receivable(
                action.ex_date,
                Item::MarginLoan,
                -repaid,
                true,
            ));
        }
        if cash > 0 {
            self.receivables.push(context.receivable(
                context.cash_date,
                Item::FractionCash,
                Money::new(cash),
                approx || price.is_none() || context.cash_date_approx(),
            ));
        }
        Ok(())
    }
}

/// How much a capital reduction returns.
#[derive(Debug, Clone, Copy)]
enum Refund {
    PerShare(PerShare),
    /// Yuan per old share as a fraction, worked out from the prices.
    Inferred {
        numerator: i128,
        denominator: i128,
    },
}

#[cfg(test)]
mod tests {
    use osmium_types::{CorporateActionKind, Price};

    use super::{Ratio, infer_ratio};

    fn price(text: &str) -> Price {
        text.parse().unwrap()
    }

    #[test]
    fn a_ratio_is_inferred_only_from_prices_close_to_a_whole_number() {
        let split = CorporateActionKind::Split;
        assert_eq!(
            infer_ratio(split, price("400"), price("100")),
            Some(Ratio { new: 4, old: 1 })
        );
        assert_eq!(
            infer_ratio(split, price("403.5"), price("100")),
            Some(Ratio { new: 4, old: 1 })
        );
        // 2.5 for 1 is not a whole number.
        assert_eq!(infer_ratio(split, price("250"), price("100")), None);
        // A split does not raise the price.
        assert_eq!(infer_ratio(split, price("100"), price("400")), None);
        assert_eq!(
            infer_ratio(CorporateActionKind::ReverseSplit, price("5"), price("50")),
            Some(Ratio { new: 1, old: 10 })
        );
        assert_eq!(
            infer_ratio(CorporateActionKind::ParChange, price("5"), price("50")),
            Some(Ratio { new: 1, old: 10 })
        );
        // The same price is not a change of the number of shares.
        assert_eq!(
            infer_ratio(CorporateActionKind::ParChange, price("50"), price("50")),
            None
        );
    }
}
