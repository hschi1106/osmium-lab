use std::collections::BTreeMap;

use market_types::{InstrumentId, MarketId, Symbol};
use strategy_api::{
    BinaryIdentity, CanonicalParamsChecksum, FactoryContractField, ParameterRange, RangeBound,
    RawStrategyParameter, SessionKind, Strategy, StrategyDeclaration, StrategyDefinition,
    StrategyEventContext, StrategyExecutionError, StrategyFactory, StrategyFactoryError,
    StrategyIdentity, StrategyOutputSink, StrategyParameterField, StrategyParameterSchema,
    StrategyParameterType, StrategyParameterValue, StrategyRegistry, StrategyRegistryError,
    ValidatedStrategyParameters,
};

fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::new(MarketId::Twse, Symbol::new(symbol).unwrap())
}

#[derive(Clone, Copy)]
enum ContractMode {
    Valid,
    Identity,
    Checksum,
    Universe,
    Sessions,
}

struct TestFactory {
    definition: StrategyDefinition,
    schema: StrategyParameterSchema,
    mode: ContractMode,
}

impl TestFactory {
    fn new(mode: ContractMode) -> Self {
        let schema = StrategyParameterSchema::new(
            7,
            [
                StrategyParameterField::required("enabled", StrategyParameterType::Bool),
                StrategyParameterField::optional_with_default(
                    "quantity",
                    StrategyParameterType::UnsignedInteger(ParameterRange::new(
                        RangeBound::Included(1),
                        RangeBound::Included(10),
                    )),
                    StrategyParameterValue::UnsignedInteger(1),
                ),
            ],
        )
        .unwrap();
        let definition = StrategyDefinition::new(
            "test.registry",
            "1",
            BinaryIdentity::new("test", [4; 32]).unwrap(),
            schema.version(),
        )
        .unwrap();
        Self {
            definition,
            schema,
            mode,
        }
    }
}

struct TestStrategy {
    identity: StrategyIdentity,
    checksum: CanonicalParamsChecksum,
    declaration: StrategyDeclaration,
}

impl Strategy for TestStrategy {
    fn identity(&self) -> &StrategyIdentity {
        &self.identity
    }

    fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
        self.checksum
    }

    fn declaration(&self) -> StrategyDeclaration {
        self.declaration.clone()
    }

    fn on_event(
        &mut self,
        _context: StrategyEventContext<'_>,
        _output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        Ok(())
    }
}

impl StrategyFactory for TestFactory {
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
        let identity = if matches!(self.mode, ContractMode::Identity) {
            StrategyIdentity::new("wrong", "1", self.definition.binary_identity().clone()).unwrap()
        } else {
            self.definition.identity().unwrap()
        };
        let checksum = if matches!(self.mode, ContractMode::Checksum) {
            CanonicalParamsChecksum::from_bytes([9; 32])
        } else {
            parameters.checksum()
        };
        let declared_universe = if matches!(self.mode, ContractMode::Universe) {
            vec![instrument("DIFFERENT")]
        } else {
            universe.to_vec()
        };
        let declared_sessions = if matches!(self.mode, ContractMode::Sessions) {
            vec![SessionKind::AfterHours]
        } else {
            sessions.to_vec()
        };
        Ok(Box::new(TestStrategy {
            identity,
            checksum,
            declaration: StrategyDeclaration::new(declared_universe, declared_sessions).unwrap(),
        }))
    }
}

fn valid_parameters() -> BTreeMap<String, RawStrategyParameter> {
    BTreeMap::from([("enabled".to_owned(), RawStrategyParameter::Bool(true))])
}

#[test]
fn registration_and_lookup_errors_are_specific() {
    let mut registry = StrategyRegistry::new();
    registry
        .register(TestFactory::new(ContractMode::Valid))
        .unwrap();
    assert!(matches!(
        registry.register(TestFactory::new(ContractMode::Valid)),
        Err(StrategyRegistryError::DuplicateRegistration { .. })
    ));
    assert!(matches!(
        registry.resolve(
            "missing",
            "1",
            &valid_parameters(),
            &[instrument("2330")],
            &[SessionKind::Regular]
        ),
        Err(StrategyRegistryError::UnknownStrategy { .. })
    ));
    assert!(matches!(
        registry.resolve(
            "test.registry",
            "2",
            &valid_parameters(),
            &[instrument("2330")],
            &[SessionKind::Regular]
        ),
        Err(StrategyRegistryError::VersionMismatch { .. })
    ));
}

#[test]
fn schema_rejects_missing_unknown_wrong_type_and_range() {
    let schema = TestFactory::new(ContractMode::Valid).schema;
    assert!(matches!(
        schema.validate(&BTreeMap::new()),
        Err(StrategyRegistryError::MissingParameter(field)) if field == "enabled"
    ));
    let unknown = BTreeMap::from([
        ("enabled".to_owned(), RawStrategyParameter::Bool(true)),
        ("extra".to_owned(), RawStrategyParameter::Bool(true)),
    ]);
    assert!(matches!(
        schema.validate(&unknown),
        Err(StrategyRegistryError::UnknownParameter(field)) if field == "extra"
    ));
    let wrong = BTreeMap::from([(
        "enabled".to_owned(),
        RawStrategyParameter::String("true".to_owned()),
    )]);
    assert!(matches!(
        schema.validate(&wrong),
        Err(StrategyRegistryError::WrongParameterType { .. })
    ));
    let range = BTreeMap::from([
        ("enabled".to_owned(), RawStrategyParameter::Bool(true)),
        (
            "quantity".to_owned(),
            RawStrategyParameter::SignedInteger(0),
        ),
    ]);
    assert!(matches!(
        schema.validate(&range),
        Err(StrategyRegistryError::ParameterOutOfRange(field)) if field == "quantity"
    ));
}

#[test]
fn defaults_and_key_order_have_one_canonical_identity() {
    let schema = TestFactory::new(ContractMode::Valid).schema;
    let omitted = schema.validate(&valid_parameters()).unwrap();
    let explicit = schema
        .validate(&BTreeMap::from([
            (
                "quantity".to_owned(),
                RawStrategyParameter::UnsignedInteger(1),
            ),
            ("enabled".to_owned(), RawStrategyParameter::Bool(true)),
        ]))
        .unwrap();
    assert_eq!(omitted.values(), explicit.values());
    assert_eq!(omitted.canonical_bytes(), explicit.canonical_bytes());
    assert_eq!(omitted.checksum(), explicit.checksum());
}

#[test]
fn factory_contract_checks_all_reported_fields() {
    for (mode, expected) in [
        (ContractMode::Identity, FactoryContractField::Identity),
        (
            ContractMode::Checksum,
            FactoryContractField::ParameterChecksum,
        ),
        (ContractMode::Universe, FactoryContractField::Universe),
        (ContractMode::Sessions, FactoryContractField::Sessions),
    ] {
        let mut registry = StrategyRegistry::new();
        registry.register(TestFactory::new(mode)).unwrap();
        assert!(matches!(
            registry.resolve(
                "test.registry",
                "1",
                &valid_parameters(),
                &[instrument("2330")],
                &[SessionKind::Regular]
            ),
            Err(StrategyRegistryError::FactoryContractMismatch(field)) if field == expected
        ));
    }
}
