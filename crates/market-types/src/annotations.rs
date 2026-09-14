/// Market-specific annotations carried atomically by a market event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MarketAnnotations {
    None,
    TwseQuote(TwseQuoteAnnotations),
    TpexQuote(TpexQuoteAnnotations),
}

impl MarketAnnotations {
    #[must_use]
    pub const fn discriminant(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::TwseQuote(_) => 1,
            Self::TpexQuote(_) => 2,
        }
    }
}

/// Lossless TPEx quote-header status and limit bytes.
///
/// TPEx uses the same wire field names as the committed regular-equity fixture,
/// but the annotation type is intentionally market-specific so the normalizer
/// cannot silently apply TWSE semantics to a TPEx record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TpexQuoteAnnotations {
    status_flags_raw: u8,
    limit_flags_raw: u8,
}

impl TpexQuoteAnnotations {
    #[must_use]
    pub const fn new(status_flags_raw: u8, limit_flags_raw: u8) -> Self {
        Self {
            status_flags_raw,
            limit_flags_raw,
        }
    }

    #[must_use]
    pub const fn status_flags_raw(self) -> u8 {
        self.status_flags_raw
    }

    #[must_use]
    pub const fn limit_flags_raw(self) -> u8 {
        self.limit_flags_raw
    }

    #[must_use]
    pub const fn status(self) -> TpexStatus {
        TpexStatus::from_raw(self.status_flags_raw)
    }

    #[must_use]
    pub const fn limits(self) -> TpexLimits {
        TpexLimits::from_raw(self.limit_flags_raw)
    }
}

/// Fixture-verified typed view over TPEx status bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TpexStatus {
    raw: u8,
}

impl TpexStatus {
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self { raw }
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.raw
    }

    #[must_use]
    pub const fn trial(self) -> bool {
        QuoteStatusBits(self.raw).trial()
    }

    #[must_use]
    pub const fn delayed_open(self) -> bool {
        QuoteStatusBits(self.raw).delayed_open()
    }

    #[must_use]
    pub const fn delayed_close(self) -> bool {
        QuoteStatusBits(self.raw).delayed_close()
    }

    #[must_use]
    pub const fn matching_method(self) -> MatchingMethod {
        QuoteStatusBits(self.raw).matching_method()
    }

    #[must_use]
    pub const fn opening_marker(self) -> bool {
        QuoteStatusBits(self.raw).opening_marker()
    }

    #[must_use]
    pub const fn closing_marker(self) -> bool {
        QuoteStatusBits(self.raw).closing_marker()
    }

    #[must_use]
    pub const fn reserved_bits(self) -> u8 {
        QuoteStatusBits(self.raw).reserved_bits()
    }
}

/// Fixture-verified typed view over TPEx limit bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TpexLimits {
    raw: u8,
}

impl TpexLimits {
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self { raw }
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.raw
    }

    #[must_use]
    pub const fn trade(self) -> LimitPosition {
        QuoteLimitBits(self.raw).trade()
    }

    #[must_use]
    pub const fn best_bid(self) -> LimitPosition {
        QuoteLimitBits(self.raw).best_bid()
    }

    #[must_use]
    pub const fn best_ask(self) -> LimitPosition {
        QuoteLimitBits(self.raw).best_ask()
    }

    #[must_use]
    pub const fn instant_trend(self) -> InstantTrend {
        QuoteLimitBits(self.raw).instant_trend()
    }
}

/// Lossless TWSE quote-header status and limit bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TwseQuoteAnnotations {
    status_flags_raw: u8,
    limit_flags_raw: u8,
}

impl TwseQuoteAnnotations {
    #[must_use]
    pub const fn new(status_flags_raw: u8, limit_flags_raw: u8) -> Self {
        Self {
            status_flags_raw,
            limit_flags_raw,
        }
    }

    #[must_use]
    pub const fn status_flags_raw(self) -> u8 {
        self.status_flags_raw
    }

    #[must_use]
    pub const fn limit_flags_raw(self) -> u8 {
        self.limit_flags_raw
    }

    #[must_use]
    pub const fn status(self) -> TwseStatus {
        TwseStatus::from_raw(self.status_flags_raw)
    }

    #[must_use]
    pub const fn limits(self) -> TwseLimits {
        TwseLimits::from_raw(self.limit_flags_raw)
    }
}

