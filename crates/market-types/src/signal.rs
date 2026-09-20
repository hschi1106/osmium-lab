use crate::{CanonicalEncodingError, CanonicalValue};

/// The semantic purpose of one call-auction observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AuctionPurpose {
    Opening = 1,
    Closing = 2,
    Periodic = 3,
    VolatilityInterruption = 4,
}

impl AuctionPurpose {
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }

    pub(crate) const fn from_discriminant(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Opening),
            2 => Some(Self::Closing),
            3 => Some(Self::Periodic),
            4 => Some(Self::VolatilityInterruption),
            _ => None,
        }
    }
}

/// Direction observed for a volatility interruption trigger or trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VolatilityDirection {
    Down = 1,
    Up = 2,
}

impl VolatilityDirection {
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }

    pub(crate) const fn from_discriminant(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Down),
            2 => Some(Self::Up),
            _ => None,
        }
    }
}

/// Knowledge state for one field of an auction observation.
///
/// `NoObservation` means this record did not update the field and a reducer
/// may carry a same-round value forward. `Unknown` means the source explicitly
/// invalidated the previous value; it must not be treated as `Known(false)` or
/// another default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuctionEvidence<T> {
    NoObservation,
    Known(T),
    Unknown,
}

impl<T> AuctionEvidence<T> {
    #[must_use]
    pub const fn known(value: T) -> Self {
        Self::Known(value)
    }

    #[must_use]
    pub const fn no_observation() -> Self {
        Self::NoObservation
    }

    #[must_use]
    pub const fn unknown() -> Self {
        Self::Unknown
    }
}

impl<T: Copy> AuctionEvidence<T> {
    #[must_use]
    pub const fn known_value(self) -> Option<T> {
        match self {
            Self::Known(value) => Some(value),
            Self::NoObservation | Self::Unknown => None,
        }
    }
}

/// Provider-neutral facts about one auction round.
///
/// Every field is partial knowledge. A provider may reassert one field on a
/// subsequent record without creating another auction round, while omitted
/// fields remain available for the reducer to merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuctionObservation {
    purpose: AuctionEvidence<AuctionPurpose>,
    delayed: AuctionEvidence<bool>,
    disposal: AuctionEvidence<bool>,
    direction: AuctionEvidence<VolatilityDirection>,
}

impl AuctionObservation {
    pub const fn from_parts(
        purpose: AuctionEvidence<AuctionPurpose>,
        delayed: AuctionEvidence<bool>,
        disposal: AuctionEvidence<bool>,
        direction: AuctionEvidence<VolatilityDirection>,
    ) -> Option<Self> {
        if matches!(direction, AuctionEvidence::Known(_))
            && !matches!(
                purpose,
                AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
            )
        {
            return None;
        }
        Some(Self {
            purpose,
            delayed,
            disposal,
            direction,
        })
    }

    #[must_use]
    pub const fn new(purpose: AuctionPurpose, delayed: bool, disposal: bool) -> Self {
        Self {
            purpose: AuctionEvidence::Known(purpose),
            delayed: AuctionEvidence::Known(delayed),
            disposal: AuctionEvidence::Known(disposal),
            direction: AuctionEvidence::NoObservation,
        }
    }

    #[must_use]
    pub const fn volatility_interruption(
        direction: VolatilityDirection,
        delayed: bool,
        disposal: bool,
    ) -> Self {
        Self::volatility_interruption_with_evidence(
            AuctionEvidence::Known(direction),
            AuctionEvidence::Known(delayed),
            AuctionEvidence::Known(disposal),
        )
    }

