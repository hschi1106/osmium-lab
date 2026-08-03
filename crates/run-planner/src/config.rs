use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use market_types::{Decimal, InstrumentId, QuantityUnit, TradingDate};
use strategy_api::{CanonicalParamsChecksum, SessionKind, StrategyDeclaration, StrategyIdentity};

use crate::canonical::{
    append_decimal, append_instrument, append_len, append_session, append_strategy_identity,
    append_text,
};

pub const CONFIG_SCHEMA_VERSION: u16 = 1;
pub const EFFECTIVE_CONFIG_VERSION: u16 = 2;
pub const SOURCE_POLICY_VERSION: u16 = 1;
pub const CACHE_POLICY_VERSION: u16 = 1;
pub const REPLAY_DATA_POLICY_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SourcePolicy {
    Strict = 1,
    ExplicitDegraded = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CachePolicy {
    ReuseOrRebuild = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ReplayDataPolicy {
    Strict = 1,
    ExplicitDegraded = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OutputPolicy {
    CreateNew = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Currency {
    Twd = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrencyAmount {
    currency: Currency,
    amount: Decimal,
}

impl CurrencyAmount {
    #[must_use]
    pub const fn new(currency: Currency, amount: Decimal) -> Self {
        Self { currency, amount }
    }

    #[must_use]
    pub const fn currency(self) -> Currency {
        self.currency
    }

    #[must_use]
    pub const fn amount(self) -> Decimal {
        self.amount
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FillEvidence {
    TopOfBook = 1,
    TradePrint = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum QuantityEvidence {
    Unlimited = 1,
    Observed = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillModelConfig {
    evidence: FillEvidence,
    quantity: QuantityEvidence,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LatencyConfig {
    market_data_latency_ms: u64,
    order_latency_ms: u64,
}

impl LatencyConfig {
    #[must_use]
    pub const fn new(market_data_latency_ms: u64, order_latency_ms: u64) -> Self {
        Self {
            market_data_latency_ms,
            order_latency_ms,
        }
    }

    #[must_use]
    pub const fn market_data_latency_ms(self) -> u64 {
        self.market_data_latency_ms
    }

    #[must_use]
    pub const fn order_latency_ms(self) -> u64 {
        self.order_latency_ms
    }
}

impl FillModelConfig {
    #[must_use]
    pub const fn new(evidence: FillEvidence, quantity: QuantityEvidence) -> Self {
        Self { evidence, quantity }
    }

    #[must_use]
    pub const fn evidence(self) -> FillEvidence {
        self.evidence
    }

    #[must_use]
    pub const fn quantity(self) -> QuantityEvidence {
        self.quantity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum QuantityAllocationConfig {
    AcceptanceSequence = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlippageModelConfig {
    AdverseFixedDelta { delta: Decimal },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ChargeSides {
    Buy = 1,
    Sell = 2,
    BuyAndSell = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RoundingPolicy {
    Down = 1,
    HalfUp = 2,
    Up = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargeConfig {
    rate: Decimal,
    applicable_sides: ChargeSides,
    minimum: Decimal,
    precision: u8,
    rounding: RoundingPolicy,
    provenance: Box<str>,
}

impl ChargeConfig {
    pub fn new(
        rate: Decimal,
        applicable_sides: ChargeSides,
        minimum: Decimal,
        precision: u8,
        rounding: RoundingPolicy,
        provenance: impl Into<Box<str>>,
    ) -> Self {
        Self {
            rate,
            applicable_sides,
            minimum,
            precision,
            rounding,
            provenance: provenance.into(),
        }
    }

    #[must_use]
    pub const fn rate(&self) -> Decimal {
        self.rate
    }

    #[must_use]
    pub const fn applicable_sides(&self) -> ChargeSides {
        self.applicable_sides
    }

    #[must_use]
    pub const fn minimum(&self) -> Decimal {
        self.minimum
    }

    #[must_use]
    pub const fn precision(&self) -> u8 {
        self.precision
    }

    #[must_use]
    pub const fn rounding(&self) -> RoundingPolicy {
        self.rounding
    }

    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulationConfig {
    fill_model: FillModelConfig,
    latency: LatencyConfig,
    quantity_allocation: QuantityAllocationConfig,
    slippage_model: SlippageModelConfig,
    fee_model: ChargeConfig,
    tax_model: ChargeConfig,
    initial_cash: CurrencyAmount,
    position_accounting: PositionAccountingConfig,
    marking_policy: MarkingPolicyConfig,
}

impl SimulationConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fill_model: FillModelConfig,
        quantity_allocation: QuantityAllocationConfig,
        slippage_model: SlippageModelConfig,
        fee_model: ChargeConfig,
        tax_model: ChargeConfig,
        initial_cash: CurrencyAmount,
        position_accounting: PositionAccountingConfig,
        marking_policy: MarkingPolicyConfig,
    ) -> Self {
        Self {
            fill_model,
            latency: LatencyConfig::default(),
            quantity_allocation,
            slippage_model,
            fee_model,
            tax_model,
            initial_cash,
            position_accounting,
            marking_policy,
        }
    }

    #[must_use]
    pub const fn fill_model(&self) -> FillModelConfig {
        self.fill_model
    }

    #[must_use]
    pub const fn latency(&self) -> LatencyConfig {
        self.latency
    }

    #[must_use]
    pub const fn with_latency(mut self, latency: LatencyConfig) -> Self {
        self.latency = latency;
        self
    }

    #[must_use]
    pub const fn quantity_allocation(&self) -> QuantityAllocationConfig {
        self.quantity_allocation
    }

    #[must_use]
    pub const fn slippage_model(&self) -> SlippageModelConfig {
        self.slippage_model
    }

    #[must_use]
    pub const fn fee_model(&self) -> &ChargeConfig {
        &self.fee_model
    }

    #[must_use]
    pub const fn tax_model(&self) -> &ChargeConfig {
        &self.tax_model
    }

    #[must_use]
    pub const fn initial_cash(&self) -> CurrencyAmount {
        self.initial_cash
    }

    #[must_use]
    pub const fn position_accounting(&self) -> PositionAccountingConfig {
        self.position_accounting
    }

    #[must_use]
    pub const fn marking_policy(&self) -> MarkingPolicyConfig {
        self.marking_policy
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PositionAccountingConfig {
    AverageCostV1 = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkingPolicyConfig {
    LastObservableV1 { allow_midpoint_fallback: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentEconomicsConfig {
    instrument: InstrumentId,
    quantity_unit: QuantityUnit,
    units_per_trading_unit: u64,
    currency: Currency,
    multiplier: Decimal,
    provenance: Box<str>,
}

impl InstrumentEconomicsConfig {
    pub fn new(
        instrument: InstrumentId,
        quantity_unit: QuantityUnit,
        units_per_trading_unit: u64,
        currency: Currency,
        multiplier: Decimal,
        provenance: impl Into<Box<str>>,
    ) -> Self {
        Self {
            instrument,
            quantity_unit,
            units_per_trading_unit,
            currency,
            multiplier,
            provenance: provenance.into(),
        }
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
    pub const fn units_per_trading_unit(&self) -> u64 {
        self.units_per_trading_unit
    }

    #[must_use]
    pub const fn currency(&self) -> Currency {
        self.currency
    }

    #[must_use]
    pub const fn multiplier(&self) -> Decimal {
        self.multiplier
    }

    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyBinding {
    identity: StrategyIdentity,
    params_checksum: CanonicalParamsChecksum,
    declaration: StrategyDeclaration,
}

impl StrategyBinding {
    #[must_use]
    pub const fn new(
        identity: StrategyIdentity,
        params_checksum: CanonicalParamsChecksum,
        declaration: StrategyDeclaration,
    ) -> Self {
        Self {
            identity,
            params_checksum,
            declaration,
        }
    }

    #[must_use]
    pub const fn identity(&self) -> &StrategyIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn params_checksum(&self) -> CanonicalParamsChecksum {
        self.params_checksum
    }

    #[must_use]
    pub const fn declaration(&self) -> &StrategyDeclaration {
        &self.declaration
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConfig {
    pub config_version: u16,
    pub trading_dates: Vec<TradingDate>,
    pub universe: Vec<InstrumentId>,
    pub session_kinds: Vec<SessionKind>,
    pub strategy: StrategyBinding,
    pub data_root: PathBuf,
    pub source_policy: Option<SourcePolicy>,
    pub cache_policy: Option<CachePolicy>,
    pub replay_data_policy: Option<ReplayDataPolicy>,
    pub simulation: SimulationConfig,
    pub instrument_economics: Vec<InstrumentEconomicsConfig>,
    pub output_policy: Option<OutputPolicy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveConfigChecksum([u8; 32]);

impl EffectiveConfigChecksum {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRunConfig {
    trading_dates: Box<[TradingDate]>,
    universe: Box<[InstrumentId]>,
    session_kinds: Box<[SessionKind]>,
    strategy: StrategyBinding,
    data_root: PathBuf,
    source_policy: SourcePolicy,
    cache_policy: CachePolicy,
    replay_data_policy: ReplayDataPolicy,
    simulation: SimulationConfig,
    instrument_economics: Box<[InstrumentEconomicsConfig]>,
    output_policy: OutputPolicy,
    canonical_semantics: Box<[u8]>,
    checksum: EffectiveConfigChecksum,
}

impl EffectiveRunConfig {
    pub fn resolve(config: RunConfig) -> Result<Self, ConfigError> {
        if config.config_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedConfigVersion {
                actual: config.config_version,
            });
        }
        if config.data_root.as_os_str().is_empty() {
            return Err(ConfigError::EmptyDataRoot);
        }

        let trading_dates = canonical_non_empty(config.trading_dates, ConfigError::EmptyDates)?;
        let universe = canonical_non_empty(config.universe, ConfigError::EmptyUniverse)?;
        let session_kinds = canonical_non_empty(config.session_kinds, ConfigError::EmptySessions)?;
        if config.strategy.declaration().universe() != &*universe {
            return Err(ConfigError::StrategyUniverseMismatch);
        }
        if config.strategy.declaration().sessions() != &*session_kinds {
            return Err(ConfigError::StrategySessionsMismatch);
        }

        let source_policy = config.source_policy.unwrap_or(SourcePolicy::Strict);
        let cache_policy = config.cache_policy.unwrap_or(CachePolicy::ReuseOrRebuild);
        let replay_data_policy = config
            .replay_data_policy
            .unwrap_or(ReplayDataPolicy::Strict);
        if matches!(source_policy, SourcePolicy::ExplicitDegraded)
            != matches!(replay_data_policy, ReplayDataPolicy::ExplicitDegraded)
        {
            return Err(ConfigError::DegradedPolicyMismatch);
        }
        let output_policy = config.output_policy.unwrap_or(OutputPolicy::CreateNew);

        validate_simulation(&config.simulation)?;
        let instrument_economics = validate_economics(config.instrument_economics, &universe)?;

        let mut effective = Self {
            trading_dates,
            universe,
            session_kinds,
            strategy: config.strategy,
            data_root: config.data_root,
            source_policy,
            cache_policy,
            replay_data_policy,
            simulation: config.simulation,
            instrument_economics,
            output_policy,
            canonical_semantics: Box::new([]),
            checksum: EffectiveConfigChecksum([0; 32]),
        };
        let canonical = effective.encode_semantics()?;
        effective.checksum = EffectiveConfigChecksum(*blake3::hash(&canonical).as_bytes());
        effective.canonical_semantics = canonical.into_boxed_slice();
        Ok(effective)
    }

    #[must_use]
    pub const fn trading_dates(&self) -> &[TradingDate] {
        &self.trading_dates
    }

    #[must_use]
    pub const fn universe(&self) -> &[InstrumentId] {
        &self.universe
    }

    #[must_use]
    pub const fn session_kinds(&self) -> &[SessionKind] {
        &self.session_kinds
    }

    #[must_use]
    pub const fn strategy(&self) -> &StrategyBinding {
        &self.strategy
    }

    #[must_use]
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    #[must_use]
    pub const fn source_policy(&self) -> SourcePolicy {
        self.source_policy
    }

    #[must_use]
    pub const fn cache_policy(&self) -> CachePolicy {
        self.cache_policy
    }

    #[must_use]
    pub const fn replay_data_policy(&self) -> ReplayDataPolicy {
        self.replay_data_policy
    }

    #[must_use]
    pub const fn simulation(&self) -> &SimulationConfig {
        &self.simulation
    }

    #[must_use]
    pub const fn instrument_economics(&self) -> &[InstrumentEconomicsConfig] {
        &self.instrument_economics
    }

    #[must_use]
    pub const fn output_policy(&self) -> OutputPolicy {
        self.output_policy
    }

    #[must_use]
    pub const fn checksum(&self) -> EffectiveConfigChecksum {
        self.checksum
    }

    #[must_use]
    pub const fn canonical_semantics(&self) -> &[u8] {
        &self.canonical_semantics
    }

    fn encode_semantics(&self) -> Result<Vec<u8>, ConfigError> {
        let mut output = Vec::new();
        output.extend_from_slice(b"OSECFG01");
        output.extend_from_slice(&EFFECTIVE_CONFIG_VERSION.to_be_bytes());
        output.extend_from_slice(&CONFIG_SCHEMA_VERSION.to_be_bytes());
        output.extend_from_slice(&SOURCE_POLICY_VERSION.to_be_bytes());
        output.extend_from_slice(&CACHE_POLICY_VERSION.to_be_bytes());
        output.extend_from_slice(&REPLAY_DATA_POLICY_VERSION.to_be_bytes());

        append_len(self.trading_dates.len(), &mut output)?;
        for date in &self.trading_dates {
            output.extend_from_slice(&date.to_canonical_bytes());
        }
        append_len(self.universe.len(), &mut output)?;
        for instrument in &self.universe {
            append_instrument(instrument, &mut output)?;
        }
        append_len(self.session_kinds.len(), &mut output)?;
        for session in &self.session_kinds {
            append_session(*session, &mut output);
        }
        append_strategy_identity(
            self.strategy.identity(),
            self.strategy.params_checksum(),
            &mut output,
        )?;
        output.push(self.source_policy as u8);
        output.push(self.cache_policy as u8);
        output.push(self.replay_data_policy as u8);
        append_simulation(&self.simulation, &mut output)?;
        append_len(self.instrument_economics.len(), &mut output)?;
        for economics in &self.instrument_economics {
            append_economics(economics, &mut output)?;
        }
        output.push(self.output_policy as u8);
        Ok(output)
    }
}

fn canonical_non_empty<T: Ord>(
    values: Vec<T>,
    error: ConfigError,
) -> Result<Box<[T]>, ConfigError> {
    let values = values.into_iter().collect::<BTreeSet<_>>();
    if values.is_empty() {
        Err(error)
    } else {
        Ok(values.into_iter().collect())
    }
}

fn validate_simulation(config: &SimulationConfig) -> Result<(), ConfigError> {
    const MAX_LATENCY_MS: u64 = i64::MAX as u64 / 1_000;
    let latency = config.latency;
    let total_latency_ms = latency
        .market_data_latency_ms
        .checked_add(latency.order_latency_ms);
    if latency.market_data_latency_ms > MAX_LATENCY_MS
        || latency.order_latency_ms > MAX_LATENCY_MS
        || total_latency_ms.is_none_or(|value| value > MAX_LATENCY_MS)
    {
        return Err(ConfigError::InvalidLatency);
    }
    match config.slippage_model {
        SlippageModelConfig::AdverseFixedDelta { delta } if delta < Decimal::ZERO => {
            return Err(ConfigError::NegativeSlippage);
        }
        SlippageModelConfig::AdverseFixedDelta { .. } => {}
    }
    validate_charge(&config.fee_model, ConfigError::InvalidFee)?;
    validate_charge(&config.tax_model, ConfigError::InvalidTax)?;
    if config.initial_cash.currency != Currency::Twd || config.initial_cash.amount <= Decimal::ZERO
    {
        return Err(ConfigError::InvalidInitialCash);
    }
    Ok(())
}

fn validate_charge(config: &ChargeConfig, error: ConfigError) -> Result<(), ConfigError> {
    if config.rate < Decimal::ZERO
        || config.minimum < Decimal::ZERO
        || config.precision > Decimal::SCALE as u8
        || config.provenance.is_empty()
    {
        Err(error)
    } else {
        Ok(())
    }
}

fn validate_economics(
    values: Vec<InstrumentEconomicsConfig>,
    universe: &[InstrumentId],
) -> Result<Box<[InstrumentEconomicsConfig]>, ConfigError> {
    let mut by_instrument = BTreeMap::new();
    for value in values {
        if !universe.contains(&value.instrument) {
            return Err(ConfigError::EconomicsOutsideUniverse(
                value.instrument.clone(),
            ));
        }
        if value.quantity_unit == QuantityUnit::SourceUnit
            || value.units_per_trading_unit == 0
            || value.multiplier <= Decimal::ZERO
            || value.provenance.is_empty()
        {
            return Err(ConfigError::InvalidInstrumentEconomics(
                value.instrument.clone(),
            ));
        }
        let instrument = value.instrument.clone();
        if by_instrument.insert(instrument.clone(), value).is_some() {
            return Err(ConfigError::DuplicateInstrumentEconomics(instrument));
        }
    }
    for instrument in universe {
        if !by_instrument.contains_key(instrument) {
            return Err(ConfigError::MissingInstrumentEconomics(instrument.clone()));
        }
    }
    Ok(by_instrument.into_values().collect())
}

fn append_simulation(config: &SimulationConfig, output: &mut Vec<u8>) -> Result<(), ConfigError> {
    output.push(config.fill_model.evidence as u8);
    output.push(config.fill_model.quantity as u8);
    output.extend_from_slice(&config.latency.market_data_latency_ms.to_be_bytes());
    output.extend_from_slice(&config.latency.order_latency_ms.to_be_bytes());
    output.push(config.quantity_allocation as u8);
    match config.slippage_model {
        SlippageModelConfig::AdverseFixedDelta { delta } => {
            output.push(1);
            append_decimal(delta, output);
        }
    }
    append_charge(&config.fee_model, output)?;
    append_charge(&config.tax_model, output)?;
    output.push(config.initial_cash.currency as u8);
    append_decimal(config.initial_cash.amount, output);
    output.push(config.position_accounting as u8);
    match config.marking_policy {
        MarkingPolicyConfig::LastObservableV1 {
            allow_midpoint_fallback,
        } => {
            output.push(1);
            output.push(u8::from(allow_midpoint_fallback));
        }
    }
    Ok(())
}

fn append_charge(config: &ChargeConfig, output: &mut Vec<u8>) -> Result<(), ConfigError> {
    append_decimal(config.rate, output);
    output.push(config.applicable_sides as u8);
    append_decimal(config.minimum, output);
    output.push(config.precision);
    output.push(config.rounding as u8);
    append_text(&config.provenance, output)
}

fn append_economics(
    config: &InstrumentEconomicsConfig,
    output: &mut Vec<u8>,
) -> Result<(), ConfigError> {
    append_instrument(&config.instrument, output)?;
    output.push(config.quantity_unit.discriminant());
    output.extend_from_slice(&config.units_per_trading_unit.to_be_bytes());
    output.push(config.currency as u8);
    append_decimal(config.multiplier, output);
    append_text(&config.provenance, output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    UnsupportedConfigVersion { actual: u16 },
    EmptyDates,
    EmptyUniverse,
    EmptySessions,
    EmptyDataRoot,
    StrategyUniverseMismatch,
    StrategySessionsMismatch,
    DegradedPolicyMismatch,
    InvalidLatency,
    NegativeSlippage,
    InvalidFee,
    InvalidTax,
    InvalidInitialCash,
    EconomicsOutsideUniverse(InstrumentId),
    DuplicateInstrumentEconomics(InstrumentId),
    MissingInstrumentEconomics(InstrumentId),
    InvalidInstrumentEconomics(InstrumentId),
    CanonicalLengthOverflow,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedConfigVersion { actual } => {
                write!(formatter, "unsupported run config version: {actual}")
            }
            Self::EmptyDates => formatter.write_str("run config requires a trading date"),
            Self::EmptyUniverse => formatter.write_str("run config requires an instrument"),
            Self::EmptySessions => formatter.write_str("run config requires a session kind"),
            Self::EmptyDataRoot => formatter.write_str("data_root must not be empty"),
            Self::StrategyUniverseMismatch => {
                formatter.write_str("strategy universe differs from run config universe")
            }
            Self::StrategySessionsMismatch => {
                formatter.write_str("strategy sessions differ from run config sessions")
            }
            Self::DegradedPolicyMismatch => formatter.write_str(
                "source and replay degraded policies must be enabled or disabled together",
            ),
            Self::InvalidLatency => formatter.write_str(
                "latency must fit in the replay time range when represented as milliseconds",
            ),
            Self::NegativeSlippage => formatter.write_str("slippage delta must not be negative"),
            Self::InvalidFee => formatter.write_str("fee configuration is invalid"),
            Self::InvalidTax => formatter.write_str("tax configuration is invalid"),
            Self::InvalidInitialCash => formatter.write_str("initial cash must be positive TWD"),
            Self::EconomicsOutsideUniverse(instrument) => {
                write!(
                    formatter,
                    "economics instrument is outside universe: {instrument:?}"
                )
            }
            Self::DuplicateInstrumentEconomics(instrument) => {
                write!(formatter, "duplicate instrument economics: {instrument:?}")
            }
            Self::MissingInstrumentEconomics(instrument) => {
                write!(formatter, "missing instrument economics: {instrument:?}")
            }
            Self::InvalidInstrumentEconomics(instrument) => {
                write!(formatter, "invalid instrument economics: {instrument:?}")
            }
            Self::CanonicalLengthOverflow => {
                formatter.write_str("canonical variable-length field exceeds u32")
            }
        }
    }
}

impl Error for ConfigError {}
