//! The in-memory form of a symbol's `books` rows: each row as its difference from the
//! row before it.
//!
//! A book message repeats most of the five levels of the previous one, so a day kept as
//! full rows is several times larger than it needs to be. Rows of one symbol are only
//! ever read in order, which is all a difference encoding allows.
//!
//! A row is a tag byte, the change of `exch_ts`, and for the kinds that disclose a book
//! the two sides. The format is private to one process; nothing is written to disk.

use osmium_types::{BOOK_DEPTH, DelayDirection, Phase};

use crate::decode::{BookKind, BookRow, CompactSide};

/// What decoding a row needs from the rows before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) exch_ts: i64,
    pub(crate) bid: CompactSide,
    pub(crate) ask: CompactSide,
}

impl Frame {
    pub(crate) const START: Self = Self {
        exch_ts: 0,
        bid: CompactSide::EMPTY,
        ask: CompactSide::EMPTY,
    };
}

/// A decoded row. Its `exch_ts` and, for the kinds that disclose one, its book are in
/// the [`Frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decoded {
    pub(crate) phase: Phase,
    pub(crate) kind: BookKind,
    /// Zero when the row has no indicative match.
    pub(crate) indicative_price: u32,
    pub(crate) indicative_qty: u32,
    pub(crate) delay_open: Option<bool>,
    pub(crate) delay_close: Option<bool>,
}

const KIND_MASK: u8 = 0b0000_0111;
const PHASE_SHIFT: u8 = 3;
const PHASE_MASK: u8 = 0b0001_1000;
/// The row does not build on the rows before it.
const RESET: u8 = 0b0010_0000;
const SAME_BID: u8 = 0b0100_0000;
const SAME_ASK: u8 = 0b1000_0000;

const LEN_MASK: u16 = 0b111;
const MARKET_CHANGED: u16 = 1 << 3;
const PRICE_SHIFT: u16 = 4;
const QTY_SHIFT: u16 = 4 + 5;

const fn kind_code(kind: BookKind) -> u8 {
    match kind {
        BookKind::Disclosure { has_trade: false } => 0,
        BookKind::Disclosure { has_trade: true } => 1,
        BookKind::IntermediateTrade => 2,
        BookKind::Indicative => 3,
        BookKind::Delay(DelayDirection::Up) => 4,
        BookKind::Delay(DelayDirection::Down) => 5,
    }
}

fn kind_of(code: u8) -> BookKind {
    match code {
        0 => BookKind::Disclosure { has_trade: false },
        1 => BookKind::Disclosure { has_trade: true },
        2 => BookKind::IntermediateTrade,
        3 => BookKind::Indicative,
        4 => BookKind::Delay(DelayDirection::Up),
        5 => BookKind::Delay(DelayDirection::Down),
        _ => unreachable!("the stream was written by `encode`"),
    }
}

const fn phase_code(phase: Phase) -> u8 {
    match phase {
        Phase::OpenAuction => 0,
        Phase::Continuous => 1,
        Phase::PeriodicAuction => 2,
        Phase::CloseAuction => 3,
    }
}

const fn phase_of(code: u8) -> Phase {
    match code {
        0 => Phase::OpenAuction,
        1 => Phase::Continuous,
        2 => Phase::PeriodicAuction,
        _ => Phase::CloseAuction,
    }
}

/// Whether rows of this kind disclose a book.
const fn has_book(kind: BookKind) -> bool {
    matches!(kind, BookKind::Disclosure { .. } | BookKind::Indicative)
}

