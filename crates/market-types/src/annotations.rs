/// Market-specific annotations carried atomically by a market event.
///
/// No source currently attaches annotations; the enum keeps the canonical slot
/// so neutral annotations (for example price-limit state) can be added later.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MarketAnnotations {
    None,
}

impl MarketAnnotations {
    #[must_use]
    pub const fn discriminant(&self) -> u8 {
        match self {
            Self::None => 0,
        }
    }
}

/// How an enabled market matches orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchingMethod {
    CallAuction,
    Continuous,
}
