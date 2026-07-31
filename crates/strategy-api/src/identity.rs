use std::{collections::BTreeSet, error::Error, fmt};

use market_types::{InstrumentId, append_bytes};

use crate::SessionKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryIdentity {
    algorithm: Box<str>,
    digest: Box<[u8]>,
}

impl BinaryIdentity {
    pub fn new(
        algorithm: impl Into<Box<str>>,
        digest: impl Into<Box<[u8]>>,
    ) -> Result<Self, DeclarationError> {
        let algorithm = algorithm.into();
        let digest = digest.into();
        if algorithm.is_empty() {
            return Err(DeclarationError::EmptyBinaryIdentityAlgorithm);
        }
        if digest.is_empty() {
            return Err(DeclarationError::EmptyBinaryIdentityDigest);
        }
        Ok(Self { algorithm, digest })
    }

    #[must_use]
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    #[must_use]
    pub fn digest(&self) -> &[u8] {
        &self.digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyIdentity {
    strategy_id: Box<str>,
    strategy_version: Box<str>,
    binary_identity: BinaryIdentity,
}

impl StrategyIdentity {
    pub fn new(
        strategy_id: impl Into<Box<str>>,
        strategy_version: impl Into<Box<str>>,
        binary_identity: BinaryIdentity,
    ) -> Result<Self, DeclarationError> {
        let strategy_id = strategy_id.into();
        let strategy_version = strategy_version.into();
        if strategy_id.is_empty() {
            return Err(DeclarationError::EmptyStrategyId);
        }
        if strategy_version.is_empty() {
            return Err(DeclarationError::EmptyStrategyVersion);
        }
        Ok(Self {
            strategy_id,
            strategy_version,
            binary_identity,
        })
    }

    #[must_use]
    pub fn strategy_id(&self) -> &str {
        &self.strategy_id
    }

    #[must_use]
    pub fn strategy_version(&self) -> &str {
        &self.strategy_version
    }

    #[must_use]
    pub const fn binary_identity(&self) -> &BinaryIdentity {
        &self.binary_identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalParamsChecksum([u8; 32]);

impl CanonicalParamsChecksum {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn for_empty_params() -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OSSP");
        bytes.extend_from_slice(&1_u16.to_be_bytes());
        append_bytes(&[], &mut bytes).expect("empty parameter frame fits in u32");
        Self(*blake3::hash(&bytes).as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyDeclaration {
    universe: Box<[InstrumentId]>,
    sessions: Box<[SessionKind]>,
}

impl StrategyDeclaration {
    pub fn new(
        universe: impl IntoIterator<Item = InstrumentId>,
        sessions: impl IntoIterator<Item = SessionKind>,
    ) -> Result<Self, DeclarationError> {
        let universe = universe.into_iter().collect::<BTreeSet<_>>();
        let sessions = sessions.into_iter().collect::<BTreeSet<_>>();
        if universe.is_empty() {
            return Err(DeclarationError::EmptyUniverse);
        }
        if sessions.is_empty() {
            return Err(DeclarationError::EmptySessions);
        }
        Ok(Self {
            universe: universe.into_iter().collect(),
            sessions: sessions.into_iter().collect(),
        })
    }

    #[must_use]
    pub const fn universe(&self) -> &[InstrumentId] {
        &self.universe
    }

    #[must_use]
    pub const fn sessions(&self) -> &[SessionKind] {
        &self.sessions
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationError {
    EmptyStrategyId,
    EmptyStrategyVersion,
    EmptyBinaryIdentityAlgorithm,
    EmptyBinaryIdentityDigest,
    EmptyUniverse,
    EmptySessions,
}

impl fmt::Display for DeclarationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyStrategyId => "strategy_id must not be empty",
            Self::EmptyStrategyVersion => "strategy_version must not be empty",
            Self::EmptyBinaryIdentityAlgorithm => "binary identity algorithm must not be empty",
            Self::EmptyBinaryIdentityDigest => "binary identity digest must not be empty",
            Self::EmptyUniverse => "strategy universe must not be empty",
            Self::EmptySessions => "strategy session declaration must not be empty",
        };
        formatter.write_str(message)
    }
}

impl Error for DeclarationError {}
