pub use crate::equity_quote::{
    ConfigError, InstrumentProfile, KnownSkipReason, KnownSkipped, NormalizationError,
    NormalizationErrorKind, NormalizationReport, NormalizationWarning, NormalizerConfig,
    QuoteNormalizer as TpexNormalizer, RealtimeGroupError, RecordContext, WarningKind,
};

pub const MAPPING_NAME: &str = "TeralionTpexQuote";
pub const MAPPING_VERSION: u16 = 9;
pub const WARRANT_MAPPING_NAME: &str = "TeralionTpexWarrant";
pub const WARRANT_MAPPING_VERSION: u16 = 8;
