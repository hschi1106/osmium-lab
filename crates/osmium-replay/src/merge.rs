use std::collections::VecDeque;

use osmium_types::{Feed, Message, Timestamp};

/// The position of a message in the total order of one trading day.
///
/// Messages compare by observed time, then feed rank, then `seq`. Within one feed
/// that is the order the exchange sent them; feeds share no sequence, so exchange
/// time is the only evidence of order between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageOrder {
    pub observed_time: Timestamp,
    pub feed_rank: u8,
    pub seq: u64,
}

impl MessageOrder {
    #[must_use]
    pub const fn of(message: &Message) -> Self {
        Self {
            observed_time: message.observed_time,
            feed_rank: message.feed.rank(),
            seq: message.seq,
        }
    }
}

struct Stream<I> {
    head: Option<Message>,
    rest: I,
    feed: Option<Feed>,
}

impl<I: Iterator<Item = Message>> Stream<I> {
    /// Moves to the message after the head, which must follow it in the same feed.
    fn advance(&mut self) {
        let Some(taken) = self
            .head
            .as_ref()
            .map(|taken| (taken.feed, MessageOrder::of(taken)))
        else {
            return;
        };
        self.head = self.rest.next();
        if let Some(next) = &self.head {
            let (feed, taken) = taken;
            assert_eq!(next.feed, feed, "a stream carries one feed");
            assert!(
                next.seq > taken.seq && next.observed_time >= taken.observed_time,
                "{} messages are not in exchange order: seq {} follows seq {}",
                feed.part(),
                next.seq,
                taken.seq
            );
        }
    }
}

/// Merges the message streams of several feeds into the day's total order.
///
/// Each stream is one feed's messages in the order the exchange sent them, as
/// `osmium_md::FeedDay::messages` produces.
pub struct MergedFeeds<I> {
    streams: Vec<Stream<I>>,
    /// The stream whose head is the next message.
    next: Option<usize>,
}

impl<I: Iterator<Item = Message>> MergedFeeds<I> {
    /// # Panics
    ///
    /// Panics when two streams carry the same feed. Iteration panics when a stream
    /// changes feed or is not in exchange order.
    pub fn new(streams: impl IntoIterator<Item = I>) -> Self {
        let streams: Vec<Stream<I>> = streams
            .into_iter()
            .map(|mut rest| {
                let head = rest.next();
                Stream {
                    feed: head.as_ref().map(|message| message.feed),
                    head,
                    rest,
                }
            })
            .collect();
        for (index, stream) in streams.iter().enumerate() {
            if let Some(feed) = stream.feed {
                assert!(
                    streams[..index]
                        .iter()
                        .all(|other| other.feed != Some(feed)),
                    "{} is merged twice",
                    feed.part()
                );
            }
        }
        let mut merged = Self {
            streams,
            next: None,
        };
        merged.next = merged.next_stream();
        merged
    }

    /// The next message without consuming it.
    #[must_use]
    pub fn peek(&self) -> Option<&Message> {
        self.next
            .and_then(|index| self.streams[index].head.as_ref())
    }

    fn next_stream(&self) -> Option<usize> {
        self.streams
            .iter()
            .enumerate()
            .filter_map(|(index, stream)| Some((MessageOrder::of(stream.head.as_ref()?), index)))
            .min()
            .map(|(_, index)| index)
    }

    /// Moves the next message to the back of `queue`; `false` when none is left.
    ///
    /// A message is large; this copies it once, where taking it out of the iterator
    /// and pushing it would copy it several times.
    pub(crate) fn pop_into(&mut self, queue: &mut VecDeque<Message>) -> bool {
        let Some(index) = self.next else {
            return false;
        };
        let stream = &mut self.streams[index];
        let Some(message) = &stream.head else {
            return false;
        };
        queue.push_back(*message);
        stream.advance();
        self.next = self.next_stream();
        true
    }
}

impl<I: Iterator<Item = Message>> Iterator for MergedFeeds<I> {
    type Item = Message;

