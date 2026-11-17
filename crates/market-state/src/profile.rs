use std::{error::Error, fmt};

use market_types::{DomainEvent, EventKind, MarketId, QuantityUnit, SourceFormatId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CumulativeVolumePolicy {
    Unconstrained,
    NonDecreasingWithinSegment { unit: QuantityUnit },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFormatRule {
    source_format: SourceFormatId,
    accepted_event_kinds: Box<[EventKind]>,
}

impl SourceFormatRule {
    pub fn new(
        source_format: SourceFormatId,
        mut accepted_event_kinds: Vec<EventKind>,
    ) -> Result<Self, ProfileError> {
        accepted_event_kinds.sort_unstable();
        accepted_event_kinds.dedup();
        if accepted_event_kinds.is_empty() {
            return Err(ProfileError::EmptyEventKindSet);
        }
        Ok(Self {
            source_format,
            accepted_event_kinds: accepted_event_kinds.into_boxed_slice(),
        })
    }

    #[must_use]
    pub const fn source_format(&self) -> &SourceFormatId {
        &self.source_format
    }

    #[must_use]
    pub const fn accepted_event_kinds(&self) -> &[EventKind] {
        &self.accepted_event_kinds
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketStateProfile {
    market: MarketId,
    source_rules: Box<[SourceFormatRule]>,
    cumulative_volume_policy: CumulativeVolumePolicy,
    segment_boundary_policy_version: u16,
}

impl MarketStateProfile {
    pub fn new(
        market: MarketId,
        mut source_rules: Vec<SourceFormatRule>,
        cumulative_volume_policy: CumulativeVolumePolicy,
        segment_boundary_policy_version: u16,
    ) -> Result<Self, ProfileError> {
        if source_rules.is_empty() {
            return Err(ProfileError::EmptySourceRules);
        }
        source_rules.sort_by_key(|left| left.source_format);
        if source_rules
            .windows(2)
            .any(|pair| pair[0].source_format == pair[1].source_format)
        {
            return Err(ProfileError::DuplicateSourceFormat);
        }
        if segment_boundary_policy_version == 0 {
            return Err(ProfileError::ZeroBoundaryPolicyVersion);
        }
        Ok(Self {
            market,
            source_rules: source_rules.into_boxed_slice(),
            cumulative_volume_policy,
            segment_boundary_policy_version,
        })
    }

    /// Accepts every event kind from each of `source_formats` (segment boundary policy 1).
    pub fn accepting_all(
        market: MarketId,
        source_formats: &[SourceFormatId],
        cumulative_volume_policy: CumulativeVolumePolicy,
    ) -> Result<Self, ProfileError> {
        let rules = source_formats
            .iter()
            .map(|&source_format| SourceFormatRule::new(source_format, EventKind::ALL.to_vec()))
            .collect::<Result<_, _>>()?;
        Self::new(market, rules, cumulative_volume_policy, 1)
    }

    #[must_use]
    pub const fn market(&self) -> MarketId {
        self.market
    }

    #[must_use]
    pub const fn cumulative_volume_policy(&self) -> CumulativeVolumePolicy {
        self.cumulative_volume_policy
    }

    #[must_use]
    pub const fn segment_boundary_policy_version(&self) -> u16 {
        self.segment_boundary_policy_version
    }

    pub(crate) fn validate_event(&self, event: &DomainEvent) -> Result<(), ProfileError> {
        if event.instrument().market() != self.market {
            return Err(ProfileError::MarketMismatch);
        }
        let Some(rule) = self
            .source_rules
            .iter()
            .find(|rule| rule.source_format == *event.source_format())
        else {
            return Err(ProfileError::UnsupportedSourceFormat);
        };
        if !rule.accepted_event_kinds.contains(&event.payload().kind()) {
            return Err(ProfileError::UnsupportedEventKind);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    EmptySourceRules,
    EmptyEventKindSet,
    DuplicateSourceFormat,
    ZeroBoundaryPolicyVersion,
    MarketMismatch,
    UnsupportedSourceFormat,
    UnsupportedEventKind,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptySourceRules => "market-state profile must accept at least one source format",
            Self::EmptyEventKindSet => "source-format rule must accept at least one event kind",
            Self::DuplicateSourceFormat => {
                "market-state profile contains a duplicate source format"
            }
            Self::ZeroBoundaryPolicyVersion => {
                "segment boundary policy version must be greater than zero"
            }
            Self::MarketMismatch => "event market does not match market-state profile",
            Self::UnsupportedSourceFormat => "event source format is not supported by profile",
            Self::UnsupportedEventKind => "event kind is not supported for its source format",
        };
        formatter.write_str(message)
    }
}

impl Error for ProfileError {}
