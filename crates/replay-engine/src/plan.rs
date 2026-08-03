use std::{error::Error, fmt};

use market_types::{InstrumentId, TradingDate};

pub const REPLAY_PLAN_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableStreamDescriptorId([u8; 32]);

impl StableStreamDescriptorId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplayPlanIdentity([u8; 32]);

impl ReplayPlanIdentity {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayStreamBinding {
    descriptor_id: StableStreamDescriptorId,
    instrument: InstrumentId,
    trading_date: TradingDate,
    source_revision_identity: [u8; 32],
    cache_identity: [u8; 32],
}

impl ReplayStreamBinding {
    #[must_use]
    pub const fn new(
        descriptor_id: StableStreamDescriptorId,
        instrument: InstrumentId,
        trading_date: TradingDate,
        source_revision_identity: [u8; 32],
        cache_identity: [u8; 32],
    ) -> Self {
        Self {
            descriptor_id,
            instrument,
            trading_date,
            source_revision_identity,
            cache_identity,
        }
    }

    #[must_use]
    pub const fn descriptor_id(&self) -> StableStreamDescriptorId {
        self.descriptor_id
    }

    #[must_use]
    pub const fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn trading_date(&self) -> TradingDate {
        self.trading_date
    }

    #[must_use]
    pub const fn source_revision_identity(&self) -> &[u8; 32] {
        &self.source_revision_identity
    }

