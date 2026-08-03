use std::{
    collections::{BTreeMap, btree_map::Entry},
    error::Error,
    fmt,
};

use market_types::{Decimal, InstrumentId};

use crate::{
    BinaryIdentity, CanonicalParamsChecksum, SessionKind, Strategy, StrategyDeclaration,
    StrategyIdentity,
};

pub const STRATEGY_PARAMETER_CANONICAL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawStrategyParameter {
    Bool(bool),
    SignedInteger(i64),
    UnsignedInteger(u64),
    String(String),
}

pub type RawStrategyParameters = BTreeMap<String, RawStrategyParameter>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyParameterValue {
    Bool(bool),
    SignedInteger(i64),
    UnsignedInteger(u64),
    ExactDecimal(Decimal),
    Text(Box<str>),
}

impl StrategyParameterValue {
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_signed_integer(&self) -> Option<i64> {
        match self {
            Self::SignedInteger(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_unsigned_integer(&self) -> Option<u64> {
        match self {
            Self::UnsignedInteger(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_exact_decimal(&self) -> Option<Decimal> {
        match self {
            Self::ExactDecimal(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn materialized_text(&self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::SignedInteger(value) => value.to_string(),
            Self::UnsignedInteger(value) => value.to_string(),
            Self::ExactDecimal(value) => canonical_decimal(*value),
            Self::Text(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RangeBound<T> {
    #[default]
    Unbounded,
    Included(T),
    Excluded(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParameterRange<T> {
    minimum: RangeBound<T>,
    maximum: RangeBound<T>,
}

impl<T> ParameterRange<T> {
    #[must_use]
    pub const fn new(minimum: RangeBound<T>, maximum: RangeBound<T>) -> Self {
        Self { minimum, maximum }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyParameterType {
    Bool,
    SignedInteger(ParameterRange<i64>),
    UnsignedInteger(ParameterRange<u64>),
    ExactDecimal(ParameterRange<Decimal>),
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyParameterField {
    name: Box<str>,
    parameter_type: StrategyParameterType,
    required: bool,
    default: Option<StrategyParameterValue>,
}

impl StrategyParameterField {
    #[must_use]
    pub fn required(name: impl Into<Box<str>>, parameter_type: StrategyParameterType) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: true,
            default: None,
        }
    }

    #[must_use]
    pub fn optional(name: impl Into<Box<str>>, parameter_type: StrategyParameterType) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: false,
            default: None,
        }
    }

    #[must_use]
    pub fn optional_with_default(
        name: impl Into<Box<str>>,
        parameter_type: StrategyParameterType,
        default: StrategyParameterValue,
    ) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: false,
            default: Some(default),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyParameterSchema {
    version: u16,
    fields: Box<[StrategyParameterField]>,
}

impl StrategyParameterSchema {
    pub fn new(
        version: u16,
        fields: impl IntoIterator<Item = StrategyParameterField>,
    ) -> Result<Self, StrategyRegistryError> {
        if version == 0 {
            return Err(StrategyRegistryError::InvalidSchema(
                "schema version must be greater than zero".into(),
            ));
        }
        let mut fields = fields.into_iter().collect::<Vec<_>>();
        fields.sort_by(|left, right| left.name.cmp(&right.name));
        for pair in fields.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(StrategyRegistryError::InvalidSchema(format!(
                    "duplicate parameter field `{}`",
                    pair[0].name
                )));
            }
        }
        for field in &fields {
            validate_field(field)?;
        }
        Ok(Self {
            version,
            fields: fields.into_boxed_slice(),
        })
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    #[must_use]
    pub const fn fields(&self) -> &[StrategyParameterField] {
        &self.fields
    }

    pub fn validate(
        &self,
        raw: &RawStrategyParameters,
    ) -> Result<ValidatedStrategyParameters, StrategyRegistryError> {
        for name in raw.keys() {
            if self
                .fields
                .binary_search_by(|field| field.name.as_ref().cmp(name))
                .is_err()
            {
                return Err(StrategyRegistryError::UnknownParameter(name.clone()));
            }
        }
        let mut values = BTreeMap::new();
        for field in &self.fields {
            let value = match raw.get(field.name()) {
                Some(raw) => parse_value(field, raw)?,
                None => match &field.default {
                    Some(default) => default.clone(),
                    None if field.required => {
                        return Err(StrategyRegistryError::MissingParameter(
                            field.name.to_string(),
                        ));
                    }
                    None => continue,
                },
            };
            validate_range(field, &value)?;
            values.insert(field.name.to_string(), value);
        }
        ValidatedStrategyParameters::new(self.version, values)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedStrategyParameters {
    schema_version: u16,
    values: BTreeMap<String, StrategyParameterValue>,
    canonical_bytes: Box<[u8]>,
    checksum: CanonicalParamsChecksum,
}

impl ValidatedStrategyParameters {
    fn new(
        schema_version: u16,
        values: BTreeMap<String, StrategyParameterValue>,
    ) -> Result<Self, StrategyRegistryError> {
        let canonical_bytes = encode_parameters(schema_version, &values)?;
        let checksum =
            CanonicalParamsChecksum::from_bytes(*blake3::hash(&canonical_bytes).as_bytes());
        Ok(Self {
            schema_version,
            values,
            canonical_bytes: canonical_bytes.into_boxed_slice(),
            checksum,
        })
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub fn values(&self) -> &BTreeMap<String, StrategyParameterValue> {
        &self.values
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&StrategyParameterValue> {
        self.values.get(name)
    }

    #[must_use]
    pub const fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    #[must_use]
    pub const fn checksum(&self) -> CanonicalParamsChecksum {
        self.checksum
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyDefinition {
    id: Box<str>,
    version: Box<str>,
    binary_identity: BinaryIdentity,
    parameter_schema_version: u16,
}

impl StrategyDefinition {
    pub fn new(
        id: impl Into<Box<str>>,
        version: impl Into<Box<str>>,
        binary_identity: BinaryIdentity,
        parameter_schema_version: u16,
    ) -> Result<Self, StrategyRegistryError> {
        let id = id.into();
        let version = version.into();
        if id.is_empty() || version.is_empty() || parameter_schema_version == 0 {
            return Err(StrategyRegistryError::InvalidDefinition);
        }
        Ok(Self {
            id,
            version,
            binary_identity,
            parameter_schema_version,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn binary_identity(&self) -> &BinaryIdentity {
        &self.binary_identity
    }

    #[must_use]
    pub const fn parameter_schema_version(&self) -> u16 {
        self.parameter_schema_version
    }

    pub fn identity(&self) -> Result<StrategyIdentity, StrategyRegistryError> {
        StrategyIdentity::new(
            self.id.clone(),
            self.version.clone(),
            self.binary_identity.clone(),
        )
        .map_err(|_| StrategyRegistryError::InvalidDefinition)
    }
}

pub trait StrategyFactory: Send + Sync {
    fn definition(&self) -> &StrategyDefinition;

    fn parameter_schema(&self) -> &StrategyParameterSchema;

    /// Builds an already-selected strategy. Implementations must not perform I/O,
    /// inspect environment variables, or expand the explicit universe.
    fn build(
        &self,
        parameters: &ValidatedStrategyParameters,
        universe: &[InstrumentId],
        sessions: &[SessionKind],
    ) -> Result<Box<dyn Strategy>, StrategyFactoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyFactoryError {
    message: Box<str>,
}

impl StrategyFactoryError {
    #[must_use]
    pub fn new(message: impl Into<Box<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for StrategyFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for StrategyFactoryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStrategyMetadata {
    definition: StrategyDefinition,
    parameters: ValidatedStrategyParameters,
    declaration: StrategyDeclaration,
}

impl ResolvedStrategyMetadata {
    #[must_use]
    pub const fn definition(&self) -> &StrategyDefinition {
        &self.definition
    }

    #[must_use]
    pub const fn parameters(&self) -> &ValidatedStrategyParameters {
        &self.parameters
    }

    #[must_use]
    pub const fn declaration(&self) -> &StrategyDeclaration {
        &self.declaration
    }
}

pub struct ResolvedStrategy {
    strategy: Box<dyn Strategy>,
    metadata: ResolvedStrategyMetadata,
}

impl ResolvedStrategy {
    #[must_use]
    pub fn into_parts(self) -> (Box<dyn Strategy>, ResolvedStrategyMetadata) {
        (self.strategy, self.metadata)
    }
}

#[derive(Default)]
pub struct StrategyRegistry {
    factories: BTreeMap<(String, String), Box<dyn StrategyFactory>>,
}

impl StrategyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F: StrategyFactory + 'static>(
        &mut self,
        factory: F,
    ) -> Result<(), StrategyRegistryError> {
        let definition = factory.definition();
        if definition.parameter_schema_version() != factory.parameter_schema().version() {
            return Err(StrategyRegistryError::SchemaVersionMismatch {
                definition: definition.parameter_schema_version(),
                schema: factory.parameter_schema().version(),
            });
        }
        let key = (definition.id().to_owned(), definition.version().to_owned());
        match self.factories.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(Box::new(factory));
                Ok(())
            }
            Entry::Occupied(entry) => Err(StrategyRegistryError::DuplicateRegistration {
                id: entry.key().0.clone(),
                version: entry.key().1.clone(),
            }),
        }
    }

    pub fn resolve(
        &self,
        id: &str,
        version: &str,
        raw: &RawStrategyParameters,
        universe: &[InstrumentId],
        sessions: &[SessionKind],
    ) -> Result<ResolvedStrategy, StrategyRegistryError> {
        let key = (id.to_owned(), version.to_owned());
        let factory = match self.factories.get(&key) {
            Some(factory) => factory,
            None => {
                let versions = self
                    .factories
                    .keys()
                    .filter(|(registered_id, _)| registered_id == id)
                    .map(|(_, version)| version.clone())
                    .collect::<Vec<_>>();
                return if versions.is_empty() {
                    Err(StrategyRegistryError::UnknownStrategy { id: id.to_owned() })
                } else {
                    Err(StrategyRegistryError::VersionMismatch {
                        id: id.to_owned(),
                        requested: version.to_owned(),
                        available: versions,
                    })
                };
            }
        };
        let parameters = factory.parameter_schema().validate(raw)?;
        let strategy = factory
            .build(&parameters, universe, sessions)
            .map_err(StrategyRegistryError::Factory)?;
        let definition = factory.definition().clone();
        let expected_identity = definition.identity()?;
        if strategy.identity() != &expected_identity {
            return Err(StrategyRegistryError::FactoryContractMismatch(
                FactoryContractField::Identity,
            ));
        }
        if strategy.canonical_params_checksum() != parameters.checksum() {
            return Err(StrategyRegistryError::FactoryContractMismatch(
                FactoryContractField::ParameterChecksum,
            ));
        }
        let declaration = strategy.declaration();
        if declaration.universe() != universe {
            return Err(StrategyRegistryError::FactoryContractMismatch(
                FactoryContractField::Universe,
            ));
        }
        if declaration.sessions() != sessions {
            return Err(StrategyRegistryError::FactoryContractMismatch(
                FactoryContractField::Sessions,
            ));
        }
        Ok(ResolvedStrategy {
            strategy,
            metadata: ResolvedStrategyMetadata {
                definition,
                parameters,
                declaration,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactoryContractField {
    Identity,
    ParameterChecksum,
    Universe,
    Sessions,
}

#[derive(Debug)]
pub enum StrategyRegistryError {
    InvalidDefinition,
    InvalidSchema(String),
    SchemaVersionMismatch {
        definition: u16,
        schema: u16,
    },
    DuplicateRegistration {
        id: String,
        version: String,
    },
    UnknownStrategy {
        id: String,
    },
    VersionMismatch {
        id: String,
        requested: String,
        available: Vec<String>,
    },
    MissingParameter(String),
    UnknownParameter(String),
    WrongParameterType {
        field: String,
        expected: &'static str,
    },
    ParameterOutOfRange(String),
    InvalidDecimal {
        field: String,
        message: String,
    },
    ParametersTooLarge,
    Factory(StrategyFactoryError),
    FactoryContractMismatch(FactoryContractField),
}

impl fmt::Display for StrategyRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDefinition => formatter.write_str("invalid strategy definition"),
            Self::InvalidSchema(message) => {
                write!(formatter, "invalid strategy parameter schema: {message}")
            }
            Self::SchemaVersionMismatch { definition, schema } => write!(
                formatter,
                "strategy definition schema version {definition} does not match schema version {schema}"
            ),
            Self::DuplicateRegistration { id, version } => write!(
                formatter,
                "strategy `{id}` version `{version}` is registered more than once"
            ),
            Self::UnknownStrategy { id } => write!(
                formatter,
                "strategy `{id}` is not compiled into this binary"
            ),
            Self::VersionMismatch {
                id,
                requested,
                available,
            } => write!(
                formatter,
                "strategy `{id}` version `{requested}` is unavailable; compiled versions: {}",
                available.join(", ")
            ),
            Self::MissingParameter(field) => {
                write!(formatter, "missing required strategy parameter `{field}`")
            }
            Self::UnknownParameter(field) => {
                write!(formatter, "unknown strategy parameter `{field}`")
            }
            Self::WrongParameterType { field, expected } => {
                write!(formatter, "strategy parameter `{field}` must be {expected}")
            }
            Self::ParameterOutOfRange(field) => write!(
                formatter,
                "strategy parameter `{field}` is outside the allowed range"
            ),
            Self::InvalidDecimal { field, message } => write!(
                formatter,
                "strategy parameter `{field}` is not an exact decimal: {message}"
            ),
            Self::ParametersTooLarge => {
                formatter.write_str("strategy parameters are too large to encode")
            }
            Self::Factory(error) => write!(formatter, "strategy factory failed: {error}"),
            Self::FactoryContractMismatch(field) => {
                write!(formatter, "strategy factory contract mismatch: {field:?}")
            }
        }
    }
}

impl Error for StrategyRegistryError {}

fn validate_field(field: &StrategyParameterField) -> Result<(), StrategyRegistryError> {
    if field.name.is_empty() || field.name.as_bytes().contains(&0) {
        return Err(StrategyRegistryError::InvalidSchema(
            "parameter names must be non-empty and contain no NUL".into(),
        ));
    }
    if field.required && field.default.is_some() {
        return Err(StrategyRegistryError::InvalidSchema(format!(
            "required parameter `{}` must not define a default",
            field.name
        )));
    }
    let range_is_valid = match &field.parameter_type {
        StrategyParameterType::SignedInteger(range) => valid_range(range),
        StrategyParameterType::UnsignedInteger(range) => valid_range(range),
        StrategyParameterType::ExactDecimal(range) => valid_range(range),
        StrategyParameterType::Bool | StrategyParameterType::Text => true,
    };
    if !range_is_valid {
        return Err(StrategyRegistryError::InvalidSchema(format!(
            "parameter `{}` has an empty or reversed range",
            field.name
        )));
    }
    if let Some(default) = &field.default {
        validate_value_type(field, default)?;
        validate_range(field, default)?;
    }
    Ok(())
}

fn valid_range<T: Ord>(range: &ParameterRange<T>) -> bool {
    match (&range.minimum, &range.maximum) {
        (RangeBound::Unbounded, _) | (_, RangeBound::Unbounded) => true,
        (RangeBound::Included(minimum), RangeBound::Included(maximum)) => minimum <= maximum,
        (RangeBound::Included(minimum), RangeBound::Excluded(maximum))
        | (RangeBound::Excluded(minimum), RangeBound::Included(maximum))
        | (RangeBound::Excluded(minimum), RangeBound::Excluded(maximum)) => minimum < maximum,
    }
}

fn parse_value(
    field: &StrategyParameterField,
    raw: &RawStrategyParameter,
) -> Result<StrategyParameterValue, StrategyRegistryError> {
    let wrong_type = || StrategyRegistryError::WrongParameterType {
        field: field.name.to_string(),
        expected: match field.parameter_type {
            StrategyParameterType::Bool => "a bool",
            StrategyParameterType::SignedInteger(_) => "a signed integer",
            StrategyParameterType::UnsignedInteger(_) => "an unsigned integer",
            StrategyParameterType::ExactDecimal(_) => "a quoted exact decimal string",
            StrategyParameterType::Text => "text",
        },
    };
    match (&field.parameter_type, raw) {
        (StrategyParameterType::Bool, RawStrategyParameter::Bool(value)) => {
            Ok(StrategyParameterValue::Bool(*value))
        }
        (StrategyParameterType::SignedInteger(_), RawStrategyParameter::SignedInteger(value)) => {
            Ok(StrategyParameterValue::SignedInteger(*value))
        }
        (StrategyParameterType::SignedInteger(_), RawStrategyParameter::UnsignedInteger(value)) => {
            i64::try_from(*value)
                .map(StrategyParameterValue::SignedInteger)
                .map_err(|_| wrong_type())
        }
        (
            StrategyParameterType::UnsignedInteger(_),
            RawStrategyParameter::UnsignedInteger(value),
        ) => Ok(StrategyParameterValue::UnsignedInteger(*value)),
        (StrategyParameterType::UnsignedInteger(_), RawStrategyParameter::SignedInteger(value))
            if *value >= 0 =>
        {
            Ok(StrategyParameterValue::UnsignedInteger(*value as u64))
        }
        (StrategyParameterType::ExactDecimal(_), RawStrategyParameter::String(value)) => {
            Decimal::parse(value)
                .map(StrategyParameterValue::ExactDecimal)
                .map_err(|error| StrategyRegistryError::InvalidDecimal {
                    field: field.name.to_string(),
                    message: error.to_string(),
                })
        }
        (StrategyParameterType::Text, RawStrategyParameter::String(value)) => {
            Ok(StrategyParameterValue::Text(value.clone().into_boxed_str()))
        }
        _ => Err(wrong_type()),
    }
}

fn validate_value_type(
    field: &StrategyParameterField,
    value: &StrategyParameterValue,
) -> Result<(), StrategyRegistryError> {
    let valid = matches!(
        (&field.parameter_type, value),
        (StrategyParameterType::Bool, StrategyParameterValue::Bool(_))
            | (
                StrategyParameterType::SignedInteger(_),
                StrategyParameterValue::SignedInteger(_)
            )
            | (
                StrategyParameterType::UnsignedInteger(_),
                StrategyParameterValue::UnsignedInteger(_)
            )
            | (
                StrategyParameterType::ExactDecimal(_),
                StrategyParameterValue::ExactDecimal(_)
            )
            | (StrategyParameterType::Text, StrategyParameterValue::Text(_))
    );
    if valid {
        Ok(())
    } else {
        Err(StrategyRegistryError::InvalidSchema(format!(
            "default for `{}` has the wrong type",
            field.name
        )))
    }
}

fn validate_range(
    field: &StrategyParameterField,
    value: &StrategyParameterValue,
) -> Result<(), StrategyRegistryError> {
    let valid = match (&field.parameter_type, value) {
        (
            StrategyParameterType::SignedInteger(range),
            StrategyParameterValue::SignedInteger(value),
        ) => range_contains(range, value),
        (
            StrategyParameterType::UnsignedInteger(range),
            StrategyParameterValue::UnsignedInteger(value),
        ) => range_contains(range, value),
        (
            StrategyParameterType::ExactDecimal(range),
            StrategyParameterValue::ExactDecimal(value),
        ) => range_contains(range, value),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(StrategyRegistryError::ParameterOutOfRange(
            field.name.to_string(),
        ))
    }
}

fn range_contains<T: Ord>(range: &ParameterRange<T>, value: &T) -> bool {
    let minimum = match &range.minimum {
        RangeBound::Unbounded => true,
        RangeBound::Included(minimum) => value >= minimum,
        RangeBound::Excluded(minimum) => value > minimum,
    };
    let maximum = match &range.maximum {
        RangeBound::Unbounded => true,
        RangeBound::Included(maximum) => value <= maximum,
        RangeBound::Excluded(maximum) => value < maximum,
    };
    minimum && maximum
}

fn encode_parameters(
    schema_version: u16,
    values: &BTreeMap<String, StrategyParameterValue>,
) -> Result<Vec<u8>, StrategyRegistryError> {
    let mut payload = Vec::new();
    for (name, value) in values {
        append_bytes(name.as_bytes(), &mut payload)?;
        match value {
            StrategyParameterValue::Bool(value) => {
                payload.push(1);
                payload.push(u8::from(*value));
            }
            StrategyParameterValue::SignedInteger(value) => {
                payload.push(2);
                payload.extend_from_slice(&value.to_be_bytes());
            }
            StrategyParameterValue::UnsignedInteger(value) => {
                payload.push(3);
                payload.extend_from_slice(&value.to_be_bytes());
            }
            StrategyParameterValue::ExactDecimal(value) => {
                payload.push(4);
                payload.extend_from_slice(&value.to_canonical_bytes());
            }
            StrategyParameterValue::Text(value) => {
                payload.push(5);
                append_bytes(value.as_bytes(), &mut payload)?;
            }
        }
    }
    let mut output = Vec::new();
    output.extend_from_slice(b"OSSP");
    output.extend_from_slice(&schema_version.to_be_bytes());
    append_bytes(&payload, &mut output)?;
    Ok(output)
}

fn append_bytes(value: &[u8], output: &mut Vec<u8>) -> Result<(), StrategyRegistryError> {
    output.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| StrategyRegistryError::ParametersTooLarge)?
            .to_be_bytes(),
    );
    output.extend_from_slice(value);
    Ok(())
}

fn canonical_decimal(value: Decimal) -> String {
    let atoms = value.atoms();
    let negative = atoms.is_negative();
    let magnitude = atoms.unsigned_abs();
    let scale = Decimal::SCALE_FACTOR as u128;
    let whole = magnitude / scale;
    let fraction = magnitude % scale;
    let sign = if negative { "-" } else { "" };
    if fraction == 0 {
        return format!("{sign}{whole}");
    }
    let fraction = format!("{fraction:018}").trim_end_matches('0').to_owned();
    format!("{sign}{whole}.{fraction}")
}