    fn next(&mut self) -> Option<Message> {
        let stream = &mut self.streams[self.next?];
        let message = stream.head?;
        stream.advance();
        self.next = self.next_stream();
        Some(message)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use osmium_types::{DelayDirection, Feed, Message, MessageBody, Phase, Symbol, Timestamp};

    use super::{MergedFeeds, MessageOrder};

    /// A message whose only meaningful fields are its feed, seq and observed time.
    pub(crate) fn message(feed: Feed, seq: u64, observed: i64) -> Message {
        Message {
            feed,
            seq,
            symbol: Symbol::new("2330").unwrap(),
            exch_ts: Timestamp::from_micros(observed),
            observed_time: Timestamp::from_micros(observed),
            phase: Phase::Continuous,
            body: MessageBody::Delay(DelayDirection::Up),
        }
    }

    fn merged(streams: Vec<Vec<Message>>) -> Vec<(Feed, u64)> {
        MergedFeeds::new(streams.into_iter().map(Vec::into_iter))
            .map(|message| (message.feed, message.seq))
            .collect()
    }

    #[test]
    fn feeds_merge_by_observed_time_then_rank_then_seq() {
        let order = merged(vec![
            // Listed against rank order on purpose.
            vec![
                message(Feed::TwseF23, 1, 10),
                message(Feed::TwseF23, 2, 20),
                message(Feed::TwseF23, 3, 20),
            ],
            vec![message(Feed::TpexF6, 7, 20), message(Feed::TpexF6, 9, 30)],
            vec![
                message(Feed::TwseF6, 5, 15),
                message(Feed::TwseF6, 6, 20),
                message(Feed::TwseF6, 8, 20),
            ],
            vec![],
        ]);
        assert_eq!(
            order,
            [
                (Feed::TwseF23, 1),
                (Feed::TwseF6, 5),
                // At time 20: every TWSE_F6 message, then TPEX_F6, then TWSE_F23.
                (Feed::TwseF6, 6),
                (Feed::TwseF6, 8),
                (Feed::TpexF6, 7),
                (Feed::TwseF23, 2),
                (Feed::TwseF23, 3),
                (Feed::TpexF6, 9),
            ]
        );
    }

    #[test]
    fn peek_shows_the_next_message() {
        let mut feeds = MergedFeeds::new([
            vec![message(Feed::TpexF6, 1, 10)].into_iter(),
            vec![message(Feed::TwseF6, 4, 10)].into_iter(),
        ]);
        assert_eq!(feeds.peek().map(|message| message.feed), Some(Feed::TwseF6));
        assert_eq!(feeds.next().map(|message| message.feed), Some(Feed::TwseF6));
        assert_eq!(feeds.peek().map(|message| message.feed), Some(Feed::TpexF6));
        assert!(feeds.next().is_some());
        assert!(feeds.peek().is_none() && feeds.next().is_none());
    }

    #[test]
    fn order_compares_time_before_rank_before_seq() {
        let early_low_rank = MessageOrder::of(&message(Feed::TpexF23, 9, 1));
        let late = MessageOrder::of(&message(Feed::TwseF6, 1, 2));
        let late_next_feed = MessageOrder::of(&message(Feed::TpexF6, 1, 2));
        assert!(early_low_rank < late && late < late_next_feed);
    }

    #[test]
    #[should_panic(expected = "TWSE_F6 is merged twice")]
    fn a_feed_cannot_be_merged_twice() {
        let _ = MergedFeeds::new([
            vec![message(Feed::TwseF6, 1, 10)].into_iter(),
            vec![message(Feed::TwseF6, 2, 10)].into_iter(),
        ]);
    }

    #[test]
    #[should_panic(expected = "not in exchange order")]
    fn a_stream_out_of_exchange_order_is_rejected() {
        let _ = merged(vec![vec![
            message(Feed::TwseF6, 2, 10),
            message(Feed::TwseF6, 1, 10),
        ]]);
    }

    #[test]
    #[should_panic(expected = "not in exchange order")]
    fn observed_time_cannot_go_back_within_a_feed() {
        let _ = merged(vec![vec![
            message(Feed::TwseF6, 1, 10),
            message(Feed::TwseF6, 2, 9),
        ]]);
    }
}