    #[must_use]
    pub const fn cache_identity(&self) -> &[u8; 32] {
        &self.cache_identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayPlan {
    upstream_plan_identity: [u8; 32],
    bindings: Box<[ReplayStreamBinding]>,
    identity: ReplayPlanIdentity,
}

impl ReplayPlan {
    pub fn new(
        upstream_plan_identity: [u8; 32],
        bindings: Vec<ReplayStreamBinding>,
    ) -> Result<Self, ReplayPlanError> {
        let [binding] = bindings
            .try_into()
            .map_err(|bindings: Vec<_>| match bindings.len() {
                0 => ReplayPlanError::EmptyBindings,
                count => ReplayPlanError::SingleBindingRequired(count),
            })?;
        Self::build_single(upstream_plan_identity, binding)
    }

    pub fn new_multi(
        upstream_plan_identity: [u8; 32],
        bindings: Vec<ReplayStreamBinding>,
    ) -> Result<Self, ReplayPlanError> {
        if bindings.is_empty() {
            return Err(ReplayPlanError::EmptyBindings);
        }
        Self::build_multi(upstream_plan_identity, bindings)
    }

    fn build_single(
        upstream_plan_identity: [u8; 32],
        binding: ReplayStreamBinding,
    ) -> Result<Self, ReplayPlanError> {
        let mut canonical = Vec::new();
        canonical.extend_from_slice(b"OSRP");
        canonical.extend_from_slice(&REPLAY_PLAN_VERSION.to_be_bytes());
        canonical.extend_from_slice(&upstream_plan_identity);
        append_binding(&binding, &mut canonical)?;
        let identity = ReplayPlanIdentity(*blake3::hash(&canonical).as_bytes());
        Ok(Self {
            upstream_plan_identity,
            bindings: vec![binding].into_boxed_slice(),
            identity,
        })
    }

    fn build_multi(
        upstream_plan_identity: [u8; 32],
        mut bindings: Vec<ReplayStreamBinding>,
    ) -> Result<Self, ReplayPlanError> {
        bindings.sort_by(|left, right| {
            left.instrument
                .cmp(&right.instrument)
                .then_with(|| left.trading_date.cmp(&right.trading_date))
                .then_with(|| left.descriptor_id.cmp(&right.descriptor_id))
                .then_with(|| left.cache_identity.cmp(&right.cache_identity))
        });
        if bindings.windows(2).any(|pair| {
            pair[0].instrument == pair[1].instrument && pair[0].trading_date == pair[1].trading_date
        }) {
            return Err(ReplayPlanError::DuplicateLogicalStream);
        }
        let mut canonical = Vec::new();
        canonical.extend_from_slice(b"OSRP");
        canonical.extend_from_slice(&REPLAY_PLAN_VERSION.to_be_bytes());
        canonical.extend_from_slice(&upstream_plan_identity);
        append_len(bindings.len(), &mut canonical)?;
        for binding in &bindings {
            append_binding(binding, &mut canonical)?;
        }
        let identity = ReplayPlanIdentity(*blake3::hash(&canonical).as_bytes());
        Ok(Self {
            upstream_plan_identity,
            bindings: bindings.into_boxed_slice(),
            identity,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> ReplayPlanIdentity {
        self.identity
    }

    #[must_use]
    pub const fn upstream_plan_identity(&self) -> &[u8; 32] {
        &self.upstream_plan_identity
    }

    #[must_use]
    pub const fn binding(&self) -> &ReplayStreamBinding {
        &self.bindings[0]
    }

    #[must_use]
    pub const fn bindings(&self) -> &[ReplayStreamBinding] {
        &self.bindings
    }
}

fn append_binding(
    binding: &ReplayStreamBinding,
    output: &mut Vec<u8>,
) -> Result<(), ReplayPlanError> {
    output.extend_from_slice(binding.descriptor_id.as_bytes());
    output.push(binding.instrument.market().discriminant());
    append_bytes(binding.instrument.symbol().as_bytes(), output)?;
    output.extend_from_slice(&binding.trading_date.to_canonical_bytes());
    output.extend_from_slice(&binding.source_revision_identity);
    output.extend_from_slice(&binding.cache_identity);
    Ok(())
}

fn append_len(length: usize, output: &mut Vec<u8>) -> Result<(), ReplayPlanError> {
    output.extend_from_slice(
        &u32::try_from(length)
            .map_err(|_| ReplayPlanError::CanonicalLength)?
            .to_be_bytes(),
    );
    Ok(())
}

fn append_bytes(value: &[u8], output: &mut Vec<u8>) -> Result<(), ReplayPlanError> {
    let length = u32::try_from(value.len()).map_err(|_| ReplayPlanError::CanonicalLength)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayPlanError {
    EmptyBindings,
    SingleBindingRequired(usize),
    DuplicateLogicalStream,
    CanonicalLength,
}

impl fmt::Display for ReplayPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBindings => formatter.write_str("replay plan requires a stream binding"),
            Self::SingleBindingRequired(count) => {
                write!(formatter, "replay plan requires one binding, got {count}")
            }
            Self::DuplicateLogicalStream => {
                formatter.write_str("replay plan contains duplicate instrument/date streams")
            }
            Self::CanonicalLength => formatter.write_str("replay plan canonical field is too long"),
        }
    }
}

impl Error for ReplayPlanError {}

#[cfg(test)]
mod tests {
    use market_types::{MarketId, Symbol};

    use super::*;

    fn binding() -> ReplayStreamBinding {
        ReplayStreamBinding::new(
            StableStreamDescriptorId::from_bytes([1; 32]),
            InstrumentId::new(MarketId::Twse, Symbol::new("2330").unwrap()),
            "2026-07-27".parse().unwrap(),
            [2; 32],
            [3; 32],
        )
    }

    #[test]
    fn replay_plan_identity_is_deterministic_and_lineage_bound() {
        let first = ReplayPlan::new([4; 32], vec![binding()]).unwrap();
        let second = ReplayPlan::new([4; 32], vec![binding()]).unwrap();
        assert_eq!(first.identity(), second.identity());

        let mut changed = binding();
        changed.cache_identity = [9; 32];
        assert_ne!(
            first.identity(),
            ReplayPlan::new([4; 32], vec![changed]).unwrap().identity()
        );
    }

    #[test]
    fn single_stream_plan_requires_exactly_one_selected_stream() {
        assert_eq!(
            ReplayPlan::new([0; 32], Vec::new()).unwrap_err(),
            ReplayPlanError::EmptyBindings
        );
        assert_eq!(
            ReplayPlan::new([0; 32], vec![binding(), binding()]).unwrap_err(),
            ReplayPlanError::SingleBindingRequired(2)
        );
    }
}