/// Typed view over the independent bits in a TWSE status byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TwseStatus {
    raw: u8,
}

impl TwseStatus {
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self { raw }
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.raw
    }

    #[must_use]
    pub const fn trial(self) -> bool {
        QuoteStatusBits(self.raw).trial()
    }

    #[must_use]
    pub const fn delayed_open(self) -> bool {
        QuoteStatusBits(self.raw).delayed_open()
    }

    #[must_use]
    pub const fn delayed_close(self) -> bool {
        QuoteStatusBits(self.raw).delayed_close()
    }

    #[must_use]
    pub const fn matching_method(self) -> MatchingMethod {
        QuoteStatusBits(self.raw).matching_method()
    }

    #[must_use]
    pub const fn opening_marker(self) -> bool {
        QuoteStatusBits(self.raw).opening_marker()
    }

    #[must_use]
    pub const fn closing_marker(self) -> bool {
        QuoteStatusBits(self.raw).closing_marker()
    }

    #[must_use]
    pub const fn reserved_bits(self) -> u8 {
        QuoteStatusBits(self.raw).reserved_bits()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchingMethod {
    CallAuction,
    Continuous,
}

/// Typed view over the four two-bit fields in a TWSE limit byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TwseLimits {
    raw: u8,
}

impl TwseLimits {
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self { raw }
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.raw
    }

    #[must_use]
    pub const fn trade(self) -> LimitPosition {
        QuoteLimitBits(self.raw).trade()
    }

    #[must_use]
    pub const fn best_bid(self) -> LimitPosition {
        QuoteLimitBits(self.raw).best_bid()
    }

    #[must_use]
    pub const fn best_ask(self) -> LimitPosition {
        QuoteLimitBits(self.raw).best_ask()
    }

    #[must_use]
    pub const fn instant_trend(self) -> InstantTrend {
        QuoteLimitBits(self.raw).instant_trend()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitPosition {
    Normal,
    LowerLimit,
    UpperLimit,
    Reserved,
}

impl LimitPosition {
    const fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Normal,
            1 => Self::LowerLimit,
            2 => Self::UpperLimit,
            _ => Self::Reserved,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstantTrend {
    Normal,
    VolatilityInterruptionDown,
    VolatilityInterruptionUp,
    Reserved,
}

impl InstantTrend {
    const fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Normal,
            1 => Self::VolatilityInterruptionDown,
            2 => Self::VolatilityInterruptionUp,
            _ => Self::Reserved,
        }
    }
}

/// Private decoder for currently shared TWSE/TPEx status-byte positions.
/// The public wrappers remain market-specific so a future wire divergence is explicit.
#[derive(Debug, Clone, Copy)]
struct QuoteStatusBits(u8);

impl QuoteStatusBits {
    const fn trial(self) -> bool {
        self.0 & 0x80 != 0
    }

    const fn delayed_open(self) -> bool {
        self.trial() && self.0 & 0x40 != 0
    }

    const fn delayed_close(self) -> bool {
        self.trial() && self.0 & 0x20 != 0
    }

    const fn matching_method(self) -> MatchingMethod {
        if self.0 & 0x10 != 0 {
            MatchingMethod::Continuous
        } else {
            MatchingMethod::CallAuction
        }
    }

    const fn opening_marker(self) -> bool {
        self.0 & 0x08 != 0
    }

    const fn closing_marker(self) -> bool {
        self.0 & 0x04 != 0
    }

    const fn reserved_bits(self) -> u8 {
        self.0 & 0x03
    }
}

/// Private decoder for currently shared TWSE/TPEx limit-byte positions.
#[derive(Debug, Clone, Copy)]
struct QuoteLimitBits(u8);

impl QuoteLimitBits {
    const fn trade(self) -> LimitPosition {
        LimitPosition::from_bits((self.0 >> 6) & 0x03)
    }

    const fn best_bid(self) -> LimitPosition {
        LimitPosition::from_bits((self.0 >> 4) & 0x03)
    }

    const fn best_ask(self) -> LimitPosition {
        LimitPosition::from_bits((self.0 >> 2) & 0x03)
    }

    const fn instant_trend(self) -> InstantTrend {
        InstantTrend::from_bits(self.0 & 0x03)
    }
}
