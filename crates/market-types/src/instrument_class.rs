/// Product kind carried by the explicit instrument contract reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum InstrumentClass {
    Equity = 1,
    Warrant = 2,
    Future = 3,
    Option = 4,
}

/// Market contract shape that changes quote semantics beyond broad instrument class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ContractShape {
    Outright = 1,
    CalendarSpread = 2,
}

impl ContractShape {
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }
}

/// Whether an instrument profile accepts only positive prices or signed spread values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum PricePolicy {
    PositiveOnly = 1,
    Signed = 2,
}

impl PricePolicy {
    #[must_use]
    pub const fn accepts(self, price: crate::Price) -> bool {
        matches!(self, Self::Signed) || price.atoms() > 0
    }

    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }
}

/// Call/put side for an option-like contract reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum OptionSide {
    Call = 1,
    Put = 2,
}
