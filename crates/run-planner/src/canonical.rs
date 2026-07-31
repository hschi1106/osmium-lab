use market_types::{Decimal, InstrumentId};
use strategy_api::{CanonicalParamsChecksum, SessionKind, StrategyIdentity};

use crate::config::ConfigError;

pub(crate) fn append_len(length: usize, output: &mut Vec<u8>) -> Result<(), ConfigError> {
    let length = u32::try_from(length).map_err(|_| ConfigError::CanonicalLengthOverflow)?;
    output.extend_from_slice(&length.to_be_bytes());
    Ok(())
}

pub(crate) fn append_bytes(value: &[u8], output: &mut Vec<u8>) -> Result<(), ConfigError> {
    append_len(value.len(), output)?;
    output.extend_from_slice(value);
    Ok(())
}

pub(crate) fn append_text(value: &str, output: &mut Vec<u8>) -> Result<(), ConfigError> {
    append_bytes(value.as_bytes(), output)
}

pub(crate) fn append_decimal(value: Decimal, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_canonical_bytes());
}

pub(crate) fn append_instrument(
    instrument: &InstrumentId,
    output: &mut Vec<u8>,
) -> Result<(), ConfigError> {
    output.push(instrument.market().discriminant());
    append_bytes(instrument.symbol().as_bytes(), output)
}

pub(crate) fn append_session(session: SessionKind, output: &mut Vec<u8>) {
    output.push(session as u8);
}

pub(crate) fn append_strategy_identity(
    identity: &StrategyIdentity,
    params: CanonicalParamsChecksum,
    output: &mut Vec<u8>,
) -> Result<(), ConfigError> {
    append_text(identity.strategy_id(), output)?;
    append_text(identity.strategy_version(), output)?;
    append_text(identity.binary_identity().algorithm(), output)?;
    append_bytes(identity.binary_identity().digest(), output)?;
    output.extend_from_slice(params.as_bytes());
    Ok(())
}
