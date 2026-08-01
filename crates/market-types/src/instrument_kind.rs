/// Product kind carried by the explicit instrument contract reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum InstrumentKind {
    Equity = 1,
    Warrant = 2,
    Future = 3,
    Option = 4,
    Unknown = 255,
}

/// Call/put side for an option-like contract reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum OptionSide {
    Call = 1,
    Put = 2,
}
