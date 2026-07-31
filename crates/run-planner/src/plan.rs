use std::{collections::BTreeSet, error::Error, fmt};

use market_types::{InstrumentId, TradingDate};

use crate::{
    CACHE_POLICY_VERSION, CONFIG_SCHEMA_VERSION, CacheIdentity, CacheState, CorruptReason,
    EFFECTIVE_CONFIG_VERSION, EffectiveConfigChecksum, EffectiveRunConfig, IncompleteReason,
    REPLAY_DATA_POLICY_VERSION, SOURCE_PARTITION_KEY_VERSION, SOURCE_POLICY_VERSION,
    SourcePartitionIdentity, SourcePartitionKey, SourcePolicy, SourceRevisionIdentity, SourceState,
    canonical::append_len,
};

pub const EXECUTION_PLAN_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningVersionSet {
    pub config_schema: u16,
    pub effective_config: u16,
    pub source_policy: u16,
    pub cache_policy: u16,
    pub replay_data_policy: u16,
    pub source_partition_key: u16,
    pub execution_plan: u16,
}

impl PlanningVersionSet {
    pub const CURRENT: Self = Self {
        config_schema: CONFIG_SCHEMA_VERSION,
        effective_config: EFFECTIVE_CONFIG_VERSION,
        source_policy: SOURCE_POLICY_VERSION,
        cache_policy: CACHE_POLICY_VERSION,
        replay_data_policy: REPLAY_DATA_POLICY_VERSION,
        source_partition_key: SOURCE_PARTITION_KEY_VERSION,
        execution_plan: EXECUTION_PLAN_VERSION,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompletionPolicy {
    Strict = 1,
    ExplicitDegraded = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NetworkRequirement {
    NotRequired = 1,
    Required = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAction {
    ReuseCompleteSource { revision: SourceRevisionIdentity },
    DownloadMissingSource,
    ResumeOrRestartBuilding,
    RejectIncomplete { reason: IncompleteReason },
    RejectCorrupt { reason: CorruptReason },
    CoverageUnavailable,
}

impl SourceAction {
    #[must_use]
    pub const fn requires_network(self) -> bool {
        matches!(
            self,
            Self::DownloadMissingSource | Self::ResumeOrRestartBuilding
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VerificationAction {
    VerifyCompleteSource = 1,
    AwaitSourcePreparation = 2,
    RejectIncomplete = 3,
    RejectCorrupt = 4,
    CoverageUnavailable = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheAction {
    ReuseValidCache { identity: CacheIdentity },
    RebuildCacheFromCompleteSource,
    AwaitCompleteSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPartition {
    key: SourcePartitionKey,
    source_state: SourceState,
    source_action: SourceAction,
    verification_action: VerificationAction,
    cache_action: CacheAction,
}

impl PlannedPartition {
    #[must_use]
    pub const fn classify(
        key: SourcePartitionKey,
        source_state: SourceState,
        cache_state: CacheState,
    ) -> Self {
        let (source_action, verification_action) = match source_state {
            SourceState::Missing => (
                SourceAction::DownloadMissingSource,
                VerificationAction::AwaitSourcePreparation,
            ),
            SourceState::Building => (
                SourceAction::ResumeOrRestartBuilding,
                VerificationAction::AwaitSourcePreparation,
            ),
            SourceState::Complete { revision } => (
                SourceAction::ReuseCompleteSource { revision },
                VerificationAction::VerifyCompleteSource,
            ),
            SourceState::Incomplete { reason } => (
                SourceAction::RejectIncomplete { reason },
                VerificationAction::RejectIncomplete,
            ),
            SourceState::Corrupt { reason } => (
                SourceAction::RejectCorrupt { reason },
                VerificationAction::RejectCorrupt,
            ),
        };
        let cache_action = match (source_state, cache_state) {
            (SourceState::Complete { .. }, CacheState::Valid { identity }) => {
                CacheAction::ReuseValidCache { identity }
            }
            (SourceState::Complete { .. }, _) => CacheAction::RebuildCacheFromCompleteSource,
            _ => CacheAction::AwaitCompleteSource,
        };
        Self {
            key,
            source_state,
            source_action,
            verification_action,
            cache_action,
        }
    }

    #[must_use]
    pub const fn coverage_unavailable(key: SourcePartitionKey) -> Self {
        Self {
            key,
            source_state: SourceState::Missing,
            source_action: SourceAction::CoverageUnavailable,
            verification_action: VerificationAction::CoverageUnavailable,
            cache_action: CacheAction::AwaitCompleteSource,
        }
    }

    #[must_use]
    pub const fn key(&self) -> &SourcePartitionKey {
        &self.key
    }

    #[must_use]
    pub const fn source_state(&self) -> SourceState {
        self.source_state
    }

    #[must_use]
    pub const fn source_action(&self) -> SourceAction {
        self.source_action
    }

    #[must_use]
    pub const fn verification_action(&self) -> VerificationAction {
        self.verification_action
    }

    #[must_use]
    pub const fn cache_action(&self) -> CacheAction {
        self.cache_action
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DegradedScope {
    partition: SourcePartitionIdentity,
}

impl DegradedScope {
    #[must_use]
    pub const fn new(partition: SourcePartitionIdentity) -> Self {
        Self { partition }
    }

    #[must_use]
    pub const fn partition(self) -> SourcePartitionIdentity {
        self.partition
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionPlanIdentity([u8; 32]);

impl ExecutionPlanIdentity {
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
pub struct ExecutionPlan {
    config: EffectiveRunConfig,
    partitions: Box<[PlannedPartition]>,
    completion_policy: CompletionPolicy,
    network_requirement: NetworkRequirement,
    degraded_scopes: Box<[DegradedScope]>,
    canonical: Box<[u8]>,
    identity: ExecutionPlanIdentity,
}

impl ExecutionPlan {
    pub fn new(
        config: EffectiveRunConfig,
        partitions: Vec<PlannedPartition>,
        degraded_scopes: Vec<DegradedScope>,
    ) -> Result<Self, PlanError> {
        if partitions.is_empty() {
            return Err(PlanError::EmptyPartitions);
        }

        let mut partitions = partitions;
        partitions.sort_by(|left, right| left.key.cmp(&right.key));
        let mut identities = BTreeSet::new();
        let mut logical_requests = BTreeSet::new();
        for partition in &partitions {
            validate_partition(&config, partition)?;
            if !identities.insert(partition.key.identity()) {
                return Err(PlanError::DuplicatePartition(partition.key.identity()));
            }
            let logical_request = (
                partition.key.instrument().clone(),
                partition.key.trading_date(),
            );
            if !logical_requests.insert(logical_request.clone()) {
                return Err(PlanError::DuplicateLogicalPartition {
                    instrument: logical_request.0,
                    trading_date: logical_request.1,
                });
            }
        }
        for instrument in config.universe() {
            for trading_date in config.trading_dates() {
                if !logical_requests.contains(&(instrument.clone(), *trading_date)) {
                    return Err(PlanError::MissingRequestedPartition {
                        instrument: instrument.clone(),
                        trading_date: *trading_date,
                    });
                }
            }
        }

        let degraded_scopes = degraded_scopes.into_iter().collect::<BTreeSet<_>>();
        let completion_policy = match config.source_policy() {
            SourcePolicy::Strict => {
                if !degraded_scopes.is_empty() {
                    return Err(PlanError::DegradedScopeWithStrictPolicy);
                }
                CompletionPolicy::Strict
            }
            SourcePolicy::ExplicitDegraded => CompletionPolicy::ExplicitDegraded,
        };
        for scope in &degraded_scopes {
            let Some(partition) = partitions
                .iter()
                .find(|partition| partition.key.identity() == scope.partition)
            else {
                return Err(PlanError::UnknownDegradedScope(scope.partition));
            };
            if !matches!(partition.source_state, SourceState::Incomplete { .. }) {
                return Err(PlanError::InvalidDegradedScope(scope.partition));
            }
        }

        let network_requirement = if partitions
            .iter()
            .any(|partition| partition.source_action.requires_network())
        {
            NetworkRequirement::Required
        } else {
            NetworkRequirement::NotRequired
        };

        let degraded_scopes = degraded_scopes.into_iter().collect::<Box<[_]>>();
        let mut plan = Self {
            config,
            partitions: partitions.into_boxed_slice(),
            completion_policy,
            network_requirement,
            degraded_scopes,
            canonical: Box::new([]),
            identity: ExecutionPlanIdentity([0; 32]),
        };
        let canonical = plan.encode()?;
        plan.identity = ExecutionPlanIdentity(*blake3::hash(&canonical).as_bytes());
        plan.canonical = canonical.into_boxed_slice();
        Ok(plan)
    }

    #[must_use]
    pub const fn config(&self) -> &EffectiveRunConfig {
        &self.config
    }

    #[must_use]
    pub const fn config_checksum(&self) -> EffectiveConfigChecksum {
        self.config.checksum()
    }

    #[must_use]
    pub const fn partitions(&self) -> &[PlannedPartition] {
        &self.partitions
    }

    #[must_use]
    pub const fn completion_policy(&self) -> CompletionPolicy {
        self.completion_policy
    }

    #[must_use]
    pub const fn network_requirement(&self) -> NetworkRequirement {
        self.network_requirement
    }

    #[must_use]
    pub const fn degraded_scopes(&self) -> &[DegradedScope] {
        &self.degraded_scopes
    }

    #[must_use]
    pub const fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    #[must_use]
    pub const fn identity(&self) -> ExecutionPlanIdentity {
        self.identity
    }

    #[must_use]
    pub const fn version_set(&self) -> PlanningVersionSet {
        PlanningVersionSet::CURRENT
    }

    fn encode(&self) -> Result<Vec<u8>, PlanError> {
        let mut output = Vec::new();
        output.extend_from_slice(b"OSPLAN01");
        output.extend_from_slice(&EXECUTION_PLAN_VERSION.to_be_bytes());
        let versions = PlanningVersionSet::CURRENT;
        output.extend_from_slice(&versions.config_schema.to_be_bytes());
        output.extend_from_slice(&versions.effective_config.to_be_bytes());
        output.extend_from_slice(&versions.source_policy.to_be_bytes());
        output.extend_from_slice(&versions.cache_policy.to_be_bytes());
        output.extend_from_slice(&versions.replay_data_policy.to_be_bytes());
        output.extend_from_slice(&versions.source_partition_key.to_be_bytes());
        output.extend_from_slice(&versions.execution_plan.to_be_bytes());
        output.extend_from_slice(self.config.checksum().as_bytes());
        append_len(self.partitions.len(), &mut output)
            .map_err(|_| PlanError::CanonicalLengthOverflow)?;
        for partition in &self.partitions {
            append_len(partition.key.canonical_bytes().len(), &mut output)
                .map_err(|_| PlanError::CanonicalLengthOverflow)?;
            output.extend_from_slice(partition.key.canonical_bytes());
            append_source_state(partition.source_state, &mut output);
            append_source_action(partition.source_action, &mut output);
            output.push(partition.verification_action as u8);
            append_cache_action(partition.cache_action, &mut output);
        }
        output.push(self.completion_policy as u8);
        output.push(self.network_requirement as u8);
        append_len(self.degraded_scopes.len(), &mut output)
            .map_err(|_| PlanError::CanonicalLengthOverflow)?;
        for scope in &self.degraded_scopes {
            output.extend_from_slice(scope.partition.as_bytes());
        }
        Ok(output)
    }
}

fn validate_partition(
    config: &EffectiveRunConfig,
    partition: &PlannedPartition,
) -> Result<(), PlanError> {
    if !config.universe().contains(partition.key.instrument())
        || !config
            .trading_dates()
            .contains(&partition.key.trading_date())
        || config.session_kinds() != partition.key.session_kinds()
    {
        return Err(PlanError::PartitionOutsideConfig(partition.key.identity()));
    }
    Ok(())
}

fn append_source_state(state: SourceState, output: &mut Vec<u8>) {
    output.push(state.kind() as u8);
    match state {
        SourceState::Complete { revision } => output.extend_from_slice(revision.as_bytes()),
        SourceState::Incomplete { reason } => output.push(reason as u8),
        SourceState::Corrupt { reason } => output.push(reason as u8),
        SourceState::Missing | SourceState::Building => {}
    }
}

fn append_source_action(action: SourceAction, output: &mut Vec<u8>) {
    match action {
        SourceAction::ReuseCompleteSource { revision } => {
            output.push(1);
            output.extend_from_slice(revision.as_bytes());
        }
        SourceAction::DownloadMissingSource => output.push(2),
        SourceAction::ResumeOrRestartBuilding => output.push(3),
        SourceAction::RejectIncomplete { reason } => {
            output.push(4);
            output.push(reason as u8);
        }
        SourceAction::RejectCorrupt { reason } => {
            output.push(5);
            output.push(reason as u8);
        }
        SourceAction::CoverageUnavailable => output.push(6),
    }
}

fn append_cache_action(action: CacheAction, output: &mut Vec<u8>) {
    match action {
        CacheAction::ReuseValidCache { identity } => {
            output.push(1);
            output.extend_from_slice(identity.as_bytes());
        }
        CacheAction::RebuildCacheFromCompleteSource => output.push(2),
        CacheAction::AwaitCompleteSource => output.push(3),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    EmptyPartitions,
    DuplicatePartition(SourcePartitionIdentity),
    DuplicateLogicalPartition {
        instrument: InstrumentId,
        trading_date: TradingDate,
    },
    MissingRequestedPartition {
        instrument: InstrumentId,
        trading_date: TradingDate,
    },
    PartitionOutsideConfig(SourcePartitionIdentity),
    DegradedScopeWithStrictPolicy,
    UnknownDegradedScope(SourcePartitionIdentity),
    InvalidDegradedScope(SourcePartitionIdentity),
    CanonicalLengthOverflow,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPartitions => formatter.write_str("execution plan requires a partition"),
            Self::DuplicatePartition(identity) => {
                write!(formatter, "duplicate source partition: {identity:?}")
            }
            Self::DuplicateLogicalPartition {
                instrument,
                trading_date,
            } => write!(
                formatter,
                "duplicate logical source partition: {instrument:?} on {trading_date}"
            ),
            Self::MissingRequestedPartition {
                instrument,
                trading_date,
            } => write!(
                formatter,
                "missing requested source partition: {instrument:?} on {trading_date}"
            ),
            Self::PartitionOutsideConfig(identity) => {
                write!(
                    formatter,
                    "partition is outside effective config: {identity:?}"
                )
            }
            Self::DegradedScopeWithStrictPolicy => {
                formatter.write_str("strict plan cannot declare degraded scopes")
            }
            Self::UnknownDegradedScope(identity) => {
                write!(formatter, "degraded scope is not planned: {identity:?}")
            }
            Self::InvalidDegradedScope(identity) => write!(
                formatter,
                "degraded scope is not an incomplete source partition: {identity:?}"
            ),
            Self::CanonicalLengthOverflow => {
                formatter.write_str("execution plan canonical field exceeds u32 length")
            }
        }
    }
}

impl Error for PlanError {}