    #[must_use]
    pub const fn volatility_interruption_with_evidence(
        direction: AuctionEvidence<VolatilityDirection>,
        delayed: AuctionEvidence<bool>,
        disposal: AuctionEvidence<bool>,
    ) -> Self {
        Self {
            purpose: AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption),
            delayed,
            disposal,
            direction,
        }
    }

    #[must_use]
    pub const fn unclassified() -> Self {
        Self {
            purpose: AuctionEvidence::NoObservation,
            delayed: AuctionEvidence::NoObservation,
            disposal: AuctionEvidence::NoObservation,
            direction: AuctionEvidence::NoObservation,
        }
    }

    #[must_use]
    pub const fn unknown_purpose() -> Self {
        Self {
            purpose: AuctionEvidence::Unknown,
            delayed: AuctionEvidence::NoObservation,
            disposal: AuctionEvidence::NoObservation,
            direction: AuctionEvidence::NoObservation,
        }
    }

    #[must_use]
    pub fn merge(self, prior: Self) -> Self {
        Self {
            purpose: merge_evidence(prior.purpose, self.purpose),
            delayed: merge_evidence(prior.delayed, self.delayed),
            disposal: merge_evidence(prior.disposal, self.disposal),
            direction: merge_evidence(prior.direction, self.direction),
        }
    }

    #[must_use]
    pub const fn opening(delayed: bool, disposal: bool) -> Self {
        Self::opening_with_evidence(
            AuctionEvidence::Known(delayed),
            AuctionEvidence::Known(disposal),
        )
    }

    #[must_use]
    pub const fn opening_with_evidence(
        delayed: AuctionEvidence<bool>,
        disposal: AuctionEvidence<bool>,
    ) -> Self {
        Self {
            purpose: AuctionEvidence::Known(AuctionPurpose::Opening),
            delayed,
            disposal,
            direction: AuctionEvidence::NoObservation,
        }
    }

    #[must_use]
    pub const fn closing(delayed: bool, disposal: bool) -> Self {
        Self::closing_with_evidence(
            AuctionEvidence::Known(delayed),
            AuctionEvidence::Known(disposal),
        )
    }

    #[must_use]
    pub const fn closing_with_evidence(
        delayed: AuctionEvidence<bool>,
        disposal: AuctionEvidence<bool>,
    ) -> Self {
        Self {
            purpose: AuctionEvidence::Known(AuctionPurpose::Closing),
            delayed,
            disposal,
            direction: AuctionEvidence::NoObservation,
        }
    }

    #[must_use]
    pub const fn periodic(delayed: bool, disposal: bool) -> Self {
        Self::periodic_with_evidence(
            AuctionEvidence::Known(delayed),
            AuctionEvidence::Known(disposal),
        )
    }

    #[must_use]
    pub const fn periodic_with_evidence(
        delayed: AuctionEvidence<bool>,
        disposal: AuctionEvidence<bool>,
    ) -> Self {
        Self {
            purpose: AuctionEvidence::Known(AuctionPurpose::Periodic),
            delayed,
            disposal,
            direction: AuctionEvidence::NoObservation,
        }
    }

    #[must_use]
    pub const fn purpose(self) -> AuctionEvidence<AuctionPurpose> {
        self.purpose
    }

    #[must_use]
    pub const fn delayed(self) -> AuctionEvidence<bool> {
        self.delayed
    }

    #[must_use]
    pub const fn disposal(self) -> AuctionEvidence<bool> {
        self.disposal
    }

    #[must_use]
    pub const fn direction(self) -> AuctionEvidence<VolatilityDirection> {
        self.direction
    }
}

fn merge_evidence<T: Copy>(
    prior: AuctionEvidence<T>,
    current: AuctionEvidence<T>,
) -> AuctionEvidence<T> {
    match current {
        AuctionEvidence::NoObservation => prior,
        AuctionEvidence::Known(value) => AuctionEvidence::Known(value),
        AuctionEvidence::Unknown => AuctionEvidence::Unknown,
    }
}

/// Provider-neutral market regime observation carried by a domain event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarketSignal {
    Continuous,
    AuctionCollecting(AuctionObservation),
    AuctionUncross(AuctionObservation),
    Closed,
}

impl MarketSignal {
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        match self {
            Self::Continuous => 1,
            Self::AuctionCollecting(_) => 2,
            Self::AuctionUncross(_) => 3,
            Self::Closed => 4,
        }
    }

    #[must_use]
    pub const fn auction(self) -> Option<AuctionObservation> {
        match self {
            Self::AuctionCollecting(observation) | Self::AuctionUncross(observation) => {
                Some(observation)
            }
            Self::Continuous | Self::Closed => None,
        }
    }

    pub(crate) fn from_discriminant(
        value: u8,
        observation: Option<AuctionObservation>,
    ) -> Option<Self> {
        match value {
            1 if observation.is_none() => Some(Self::Continuous),
            2 => observation.map(Self::AuctionCollecting),
            3 => observation.map(Self::AuctionUncross),
            4 if observation.is_none() => Some(Self::Closed),
            _ => None,
        }
    }
}

impl CanonicalValue for AuctionObservation {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        self.purpose.append_canonical(bytes)?;
        self.delayed.append_canonical(bytes)?;
        self.disposal.append_canonical(bytes)?;
        self.direction.append_canonical(bytes)?;
        Ok(())
    }
}

impl CanonicalValue for AuctionEvidence<AuctionPurpose> {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        match self {
            Self::NoObservation => bytes.push(0),
            Self::Known(purpose) => {
                bytes.push(1);
                bytes.push(purpose.discriminant());
            }
            Self::Unknown => bytes.push(2),
        }
        Ok(())
    }
}

impl CanonicalValue for AuctionEvidence<bool> {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        match self {
            Self::NoObservation => bytes.push(0),
            Self::Known(value) => {
                bytes.push(1);
                bytes.push(u8::from(*value));
            }
            Self::Unknown => bytes.push(2),
        }
        Ok(())
    }
}

impl CanonicalValue for AuctionEvidence<VolatilityDirection> {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        match self {
            Self::NoObservation => bytes.push(0),
            Self::Known(direction) => {
                bytes.push(1);
                bytes.push(direction.discriminant());
            }
            Self::Unknown => bytes.push(2),
        }
        Ok(())
    }
}

impl CanonicalValue for MarketSignal {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        bytes.push(self.discriminant());
        match self {
            Self::AuctionCollecting(observation) | Self::AuctionUncross(observation) => {
                observation.append_canonical(bytes)?;
            }
            Self::Continuous | Self::Closed => {}
        }
        Ok(())
    }
}