const fn flag_code(flag: Option<bool>) -> u8 {
    match flag {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

const fn flag_of(code: u8) -> Option<bool> {
    match code & 0b11 {
        0 => None,
        1 => Some(false),
        _ => Some(true),
    }
}

pub(crate) fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        #[allow(clippy::cast_possible_truncation)]
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    #[allow(clippy::cast_possible_truncation)]
    out.push(value as u8);
}

#[allow(clippy::cast_sign_loss)]
fn put_signed(out: &mut Vec<u8>, value: i64) {
    put_varint(out, ((value << 1) ^ (value >> 63)) as u64);
}

pub(crate) fn varint(bytes: &[u8], at: &mut usize) -> u64 {
    let mut value = 0;
    let mut shift = 0;
    loop {
        let byte = bytes[*at];
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

#[allow(clippy::cast_possible_wrap)]
fn signed(bytes: &[u8], at: &mut usize) -> i64 {
    let value = varint(bytes, at);
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

/// A `u32` moved by a difference `encode_side` wrote.
fn moved(from: u32, by: i64) -> u32 {
    u32::try_from(i64::from(from) + by).expect("the stream was written by `encode`")
}

fn encode_side(out: &mut Vec<u8>, before: &CompactSide, side: &CompactSide) {
    let len = usize::from(side.len);
    let mut head = u16::from(side.len);
    if side.market_qty != before.market_qty {
        head |= MARKET_CHANGED;
    }
    for level in 0..len {
        if side.price[level] != before.price[level] {
            head |= 1 << (PRICE_SHIFT + level as u16);
        }
        if side.qty[level] != before.qty[level] {
            head |= 1 << (QTY_SHIFT + level as u16);
        }
    }
    out.extend_from_slice(&head.to_le_bytes());
    if head & MARKET_CHANGED != 0 {
        put_varint(out, u64::from(side.market_qty));
    }
    for level in 0..len {
        if side.price[level] != before.price[level] {
            put_signed(
                out,
                i64::from(side.price[level]) - i64::from(before.price[level]),
            );
        }
        if side.qty[level] != before.qty[level] {
            put_signed(
                out,
                i64::from(side.qty[level]) - i64::from(before.qty[level]),
            );
        }
    }
}

fn decode_side(bytes: &[u8], at: &mut usize, side: &mut CompactSide) {
    let head = u16::from_le_bytes([bytes[*at], bytes[*at + 1]]);
    *at += 2;
    let len = usize::from(head & LEN_MASK);
    if head & MARKET_CHANGED != 0 {
        side.market_qty =
            u32::try_from(varint(bytes, at)).expect("the stream was written by `encode`");
    }
    for level in 0..len {
        if head & (1 << (PRICE_SHIFT + level as u16)) != 0 {
            side.price[level] = moved(side.price[level], signed(bytes, at));
        }
        if head & (1 << (QTY_SHIFT + level as u16)) != 0 {
            side.qty[level] = moved(side.qty[level], signed(bytes, at));
        }
    }
    for level in len..BOOK_DEPTH {
        side.price[level] = 0;
        side.qty[level] = 0;
    }
    #[allow(clippy::cast_possible_truncation)]
    {
        side.len = len as u8;
    }
}

/// Appends `row` to a symbol's stream. `frame` is the state after the symbol's previous
/// row and becomes the state after this one; with `reset` the row is written so that it
/// can be read from [`Frame::START`].
///
/// `delay_open` and `delay_close` are kept only for indicative rows, the only ones
/// whose message carries them.
pub(crate) fn encode(out: &mut Vec<u8>, frame: &mut Frame, reset: bool, row: &BookRow) {
    if reset {
        *frame = Frame::START;
    }
    let book = has_book(row.kind);
    let mut tag = kind_code(row.kind) | (phase_code(row.phase) << PHASE_SHIFT);
    if reset {
        tag |= RESET;
    }
    if book && row.bid == frame.bid {
        tag |= SAME_BID;
    }
    if book && row.ask == frame.ask {
        tag |= SAME_ASK;
    }
    out.push(tag);
    put_signed(out, row.exch_ts.wrapping_sub(frame.exch_ts));
    frame.exch_ts = row.exch_ts;
    if !book {
        return;
    }
    if tag & SAME_BID == 0 {
        encode_side(out, &frame.bid, &row.bid);
        frame.bid = row.bid;
    }
    if tag & SAME_ASK == 0 {
        encode_side(out, &frame.ask, &row.ask);
        frame.ask = row.ask;
    }
    if row.kind == BookKind::Indicative {
        out.push(flag_code(row.delay_open) | (flag_code(row.delay_close) << 2));
        put_varint(out, u64::from(row.indicative_price));
        if row.indicative_price != 0 {
            put_varint(out, u64::from(row.indicative_qty));
        }
    }
}

/// Reads the row at `*at` of a symbol's stream and moves `*at` and `frame` past it.
pub(crate) fn decode(bytes: &[u8], at: &mut usize, frame: &mut Frame) -> Decoded {
    let tag = bytes[*at];
    *at += 1;
    if tag & RESET != 0 {
        *frame = Frame::START;
    }
    let kind = kind_of(tag & KIND_MASK);
    let mut decoded = Decoded {
        phase: phase_of((tag & PHASE_MASK) >> PHASE_SHIFT),
        kind,
        indicative_price: 0,
        indicative_qty: 0,
        delay_open: None,
        delay_close: None,
    };
    frame.exch_ts = frame.exch_ts.wrapping_add(signed(bytes, at));
    if !has_book(kind) {
        return decoded;
    }
    if tag & SAME_BID == 0 {
        decode_side(bytes, at, &mut frame.bid);
    }
    if tag & SAME_ASK == 0 {
        decode_side(bytes, at, &mut frame.ask);
    }
    if kind == BookKind::Indicative {
        let flags = bytes[*at];
        *at += 1;
        decoded.delay_open = flag_of(flags);
        decoded.delay_close = flag_of(flags >> 2);
        let price = varint(bytes, at);
        decoded.indicative_price =
            u32::try_from(price).expect("the stream was written by `encode`");
        if price != 0 {
            decoded.indicative_qty =
                u32::try_from(varint(bytes, at)).expect("the stream was written by `encode`");
        }
    }
    decoded
}

#[cfg(test)]
mod tests {
    use osmium_types::{BOOK_DEPTH, DelayDirection, Phase};

    use super::{Decoded, Frame, decode, encode};
    use crate::decode::{BookKind, BookRow, CompactSide};

    fn side(levels: &[(u32, u32)], market_qty: u32) -> CompactSide {
        let mut side = CompactSide {
            market_qty,
            ..CompactSide::EMPTY
        };
        for (level, &(price, qty)) in levels.iter().enumerate() {
            side.price[level] = price;
            side.qty[level] = qty;
        }
        side.len = u8::try_from(levels.len()).unwrap();
        side
    }

    fn row(kind: BookKind, exch_ts: i64, bid: CompactSide, ask: CompactSide) -> BookRow {
        BookRow {
            exch_ts,
            phase: Phase::Continuous,
            kind,
            bid,
            ask,
            indicative_price: 0,
            indicative_qty: 0,
            delay_open: None,
            delay_close: None,
        }
    }

    /// What `decode` must give back for `row`.
    fn expected(row: &BookRow) -> (Decoded, i64, Option<(CompactSide, CompactSide)>) {
        let indicative = row.kind == BookKind::Indicative;
        let book = matches!(row.kind, BookKind::Disclosure { .. } | BookKind::Indicative);
        (
            Decoded {
                phase: row.phase,
                kind: row.kind,
                indicative_price: row.indicative_price,
                indicative_qty: row.indicative_qty,
                delay_open: row.delay_open.filter(|_| indicative),
                delay_close: row.delay_close.filter(|_| indicative),
            },
            row.exch_ts,
            book.then_some((row.bid, row.ask)),
        )
    }

    fn rows() -> Vec<BookRow> {
        let disclosure = BookKind::Disclosure { has_trade: false };
        let traded = BookKind::Disclosure { has_trade: true };
        let bid = side(&[(1_000_000, 7), (995_000, 3), (990_000, 12)], 0);
        let ask = side(&[(1_005_000, 3), (1_010_000, 4_000_000_000)], 0);
        let far = side(
            &[
                (u32::MAX, u32::MAX),
                (4_000_000_000, 1),
                (3, 2),
                (2, u32::MAX),
                (1, 1),
            ],
            u32::MAX,
        );
        let mut indicative = row(BookKind::Indicative, 1_760_000_000_000_500, bid, far);
        indicative.phase = Phase::OpenAuction;
        indicative.indicative_price = 1_000_000;
        indicative.indicative_qty = 42;
        indicative.delay_open = Some(true);
        indicative.delay_close = Some(false);
        let mut unmatched = row(BookKind::Indicative, 1_760_000_000_000_400, far, bid);
        unmatched.phase = Phase::CloseAuction;
        let mut odd = row(disclosure, 1_760_000_000_000_900, bid, ask);
        odd.phase = Phase::PeriodicAuction;
        // Never read back: only indicative rows keep these.
        odd.delay_open = Some(false);
        vec![
            row(disclosure, 1_760_000_000_000_000, bid, ask),
            // Nothing but the time changes.
            row(disclosure, 1_760_000_000_000_100, bid, ask),
            // One quantity changes; then the side loses a level and gains market orders.
            row(
                traded,
                1_760_000_000_000_100,
                side(&[(1_000_000, 6)], 0),
                ask,
            ),
            row(
                disclosure,
                1_760_000_000_000_050,
                side(&[(1_000_000, 6)], 9),
                ask,
            ),
            row(
                BookKind::IntermediateTrade,
                1_760_000_000_000_200,
                CompactSide::EMPTY,
                CompactSide::EMPTY,
            ),
            row(
                BookKind::Delay(DelayDirection::Up),
                1_760_000_000_000_200,
                CompactSide::EMPTY,
                CompactSide::EMPTY,
            ),
            row(
                BookKind::Delay(DelayDirection::Down),
                1_760_000_000_000_300,
                CompactSide::EMPTY,
                CompactSide::EMPTY,
            ),
            // The book after rows without one builds on the last disclosed book.
            row(
                disclosure,
                1_760_000_000_000_300,
                side(&[(1_000_000, 6)], 9),
                far,
            ),
            indicative,
            unmatched,
            row(
                traded,
                1_760_000_000_000_600,
                CompactSide::EMPTY,
                CompactSide::EMPTY,
            ),
            odd,
            row(disclosure, i64::MIN, far, far),
            row(disclosure, i64::MAX, bid, CompactSide::EMPTY),
        ]
    }

    fn read_back(bytes: &[u8], rows: &[BookRow]) {
        let (mut at, mut frame) = (0, Frame::START);
        for row in rows {
            let decoded = decode(bytes, &mut at, &mut frame);
            let (wanted, exch_ts, book) = expected(row);
            assert_eq!(decoded, wanted);
            assert_eq!(frame.exch_ts, exch_ts);
            if let Some((bid, ask)) = book {
                assert_eq!((frame.bid, frame.ask), (bid, ask));
            }
        }
        assert_eq!(at, bytes.len());
    }

    #[test]
    fn rows_read_back_as_they_were_written() {
        let rows = rows();
        let (mut bytes, mut frame) = (Vec::new(), Frame::START);
        for row in &rows {
            encode(&mut bytes, &mut frame, false, row);
        }
        read_back(&bytes, &rows);
    }

    #[test]
    fn a_reset_row_does_not_depend_on_the_rows_before_it() {
        let rows = rows();
        for split in 0..rows.len() {
            let (mut bytes, mut frame) = (Vec::new(), Frame::START);
            for (index, row) in rows.iter().enumerate() {
                if index == split {
                    // Written the way another thread would write it: from no state.
                    let mut fresh = Frame {
                        exch_ts: 77,
                        bid: rows[0].ask,
                        ask: rows[0].bid,
                    };
                    encode(&mut bytes, &mut fresh, true, row);
                    frame = fresh;
                } else {
                    encode(&mut bytes, &mut frame, false, row);
                }
            }
            read_back(&bytes, &rows);
        }
    }

    #[test]
    fn a_row_that_repeats_the_book_takes_a_few_bytes() {
        let rows = rows();
        let (mut bytes, mut frame) = (Vec::new(), Frame::START);
        encode(&mut bytes, &mut frame, false, &rows[0]);
        let first = bytes.len();
        encode(&mut bytes, &mut frame, false, &rows[1]);
        // The tag and a 100-microsecond step.
        assert_eq!(bytes.len() - first, 3);
        assert!(first > 3 * BOOK_DEPTH);
    }
}
