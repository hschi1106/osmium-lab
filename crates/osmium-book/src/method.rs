use osmium_types::{Board, Message, MessageBody, Phase, Timestamp};

/// How a book matches orders between its opening and closing auctions. The exchange
/// fixes it per instrument before the trading day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// Each order is matched as it arrives.
    Continuous,
    /// Orders are collected and matched in call auctions at intervals.
    Call,
}

impl Method {
    /// The method that the board and `instruments.match_interval_s` determine, or `None`
    /// when the interval is missing for a regular-board book; [`MethodEvidence`] then
    /// decides.
    #[must_use]
    pub const fn known(board: Board, match_interval_s: Option<u32>) -> Option<Self> {
        match (board, match_interval_s) {
            (Board::OddLot, _) => Some(Self::Call),
            (Board::Regular, Some(0)) => Some(Self::Continuous),
            (Board::Regular, Some(_)) => Some(Self::Call),
            (Board::Regular, None) => None,
        }
    }
}

/// Infers a regular-board book's [`Method`] from all of its messages of one day, for
/// days on which marketdata has no `match_interval_s`.
///
/// The book matches by call auction when it has messages outside continuous trading
/// between its opening auction result and the start of closing-auction order
/// collection, and none in continuous trading all day. The method is a property the
/// exchange announces beforehand, so using the whole day does not look ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MethodEvidence {
    opened: bool,
    call_after_open: bool,
    continuous: bool,
}

impl MethodEvidence {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            opened: false,
            call_after_open: false,
            continuous: false,
        }
    }

    /// Records one message of the book. `close_collection` is when order collection
    /// for the closing auction starts.
    pub fn observe(&mut self, message: &Message, close_collection: Timestamp) {
        if message.phase == Phase::Continuous {
            self.continuous = true;
        } else if self.opened && message.exch_ts < close_collection {
            self.call_after_open = true;
        }
        if message.phase == Phase::OpenAuction
            && matches!(message.body, MessageBody::Disclosure { .. })
        {
            self.opened = true;
        }
    }

    #[must_use]
    pub const fn method(&self) -> Method {
        if self.call_after_open && !self.continuous {
            Method::Call
        } else {
            Method::Continuous
        }
    }
}

#[cfg(test)]
mod tests {
    use osmium_types::{Board, Book, DelayDirection, MessageBody, Phase, Timestamp};

    use super::{Method, MethodEvidence};
    use crate::BookState;
    use crate::state::tests::{at, book, date, disclosure, message};

    fn infer(messages: &[(Timestamp, Phase, MessageBody)]) -> Method {
        let mut evidence = MethodEvidence::new();
        for &(time, phase, body) in messages {
            evidence.observe(
                &message(time, phase, body),
                BookState::close_collection(date()),
            );
        }
        evidence.method()
    }

    fn indicative() -> MessageBody {
        MessageBody::Indicative {
            book: Book::EMPTY,
            matched: None,
            delay_open: None,
            delay_close: None,
        }
    }

    #[test]
    fn the_board_and_the_match_interval_decide_the_method_when_known() {
        assert_eq!(Method::known(Board::OddLot, None), Some(Method::Call));
        assert_eq!(Method::known(Board::OddLot, Some(0)), Some(Method::Call));
        assert_eq!(
            Method::known(Board::Regular, Some(0)),
            Some(Method::Continuous)
        );
        assert_eq!(Method::known(Board::Regular, Some(5)), Some(Method::Call));
        assert_eq!(
            Method::known(Board::Regular, Some(2700)),
            Some(Method::Call)
        );
        assert_eq!(Method::known(Board::Regular, None), None);
    }

    #[test]
    fn a_book_with_continuous_messages_is_continuous() {
        let quiet = book("100", "101");
        assert_eq!(
            infer(&[
                (at(8, 30, 0), Phase::OpenAuction, indicative()),
                (at(9, 0, 0), Phase::OpenAuction, disclosure(quiet, None)),
                (at(9, 0, 1), Phase::Continuous, disclosure(quiet, None)),
                // A delay puts auction messages between the open and 13:25.
                (
                    at(9, 1, 0),
                    Phase::Continuous,
                    MessageBody::Delay(DelayDirection::Up)
                ),
                (at(9, 1, 5), Phase::PeriodicAuction, indicative()),
                (at(9, 3, 0), Phase::PeriodicAuction, disclosure(quiet, None)),
                (at(13, 25, 0), Phase::CloseAuction, indicative()),
                (at(13, 30, 0), Phase::CloseAuction, disclosure(quiet, None)),
            ]),
            Method::Continuous
        );
    }

    #[test]
    fn auction_messages_after_the_open_without_continuous_ones_mean_call() {
        let quiet = book("100", "101");
        assert_eq!(
            infer(&[
                (at(8, 30, 0), Phase::OpenAuction, indicative()),
                (at(9, 0, 0), Phase::OpenAuction, disclosure(quiet, None)),
                (at(9, 5, 0), Phase::PeriodicAuction, disclosure(quiet, None)),
                (at(13, 30, 0), Phase::CloseAuction, disclosure(quiet, None)),
            ]),
            Method::Call
        );
        // An indicative message between the auctions is evidence as well.
        assert_eq!(
            infer(&[
                (at(9, 0, 0), Phase::OpenAuction, disclosure(quiet, None)),
                (at(9, 3, 0), Phase::PeriodicAuction, indicative()),
            ]),
            Method::Call
        );
    }

    #[test]
    fn a_book_that_is_silent_between_its_auctions_is_continuous() {
        let quiet = book("100", "101");
        assert_eq!(infer(&[]), Method::Continuous);
        assert_eq!(
            infer(&[
                (at(8, 30, 0), Phase::OpenAuction, indicative()),
                (at(8, 59, 0), Phase::OpenAuction, indicative()),
                (at(9, 0, 0), Phase::OpenAuction, disclosure(quiet, None)),
                // Closing-auction collection is not evidence of call matching.
                (at(13, 25, 0), Phase::CloseAuction, indicative()),
                (at(13, 30, 0), Phase::CloseAuction, disclosure(quiet, None)),
            ]),
            Method::Continuous
        );
    }

    #[test]
    fn messages_before_the_opening_result_are_not_evidence() {
        let quiet = book("100", "101");
        // A delayed open: indicative messages continue past 09:00 until the result.
        assert_eq!(
            infer(&[
                (at(9, 1, 0), Phase::OpenAuction, indicative()),
                (at(9, 3, 0), Phase::OpenAuction, indicative()),
                (at(9, 5, 0), Phase::OpenAuction, disclosure(quiet, None)),
            ]),
            Method::Continuous
        );
    }
}
