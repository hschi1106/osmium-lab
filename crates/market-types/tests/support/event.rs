use market_types::{BookSide, BookSideKind, CompleteBookSnapshot};

pub(crate) fn empty_book() -> CompleteBookSnapshot {
    CompleteBookSnapshot::new(
        BookSide::new(BookSideKind::Bid, vec![]).unwrap(),
        BookSide::new(BookSideKind::Ask, vec![]).unwrap(),
    )
    .unwrap()
}
