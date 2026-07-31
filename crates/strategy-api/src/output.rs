use std::{error::Error, fmt};

use market_types::{CanonicalEncodingError, Decimal, append_bytes};
use replay_engine::EventOccurrence;

use crate::{CanonicalParamsChecksum, OrderIntent, OrderIntentError, StrategyIdentity};

pub const CANONICAL_STRATEGY_OUTPUT_VERSION: u16 = 1;
const STRATEGY_OUTPUT_MAGIC: &[u8; 4] = b"OSSO";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndicatorValue {
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    Decimal(Decimal),
    Text(Box<str>),
}

impl IndicatorValue {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        match self {
            Self::Bool(value) => {
                bytes.push(1);
                bytes.push(u8::from(*value));
            }
            Self::Signed(value) => {
                bytes.push(2);
                bytes.extend_from_slice(&value.to_be_bytes());
            }
            Self::Unsigned(value) => {
                bytes.push(3);
                bytes.extend_from_slice(&value.to_be_bytes());
            }
            Self::Decimal(value) => {
                bytes.push(4);
                bytes.extend_from_slice(&value.to_canonical_bytes());
            }
            Self::Text(value) => {
                bytes.push(5);
                append_bytes(value.as_bytes(), bytes)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyOutputRecord {
    EventIndicator {
        run_event_ordinal: u64,
        event_fingerprint: [u8; 32],
        instrument_state_version: u64,
        output_sequence: u32,
        indicator_name: Box<str>,
        value: IndicatorValue,
    },
    FinalizeIndicator {
        output_sequence: u32,
        indicator_name: Box<str>,
        value: IndicatorValue,
    },
}

impl StrategyOutputRecord {
    fn append_canonical(&self, bytes: &mut Vec<u8>) -> Result<(), CanonicalEncodingError> {
        match self {
            Self::EventIndicator {
                run_event_ordinal,
                event_fingerprint,
                instrument_state_version,
                output_sequence,
                indicator_name,
                value,
            } => {
                bytes.push(1);
                bytes.extend_from_slice(&run_event_ordinal.to_be_bytes());
                bytes.extend_from_slice(event_fingerprint);
                bytes.extend_from_slice(&instrument_state_version.to_be_bytes());
                bytes.extend_from_slice(&output_sequence.to_be_bytes());
                append_bytes(indicator_name.as_bytes(), bytes)?;
                value.append_canonical(bytes)?;
            }
            Self::FinalizeIndicator {
                output_sequence,
                indicator_name,
                value,
            } => {
                bytes.push(2);
                bytes.extend_from_slice(&output_sequence.to_be_bytes());
                append_bytes(indicator_name.as_bytes(), bytes)?;
                value.append_canonical(bytes)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct StrategyOutputSink {
    pending: Vec<(Box<str>, IndicatorValue)>,
    intents: Vec<OrderIntent>,
    order_intents_enabled: bool,
}

impl StrategyOutputSink {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: Vec::new(),
            intents: Vec::new(),
            order_intents_enabled: false,
        }
    }

    #[must_use]
    pub const fn with_order_intents() -> Self {
        Self {
            pending: Vec::new(),
            intents: Vec::new(),
            order_intents_enabled: true,
        }
    }

    pub fn emit_order_intent(&mut self, intent: OrderIntent) -> Result<(), OrderIntentError> {
        if !self.order_intents_enabled {
            return Err(OrderIntentError);
        }
        self.intents.push(intent);
        Ok(())
    }

    #[must_use]
    pub fn intents(&self) -> &[OrderIntent] {
        &self.intents
    }

    pub fn take_intents(&mut self) -> Vec<OrderIntent> {
        std::mem::take(&mut self.intents)
    }

    pub fn emit_indicator(
        &mut self,
        name: impl Into<Box<str>>,
        value: IndicatorValue,
    ) -> Result<(), StrategyOutputEncodingError> {
        let name = name.into();
        if name.is_empty() {
            return Err(StrategyOutputEncodingError::EmptyIndicatorName);
        }
        self.pending.push((name, value));
        Ok(())
    }

    pub fn into_event_records(
        self,
        occurrence: &EventOccurrence,
    ) -> Result<Vec<StrategyOutputRecord>, StrategyOutputEncodingError> {
        self.pending
            .into_iter()
            .enumerate()
            .map(|(index, (indicator_name, value))| {
                let output_sequence = u32::try_from(index + 1)
                    .map_err(|_| StrategyOutputEncodingError::OutputSequenceOverflow)?;
                Ok(StrategyOutputRecord::EventIndicator {
                    run_event_ordinal: occurrence.run_event_ordinal(),
                    event_fingerprint: *occurrence.event_fingerprint().as_bytes(),
                    instrument_state_version: occurrence.instrument_state_version(),
                    output_sequence,
                    indicator_name,
                    value,
                })
            })
            .collect()
    }

    pub fn into_finalize_records(
        self,
    ) -> Result<Vec<StrategyOutputRecord>, StrategyOutputEncodingError> {
        self.pending
            .into_iter()
            .enumerate()
            .map(|(index, (indicator_name, value))| {
                let output_sequence = u32::try_from(index + 1)
                    .map_err(|_| StrategyOutputEncodingError::OutputSequenceOverflow)?;
                Ok(StrategyOutputRecord::FinalizeIndicator {
                    output_sequence,
                    indicator_name,
                    value,
                })
            })
            .collect()
    }
}

impl Default for StrategyOutputSink {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct StrategyOutput {
    identity: StrategyIdentity,
    canonical_params_checksum: CanonicalParamsChecksum,
    records: Vec<StrategyOutputRecord>,
}

impl StrategyOutput {
    pub const fn new(
        identity: StrategyIdentity,
        canonical_params_checksum: CanonicalParamsChecksum,
    ) -> Self {
        Self {
            identity,
            canonical_params_checksum,
            records: Vec::new(),
        }
    }

    pub fn extend(&mut self, records: Vec<StrategyOutputRecord>) {
        self.records.extend(records);
    }

    #[must_use]
    pub const fn identity(&self) -> &StrategyIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
        self.canonical_params_checksum
    }

    #[must_use]
    pub fn records(&self) -> &[StrategyOutputRecord] {
        &self.records
    }

    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, StrategyOutputEncodingError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(STRATEGY_OUTPUT_MAGIC);
        bytes.extend_from_slice(&CANONICAL_STRATEGY_OUTPUT_VERSION.to_be_bytes());
        append_bytes(self.identity.strategy_id().as_bytes(), &mut bytes)?;
        append_bytes(self.identity.strategy_version().as_bytes(), &mut bytes)?;
        append_bytes(
            self.identity.binary_identity().algorithm().as_bytes(),
            &mut bytes,
        )?;
        append_bytes(self.identity.binary_identity().digest(), &mut bytes)?;
        bytes.extend_from_slice(self.canonical_params_checksum.as_bytes());
        let count = u64::try_from(self.records.len())
            .map_err(|_| StrategyOutputEncodingError::RecordCountOverflow)?;
        bytes.extend_from_slice(&count.to_be_bytes());
        for record in &self.records {
            record.append_canonical(&mut bytes)?;
        }
        Ok(bytes)
    }

    pub fn checksum(&self) -> Result<StrategyOutputChecksum, StrategyOutputEncodingError> {
        let bytes = self.to_canonical_bytes()?;
        Ok(StrategyOutputChecksum(*blake3::hash(&bytes).as_bytes()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrategyOutputChecksum([u8; 32]);

impl StrategyOutputChecksum {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyOutputEncodingError {
    EmptyIndicatorName,
    OutputSequenceOverflow,
    RecordCountOverflow,
    Canonical(CanonicalEncodingError),
}

impl fmt::Display for StrategyOutputEncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyIndicatorName => formatter.write_str("indicator name must not be empty"),
            Self::OutputSequenceOverflow => {
                formatter.write_str("callback output sequence exceeds u32")
            }
            Self::RecordCountOverflow => {
                formatter.write_str("strategy output record count exceeds u64")
            }
            Self::Canonical(error) => error.fmt(formatter),
        }
    }
}

impl Error for StrategyOutputEncodingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Canonical(error) => Some(error),
            Self::EmptyIndicatorName | Self::OutputSequenceOverflow | Self::RecordCountOverflow => {
                None
            }
        }
    }
}

impl From<CanonicalEncodingError> for StrategyOutputEncodingError {
    fn from(error: CanonicalEncodingError) -> Self {
        Self::Canonical(error)
    }
}
