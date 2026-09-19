use std::{error::Error, fmt};

/// Stable identity of the wire-to-domain mapping used to derive a replay cache.
///
/// Cache codecs only need canonical schema versions to read an artifact. The
/// acquisition provider owns this identity; planners use it to decide whether
/// an existing derived artifact may be reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizerMappingIdentity {
    name: Box<str>,
    version: u16,
}

impl NormalizerMappingIdentity {
    pub fn new(name: impl Into<Box<str>>, version: u16) -> Result<Self, MappingIdentityError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(MappingIdentityError::EmptyName);
        }
        if version == 0 {
            return Err(MappingIdentityError::ZeroVersion);
        }
        Ok(Self { name, version })
    }

    pub fn from_static(name: &'static str, version: u16) -> Self {
        Self::new(name, version).expect("built-in mapping identities are valid")
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingIdentityError {
    EmptyName,
    ZeroVersion,
}

impl fmt::Display for MappingIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("normalizer mapping name cannot be empty"),
            Self::ZeroVersion => formatter.write_str("normalizer mapping version must be non-zero"),
        }
    }
}

impl Error for MappingIdentityError {}
