pub use crate::equity_quote::{
    ConfigError, InstrumentProfile, KnownSkipReason, KnownSkipped, NormalizationError,
    NormalizationErrorKind, NormalizationReport, NormalizationWarning, NormalizerConfig,
    QuoteNormalizer as TwseNormalizer, RealtimeGroupError, RecordContext, WarningKind,
};

pub const MAPPING_NAME: &str = "TeralionTwseQuote";
pub const MAPPING_VERSION: u16 = 11;
pub const WARRANT_MAPPING_NAME: &str = "TeralionTwseWarrant";
pub const WARRANT_MAPPING_VERSION: u16 = 7;
