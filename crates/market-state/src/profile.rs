use std::{error::Error, fmt};

use market_types::{
    DomainEvent, EventKind, EventPayload, MarketAnnotations, MarketId, QuantityUnit, SourceFormatId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CumulativeVolumePolicy {
    Unconstrained,
    NonDecreasingWithinSegment { unit: QuantityUnit },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationPolicy {
    NoneOnly,
    TwseQuote,
    TpexQuote,
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
    annotation_policy: AnnotationPolicy,
    segment_boundary_policy_version: u16,
}

impl MarketStateProfile {
    pub fn new(
        market: MarketId,
        mut source_rules: Vec<SourceFormatRule>,
        cumulative_volume_policy: CumulativeVolumePolicy,
        annotation_policy: AnnotationPolicy,
        segment_boundary_policy_version: u16,
    ) -> Result<Self, ProfileError> {
        if source_rules.is_empty() {
            return Err(ProfileError::EmptySourceRules);
        }
        source_rules.sort_by(|left, right| left.source_format.cmp(&right.source_format));
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
            annotation_policy,
            segment_boundary_policy_version,
        })
    }

    pub fn twse_regular() -> Self {
        Self::new(
            MarketId::Twse,
            vec![
                SourceFormatRule::new(
                    SourceFormatId::new("STOCK_REALTIME")
                        .expect("TWSE source format constant is non-empty"),
                    vec![
                        EventKind::QuoteSnapshot,
                        EventKind::TradeBatch,
                        EventKind::IndicativeOpeningAuction,
                        EventKind::IndicativeClosingAuction,
                    ],
                )
                .expect("TWSE realtime profile has accepted event kinds"),
                SourceFormatRule::new(
                    SourceFormatId::new("STOCK_SNAPSHOT")
                        .expect("TWSE source format constant is non-empty"),
                    vec![
                        EventKind::QuoteSnapshot,
                        EventKind::IndicativeOpeningAuction,
                        EventKind::IndicativeClosingAuction,
                    ],
                )
                .expect("TWSE snapshot profile has accepted event kinds"),
            ],
            CumulativeVolumePolicy::NonDecreasingWithinSegment {
                unit: QuantityUnit::TradingUnit,
            },
            AnnotationPolicy::TwseQuote,
            1,
        )
        .expect("built-in TWSE market-state profile is valid")
    }

    pub fn taifex_futures() -> Self {
        Self::new(
            MarketId::Taifex,
            vec![
                SourceFormatRule::new(
                    SourceFormatId::new("I020").expect("TAIFEX format is non-empty"),
                    vec![EventKind::TradeBatch],
                )
                .expect("TAIFEX trade profile has accepted event kinds"),
                SourceFormatRule::new(
                    SourceFormatId::new("I022").expect("TAIFEX format is non-empty"),
                    vec![EventKind::IndicativeOpeningAuction],
                )
                .expect("TAIFEX opening profile has accepted event kinds"),
                SourceFormatRule::new(
                    SourceFormatId::new("I080").expect("TAIFEX format is non-empty"),
                    vec![EventKind::BookSnapshot],
                )
                .expect("TAIFEX book profile has accepted event kinds"),
                SourceFormatRule::new(
                    SourceFormatId::new("I082").expect("TAIFEX format is non-empty"),
                    vec![EventKind::BookSnapshot],
                )
                .expect("TAIFEX reference book profile has accepted event kinds"),
            ],
            CumulativeVolumePolicy::Unconstrained,
            AnnotationPolicy::NoneOnly,
            1,
        )
        .expect("built-in TAIFEX market-state profile is valid")
    }

    pub fn tpex_regular() -> Self {
        Self::new(
            MarketId::Tpex,
            vec![
                SourceFormatRule::new(
                    SourceFormatId::new("STOCK_REALTIME")
                        .expect("TPEx source format constant is non-empty"),
                    vec![
                        EventKind::QuoteSnapshot,
                        EventKind::TradeBatch,
                        EventKind::IndicativeOpeningAuction,
                        EventKind::IndicativeClosingAuction,
                    ],
                )
                .expect("TPEx realtime profile has accepted event kinds"),
                SourceFormatRule::new(
                    SourceFormatId::new("STOCK_SNAPSHOT")
                        .expect("TPEx source format constant is non-empty"),
                    vec![
                        EventKind::QuoteSnapshot,
                        EventKind::IndicativeOpeningAuction,
                        EventKind::IndicativeClosingAuction,
                    ],
                )
                .expect("TPEx snapshot profile has accepted event kinds"),
            ],
            CumulativeVolumePolicy::NonDecreasingWithinSegment {
                unit: QuantityUnit::TradingUnit,
            },
            AnnotationPolicy::TpexQuote,
            1,
        )
        .expect("built-in TPEx market-state profile is valid")
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

        let annotations = match event.payload() {
            EventPayload::QuoteSnapshot(snapshot) => snapshot.annotations(),
            EventPayload::BookSnapshot(snapshot) => snapshot.annotations(),
            EventPayload::TradeBatch(batch) => batch.annotations(),
            EventPayload::IndicativeOpeningAuction(auction)
            | EventPayload::IndicativeClosingAuction(auction) => auction.annotations(),
        };
        let compatible = matches!(
            (self.annotation_policy, annotations),
            (AnnotationPolicy::NoneOnly, MarketAnnotations::None)
                | (AnnotationPolicy::TwseQuote, MarketAnnotations::TwseQuote(_))
                | (AnnotationPolicy::TpexQuote, MarketAnnotations::TpexQuote(_))
        );
        if compatible {
            Ok(())
        } else {
            Err(ProfileError::IncompatibleAnnotations)
        }
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
    IncompatibleAnnotations,
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
            Self::IncompatibleAnnotations => {
                "event annotations are incompatible with market-state profile"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for ProfileError {}
