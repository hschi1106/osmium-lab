use std::cmp::Ordering;

/// Fixed-capacity, non-empty UTF-8 text stored inline so identifiers are `Copy`.
///
/// Bytes past `len` are always zero, so the derived `Eq`/`Hash` over the whole
/// array agree with byte-exact equality of the stored text.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct InlineStr<const N: usize> {
    len: u8,
    bytes: [u8; N],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineStrError {
    Empty,
    TooLong,
}

impl<const N: usize> InlineStr<N> {
    pub(crate) fn new(value: &str) -> Result<Self, InlineStrError> {
        const { assert!(N <= u8::MAX as usize) };
        let source = value.as_bytes();
        if source.is_empty() {
            return Err(InlineStrError::Empty);
        }
        if source.len() > N {
            return Err(InlineStrError::TooLong);
        }
        let mut bytes = [0; N];
        bytes[..source.len()].copy_from_slice(source);
        Ok(Self {
            len: u8::try_from(source.len()).map_err(|_| InlineStrError::TooLong)?,
            bytes,
        })
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(self.as_bytes()).expect("inline text is copied from a valid str")
    }
}

impl<const N: usize> PartialOrd for InlineStr<N> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<const N: usize> Ord for InlineStr<N> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl<const N: usize> std::fmt::Debug for InlineStr<N> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::{InlineStr, InlineStrError};

    #[test]
    fn rejects_empty_and_over_capacity_text() {
        assert_eq!(InlineStr::<4>::new(""), Err(InlineStrError::Empty));
        assert_eq!(InlineStr::<4>::new("12345"), Err(InlineStrError::TooLong));
        assert_eq!(InlineStr::<4>::new("1234").unwrap().as_str(), "1234");
    }

    #[test]
    fn orders_and_compares_by_exact_bytes() {
        let short = InlineStr::<8>::new("a").unwrap();
        let nul = InlineStr::<8>::new("a\0").unwrap();
        assert_ne!(short, nul);
        assert!(short < nul);
        assert!(InlineStr::<8>::new("2330").unwrap() < InlineStr::<8>::new("2331").unwrap());
    }
}
