use market_types::{Decimal, InstrumentId, MarketId, Price, Quantity, QuantityUnit};
use strategy_api::{
    BinaryIdentity, CanonicalParamsChecksum, NewOrderEntry, OrderIntent, OrderSide, OrderType,
    ParameterRange, RangeBound, SessionKind, SessionPhase, Strategy, StrategyDeclaration,
    StrategyDefinition, StrategyEventContext, StrategyExecutionError, StrategyFactory,
    StrategyFactoryError, StrategyIdentity, StrategyOutputSink, StrategyParameterField,
    StrategyParameterSchema, StrategyParameterType, StrategyParameterValue, StrategyRegistryError,
    ValidatedStrategyParameters,
};

pub const EXAMPLE_STRATEGY_ID: &str = "example.price-threshold-buy-once";
pub const EXAMPLE_STRATEGY_VERSION: &str = "1";

#[derive(Debug)]
pub struct PriceThresholdBuyOnce {
    identity: StrategyIdentity,
    params_checksum: CanonicalParamsChecksum,
    declaration: StrategyDeclaration,
    entry_price: Price,
    quantity: u64,
    submitted: bool,
}

impl PriceThresholdBuyOnce {
    fn quantity_unit(instrument: &InstrumentId) -> QuantityUnit {
        match instrument.market() {
            MarketId::Twse | MarketId::Tpex => QuantityUnit::TradingUnit,
            MarketId::Taifex => QuantityUnit::Contract,
        }
    }
}

impl Strategy for PriceThresholdBuyOnce {
    fn identity(&self) -> &StrategyIdentity {
        &self.identity
    }

    fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
        self.params_checksum
    }

    fn declaration(&self) -> StrategyDeclaration {
        self.declaration.clone()
    }

    fn on_event(
        &mut self,
        context: StrategyEventContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        if self.submitted
            || context.session().phase() != SessionPhase::Active
            || !matches!(context.trading().new_order_entry(), NewOrderEntry::Allowed)
        {
            return Ok(());
        }
        let Some(ask) = context.market_state().best_ask() else {
            return Ok(());
        };
        if ask.price() > self.entry_price {
            return Ok(());
        }
        let instrument = context.market_state().instrument().clone();
        let quantity = Quantity::new(self.quantity, Self::quantity_unit(&instrument))
            .map_err(|error| StrategyExecutionError::new(error.to_string()))?;
        output.emit_order_intent(OrderIntent::new(
            instrument,
            OrderSide::Buy,
            quantity,
            OrderType::Limit {
                limit_price: self.entry_price,
            },
        ))?;
        self.submitted = true;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PriceThresholdBuyOnceFactory {
    definition: StrategyDefinition,
    schema: StrategyParameterSchema,
}

impl PriceThresholdBuyOnceFactory {
    pub fn new() -> Result<Self, StrategyRegistryError> {
        let schema = StrategyParameterSchema::new(
            1,
            [
                StrategyParameterField::required(
                    "entry_price",
                    StrategyParameterType::ExactDecimal(ParameterRange::new(
                        RangeBound::Excluded(Decimal::ZERO),
                        RangeBound::Unbounded,
                    )),
                ),
                StrategyParameterField::optional_with_default(
                    "quantity",
                    StrategyParameterType::UnsignedInteger(ParameterRange::new(
                        RangeBound::Included(1),
                        RangeBound::Unbounded,
                    )),
                    StrategyParameterValue::UnsignedInteger(1),
                ),
            ],
        )?;
        let digest = blake3::hash(include_bytes!("lib.rs"));
        let binary_identity = BinaryIdentity::new("strategy-source-blake3", *digest.as_bytes())
            .map_err(|_| StrategyRegistryError::InvalidDefinition)?;
        let definition = StrategyDefinition::new(
            EXAMPLE_STRATEGY_ID,
            EXAMPLE_STRATEGY_VERSION,
            binary_identity,
            schema.version(),
        )?;
        Ok(Self { definition, schema })
    }
}

impl StrategyFactory for PriceThresholdBuyOnceFactory {
    fn definition(&self) -> &StrategyDefinition {
        &self.definition
    }

    fn parameter_schema(&self) -> &StrategyParameterSchema {
        &self.schema
    }

    fn build(
        &self,
        parameters: &ValidatedStrategyParameters,
        universe: &[InstrumentId],
        sessions: &[SessionKind],
    ) -> Result<Box<dyn Strategy>, StrategyFactoryError> {
        let [instrument] = universe else {
            return Err(StrategyFactoryError::new(
                "example strategy requires exactly one explicit instrument",
            ));
        };
        let entry_price = parameters
            .get("entry_price")
            .and_then(StrategyParameterValue::as_exact_decimal)
            .ok_or_else(|| StrategyFactoryError::new("validated entry_price is missing"))?;
        let quantity = parameters
            .get("quantity")
            .and_then(StrategyParameterValue::as_unsigned_integer)
            .ok_or_else(|| StrategyFactoryError::new("validated quantity is missing"))?;
        let identity = self
            .definition
            .identity()
            .map_err(|error| StrategyFactoryError::new(error.to_string()))?;
        let declaration = StrategyDeclaration::new([instrument.clone()], sessions.iter().copied())
            .map_err(|error| StrategyFactoryError::new(error.to_string()))?;
        Ok(Box::new(PriceThresholdBuyOnce {
            identity,
            params_checksum: parameters.checksum(),
            declaration,
            entry_price: Price::new(entry_price)
                .map_err(|error| StrategyFactoryError::new(error.to_string()))?,
            quantity,
            submitted: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use market_state::{
        MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
    };
    use market_types::{
        BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent, EventPayload,
        MarketAnnotations, MarketId, MatchTime, Observation, QuoteSnapshot, SourceFormatId, Symbol,
        TradingDate, TwseQuoteAnnotations, Volume,
    };
    use replay_engine::ReplayCore;
    use strategy_api::{
        RawStrategyParameter, SessionSegment, StrategyOutputSink, StrategyRegistry,
        TwseTradingContextEvaluator,
    };

    use super::*;

    fn instrument(market: MarketId) -> InstrumentId {
        InstrumentId::new(market, Symbol::new("TEST").unwrap())
    }

    #[test]
    fn factory_materializes_default_and_market_quantity_unit() {
        let mut registry = StrategyRegistry::new();
        registry
            .register(PriceThresholdBuyOnceFactory::new().unwrap())
            .unwrap();
        let parameters = BTreeMap::from([(
            "entry_price".to_owned(),
            RawStrategyParameter::String("101.0".to_owned()),
        )]);
        for market in [MarketId::Twse, MarketId::Tpex, MarketId::Taifex] {
            let resolved = registry
                .resolve(
                    EXAMPLE_STRATEGY_ID,
                    EXAMPLE_STRATEGY_VERSION,
                    &parameters,
                    &[instrument(market)],
                    &[SessionKind::Regular],
                )
                .unwrap();
            let (_, metadata) = resolved.into_parts();
            assert_eq!(
                metadata
                    .parameters()
                    .get("quantity")
                    .and_then(StrategyParameterValue::as_unsigned_integer),
                Some(1)
            );
            assert_eq!(
                PriceThresholdBuyOnce::quantity_unit(&instrument(market)),
                match market {
                    MarketId::Twse | MarketId::Tpex => QuantityUnit::TradingUnit,
                    MarketId::Taifex => QuantityUnit::Contract,
                }
            );
        }
    }

    #[test]
    fn factory_rejects_multiple_instruments_and_nonpositive_price() {
        let factory = PriceThresholdBuyOnceFactory::new().unwrap();
        let mut registry = StrategyRegistry::new();
        registry.register(factory).unwrap();
        let zero = BTreeMap::from([(
            "entry_price".to_owned(),
            RawStrategyParameter::String("0".to_owned()),
        )]);
        assert!(
            registry
                .resolve(
                    EXAMPLE_STRATEGY_ID,
                    EXAMPLE_STRATEGY_VERSION,
                    &zero,
                    &[instrument(MarketId::Twse)],
                    &[SessionKind::Regular],
                )
                .is_err()
        );

        let valid = BTreeMap::from([(
            "entry_price".to_owned(),
            RawStrategyParameter::String("101".to_owned()),
        )]);
        assert!(
            registry
                .resolve(
                    EXAMPLE_STRATEGY_ID,
                    EXAMPLE_STRATEGY_VERSION,
                    &valid,
                    &[instrument(MarketId::Twse), instrument(MarketId::Tpex)],
                    &[SessionKind::Regular],
                )
                .is_err()
        );
    }

    fn twse_quote(time: &str, ask: &str, status: u8) -> DomainEvent {
        let quantity = Quantity::new(10, QuantityUnit::TradingUnit).unwrap();
        let book = CompleteBookSnapshot::new(
            BookSide::new(
                BookSideKind::Bid,
                vec![BookLevel::new(Price::parse("99").unwrap(), quantity)],
            )
            .unwrap(),
            BookSide::new(
                BookSideKind::Ask,
                vec![BookLevel::new(Price::parse(ask).unwrap(), quantity)],
            )
            .unwrap(),
        )
        .unwrap();
        DomainEvent::new(
            instrument(MarketId::Twse),
            TradingDate::parse("2026-07-20").unwrap(),
            SourceFormatId::new("STOCK_SNAPSHOT").unwrap(),
            MatchTime::parse(time).unwrap(),
            None,
            EventPayload::QuoteSnapshot(
                QuoteSnapshot::new(
                    book,
                    Observation::NoObservation,
                    Observation::Set(Volume::new(1, QuantityUnit::TradingUnit)),
                    MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(status, 0)),
                )
                .unwrap(),
            ),
        )
    }

    #[test]
    fn strategy_waits_for_eligibility_and_submits_only_once_at_threshold() {
        let twse = instrument(MarketId::Twse);
        let date = TradingDate::parse("2026-07-20").unwrap();
        let segment_id = SessionSegmentId::new("regular").unwrap();
        let segment = SessionSegment::new(
            segment_id.clone(),
            SessionKind::Regular,
            date,
            MatchTime::parse("2026-07-20T09:00:00+08:00").unwrap(),
            MatchTime::parse("2026-07-20T13:30:00+08:00").unwrap(),
        )
        .unwrap();
        let mut core = ReplayCore::new(
            vec![MarketState::new(twse.clone(), date)],
            MarketStateReducer::twse_regular(),
            ReducerContext::new(date, segment_id, SegmentBoundaryPolicy::Carry, 1),
        )
        .unwrap();
        let mut registry = StrategyRegistry::new();
        registry
            .register(PriceThresholdBuyOnceFactory::new().unwrap())
            .unwrap();
        let parameters = BTreeMap::from([(
            "entry_price".to_owned(),
            RawStrategyParameter::String("101".to_owned()),
        )]);
        let mut strategy = registry
            .resolve(
                EXAMPLE_STRATEGY_ID,
                EXAMPLE_STRATEGY_VERSION,
                &parameters,
                std::slice::from_ref(&twse),
                &[SessionKind::Regular],
            )
            .unwrap()
            .into_parts()
            .0;
        let events = [
            twse_quote("2026-07-20T08:59:59+08:00", "100", 0x80),
            twse_quote("2026-07-20T09:00:00+08:00", "102", 0x10),
            twse_quote("2026-07-20T09:00:01+08:00", "101", 0x10),
            twse_quote("2026-07-20T09:00:02+08:00", "100", 0x10),
        ];
        let mut emitted = Vec::new();
        for event in &events {
            let commit = core.apply_ordered(event).unwrap();
            let state = core.state(&twse).unwrap().view();
            let trading = TwseTradingContextEvaluator
                .evaluate(event, commit.occurrence(), state, &segment)
                .unwrap();
            let mut sink = StrategyOutputSink::with_order_intents();
            strategy
                .on_event(
                    StrategyEventContext::new(commit.occurrence(), event, state, &trading),
                    &mut sink,
                )
                .unwrap();
            emitted.extend(sink.intents().iter().cloned());
        }
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].quantity().value(), 1);
        assert_eq!(emitted[0].quantity().unit(), QuantityUnit::TradingUnit);
        assert_eq!(
            emitted[0].order_type(),
            OrderType::Limit {
                limit_price: Price::parse("101").unwrap()
            }
        );
    }
}
