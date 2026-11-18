use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
};

use crate::BookId;

/// A map from books to `V`.
///
/// A replay looks up a book for every message, twice: where it takes effect at the
/// exchange and where it reaches the strategy. The standard library's default hash is
/// keyed against hostile input and takes more time than the rest of such a lookup; the
/// keys here are the instrument codes of marketdata.
///
/// The order in which a map iterates is not part of any result: callers sort.
pub type BookMap<V> = HashMap<BookId, V, BuildHasherDefault<BookHasher>>;

/// The hash of [`BookMap`]: multiply and rotate, eight bytes at a time.
#[derive(Debug, Clone, Copy, Default)]
pub struct BookHasher(u64);

impl BookHasher {
    /// An odd constant with no structure: the fractional bits of the golden ratio.
    const MULTIPLIER: u64 = 0x9e37_79b9_7f4a_7c15;

    const fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(Self::MULTIPLIER);
    }
}

impl Hasher for BookHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[allow(clippy::cast_sign_loss)]
    fn write_isize(&mut self, value: isize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        // A multiplication leaves its low bits depending only on the low bits of its
        // input, and a hash map takes its bucket from the low bits.
        self.0.rotate_left(26)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, hash::BuildHasher};

    use osmium_types::{Board, Symbol};

    use super::BookMap;
    use crate::BookId;

    fn id(code: &str, board: Board) -> BookId {
        BookId {
            symbol: Symbol::new(code).unwrap(),
            board,
        }
    }

    #[test]
    fn books_are_found_by_symbol_and_board() {
        let mut map = BookMap::default();
        for code in 1_000..9_999 {
            let code = code.to_string();
            map.insert(id(&code, Board::Regular), (code.clone(), false));
            map.insert(id(&code, Board::OddLot), (code, true));
        }
        assert_eq!(map.len(), 2 * 8_999);
        assert_eq!(
            map.get(&id("2330", Board::Regular)),
            Some(&("2330".to_owned(), false))
        );
        assert_eq!(
            map.get(&id("2330", Board::OddLot)),
            Some(&("2330".to_owned(), true))
        );
        assert_eq!(map.get(&id("233", Board::Regular)), None);
        assert_eq!(map.get(&id("00631L", Board::Regular)), None);
    }

    #[test]
    fn codes_that_differ_in_one_place_spread_over_the_buckets() {
        let map = BookMap::<()>::default();
        // The 7 bits a hash map compares first and the bits it takes a bucket from.
        let (mut tags, mut buckets) = (HashSet::new(), HashSet::new());
        for code in 0..4_096 {
            let hash = map
                .hasher()
                .hash_one(id(&format!("{code:04}"), Board::Regular));
            tags.insert(hash >> 57);
            buckets.insert(hash & 0xfff);
        }
        assert_eq!(tags.len(), 128);
        // 4,096 keys thrown at 4,096 buckets at random fill about 63 % of them.
        assert!(buckets.len() > 2_300, "{} buckets", buckets.len());
    }
}
