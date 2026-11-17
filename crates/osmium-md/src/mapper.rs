//! Maps one symbol's marketdata messages to `DomainEvent`s.
//!
//! The mapping never guesses: evidence the feed does not carry stays `NoObservation`,
//! and evidence that is ambiguous for this row becomes `Unknown`.

use market_types::{
    AuctionEvidence, AuctionObservation, BOOK_DEPTH, BookLevel, BookSide, BookSideKind,
    BookSnapshot, CompleteBookSnapshot, Decimal, DomainEvent, EventPayload, IndicativeAuction,
    InstrumentId, MarketAnnotations, MarketSignal, MarketStatusObservation, MatchTime, Observation,
    ObservedTrade, Price, Quantity, QuantityUnit, QuoteSnapshot, SourceFormatId, Symbol,
    TradeBatch, TradeBatchOrdering, TradeObservationKind, TradingDate, VolatilityDirection, Volume,
};

use crate::{
    Board, Feed, MdError,
    rows::{BookRow, Phase, SideRow, TICKS_PER_UNIT, TradeRow},
};

/// Decimal atoms per price tick.
const ATOMS_PER_TICK: i128 = Decimal::SCALE_FACTOR / TICKS_PER_UNIT as i128;

/// One exchange message: a book disclosure (with its trade when `has_trade`), or an
/// intermediate trade that carried no book.
// Book messages dominate (~99% of rows), so boxing the smaller variant saves nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Message {
    Book(BookRow, Option<TradeRow>),
    TradeOnly(TradeRow),
}

impl Message {
    pub(crate) const fn seq(&self) -> u64 {
        match self {
            Self::Book(book, _) => book.seq,
            Self::TradeOnly(trade) => trade.seq,
        }
    }

    const fn exch_ts(&self) -> i64 {
        match self {
            Self::Book(book, _) => book.exch_ts,
            Self::TradeOnly(trade) => trade.exch_ts,
        }
    }
}

/// Merges one symbol's book and trade rows (each in seq order) into messages.
pub(crate) fn merge_messages(
    symbol: Symbol,
    books: Vec<BookRow>,
    trades: Vec<TradeRow>,
) -> Result<Vec<Message>, MdError> {
    let mut messages = Vec::with_capacity(books.len() + trades.len());
    let mut trades = trades.into_iter().peekable();
    for book in books {
        while let Some(trade) = trades.next_if(|trade| trade.seq < book.seq) {
            messages.push(Message::TradeOnly(trade));
        }
        let trade = trades.next_if(|trade| trade.seq == book.seq);
        if book.has_trade != trade.is_some() {
            return Err(MdError::InvalidRow {
                symbol,
                seq: book.seq,
                reason: "has_trade disagrees with the trades table",
            });
        }
        messages.push(Message::Book(book, trade));
    }
    messages.extend(trades.map(Message::TradeOnly));
    Ok(messages)
}

/// Per-symbol mapping state.
#[derive(Debug, Clone)]
pub(crate) struct SymbolMapper {
    instrument: InstrumentId,
    trading_date: TradingDate,
    source_format: SourceFormatId,
    board: Board,
    unit: QuantityUnit,
    /// Matching cycle from `instruments`; `None` when the instrument master lacks it.
    match_interval_s: Option<i32>,
    last_exch_ts: Option<i64>,
    /// Direction of the volatility interruption in progress, if any.
    volatility: Option<VolatilityDirection>,
    /// Whether the current periodic call-auction round was delayed.
    periodic_delayed: bool,
}

impl SymbolMapper {
    pub(crate) fn new(
        feed: Feed,
        trading_date: TradingDate,
        symbol: Symbol,
        match_interval_s: Option<i32>,
    ) -> Self {
        Self {
            instrument: InstrumentId::new(feed.market(), symbol),
            trading_date,
            source_format: feed.source_format(),
            board: feed.board(),
            unit: feed.quantity_unit(),
            match_interval_s,
            last_exch_ts: None,
            volatility: None,
            periodic_delayed: false,
        }
    }

    pub(crate) fn map(
        &mut self,
        message: &Message,
        observed_time: i64,
    ) -> Result<DomainEvent, MdError> {
        let seq = message.seq();
        let exch_ts = message.exch_ts();
        if self.last_exch_ts.is_some_and(|last| exch_ts < last) {
            return Err(self.invalid(seq, "exch_ts regressed within the symbol"));
        }
        self.last_exch_ts = Some(exch_ts);
        let payload = match message {
            Message::TradeOnly(trade) => self.intermediate_trade(trade)?,
            Message::Book(book, trade) if book.delay_state != 0 => {
                self.delay(book, trade.as_ref())?
            }
            Message::Book(book, _) if book.is_indicative => self.indicative(book)?,
            Message::Book(book, trade) => self.disclosure(book, trade.as_ref())?,
        };
        DomainEvent::new(
            self.instrument,
            self.trading_date,
            self.source_format,
            MatchTime::from_unix_microseconds(exch_ts),
            Some(seq),
            payload,
        )
        .with_observed_time(MatchTime::from_unix_microseconds(observed_time))
        .map_err(|_| self.invalid(seq, "observed time precedes exch_ts"))
    }

    fn intermediate_trade(&mut self, trade: &TradeRow) -> Result<EventPayload, MdError> {
        if trade.phase != Phase::Continuous {
            return Err(self.invalid(trade.seq, "trade without a book outside continuous trading"));
        }
        self.enter_continuous();
        let batch = TradeBatch::new(
            vec![self.trade(trade, TradeObservationKind::Intermediate)?],
            TradeBatchOrdering::SourceSequencePreserved,
            self.cumulative(trade),
            MarketAnnotations::None,
        )
        .map_err(|_| self.invalid(trade.seq, "trade batch units disagree"))?
        .with_market_signal(Observation::Set(MarketSignal::Continuous));
        Ok(EventPayload::TradeBatch(batch))
    }

    /// A delay row (暫緩撮合) discloses no book; it starts or extends a delayed auction.
    fn delay(&mut self, book: &BookRow, trade: Option<&TradeRow>) -> Result<EventPayload, MdError> {
        if trade.is_some() || book.bid.is_some() || book.ask.is_some() || book.is_indicative {
            return Err(self.invalid(book.seq, "delay row carries a book, trade or indicative"));
        }
        let direction = if book.delay_state > 0 {
            VolatilityDirection::Up
        } else {
            VolatilityDirection::Down
        };
        let observation = match (book.phase, self.periodic()) {
            (Phase::OpenAuction | Phase::CloseAuction, _) => {
                return Err(self.invalid(book.seq, "delay row in an opening or closing auction"));
            }
            (Phase::PeriodicAuction, Some(true)) => {
                self.periodic_delayed = true;
                AuctionObservation::periodic_with_evidence(
                    AuctionEvidence::Known(true),
                    AuctionEvidence::NoObservation,
                )
            }
            (Phase::Continuous, Some(true)) => {
                return Err(self.invalid(book.seq, "continuous phase on a periodic instrument"));
            }
            (Phase::Continuous | Phase::PeriodicAuction, Some(false)) => {
                self.volatility = Some(direction);
                Self::volatility_interruption(direction)
            }
            (_, None) => AuctionObservation::unknown_purpose(),
        };
        let status =
            MarketStatusObservation::new(Observation::NoObservation, MarketAnnotations::None)
                .with_market_signal(Observation::Set(MarketSignal::AuctionCollecting(
                    observation,
                )));
        Ok(EventPayload::MarketStatus(status))
    }

    fn indicative(&mut self, book: &BookRow) -> Result<EventPayload, MdError> {
        let observation = self.auction_observation(book)?;
        let price = match book.indicative_price {
            Some(ticks) => Observation::Set(self.price(book.seq, ticks)?),
            None => Observation::Clear,
        };
        let quantity = match book.indicative_quantity {
            Some(value) => Observation::Set(self.quantity(book.seq, value)?),
            None => Observation::Clear,
        };
        let auction = IndicativeAuction::new(
            observation,
            price,
            quantity,
            Observation::Set(self.book(book)?),
            Observation::NoObservation,
            MarketAnnotations::None,
        )
        .map_err(|_| self.invalid(book.seq, "indicative auction units disagree"))?;
        Ok(EventPayload::IndicativeAuction(auction))
    }

    /// A firm book disclosure, with its trade when the disclosure was caused by one.
    fn disclosure(
        &mut self,
        book: &BookRow,
        trade: Option<&TradeRow>,
    ) -> Result<EventPayload, MdError> {
        if let Some(trade) = trade
            && trade.phase != book.phase
        {
            return Err(self.invalid(book.seq, "trade and book phases disagree"));
        }
        let signal = if book.phase == Phase::Continuous {
            self.enter_continuous();
            MarketSignal::Continuous
        } else {
            let observation = self.auction_observation(book)?;
            // The uncross ends the round.
            if book.phase == Phase::PeriodicAuction {
                self.periodic_delayed = false;
            }
            MarketSignal::AuctionUncross(observation)
        };
        let snapshot = self.book(book)?;
        let payload = match trade {
            Some(trade) => EventPayload::QuoteSnapshot(
                QuoteSnapshot::new(
                    snapshot,
                    Observation::Set(self.trade(trade, TradeObservationKind::Regular)?),
                    self.cumulative(trade),
                    MarketAnnotations::None,
                )
                .map_err(|_| self.invalid(book.seq, "quote units disagree"))?
                .with_market_signal(Observation::Set(signal)),
            ),
            None => EventPayload::BookSnapshot(
                BookSnapshot::new(snapshot, MarketAnnotations::None)
                    .with_market_signal(Observation::Set(signal)),
            ),
        };
        Ok(payload)
    }

    fn auction_observation(&self, book: &BookRow) -> Result<AuctionObservation, MdError> {
        Ok(match book.phase {
            Phase::OpenAuction => AuctionObservation::opening_with_evidence(
                self.delay_flag(book.delay_open),
                AuctionEvidence::NoObservation,
            ),
            Phase::CloseAuction => AuctionObservation::closing_with_evidence(
                self.delay_flag(book.delay_close),
                AuctionEvidence::NoObservation,
            ),
            Phase::PeriodicAuction => match (self.periodic(), self.volatility) {
                (Some(true), _) => AuctionObservation::periodic_with_evidence(
                    AuctionEvidence::Known(self.periodic_delayed),
                    AuctionEvidence::NoObservation,
                ),
                (Some(false), Some(direction)) => Self::volatility_interruption(direction),
                (Some(false), None) | (None, _) => AuctionObservation::unknown_purpose(),
            },
            Phase::Continuous => {
                return Err(self.invalid(book.seq, "indicative row in continuous trading"));
            }
        })
    }

    /// Whether this book matches by periodic call auction; `None` when unknown.
    fn periodic(&self) -> Option<bool> {
        match self.board {
            Board::OddLot => Some(true),
            Board::Regular => self.match_interval_s.map(|seconds| seconds > 0),
        }
    }

    /// Opening／closing delay flags exist only on the regular board; odd-lot rows
    /// carry `false` for "not disclosed", and partial (S3) days carry null.
    fn delay_flag(&self, flag: Option<bool>) -> AuctionEvidence<bool> {
        match (self.board, flag) {
            (Board::OddLot, _) => AuctionEvidence::NoObservation,
            (Board::Regular, Some(flag)) => AuctionEvidence::Known(flag),
            (Board::Regular, None) => AuctionEvidence::Unknown,
        }
    }

    const fn volatility_interruption(direction: VolatilityDirection) -> AuctionObservation {
        AuctionObservation::volatility_interruption_with_evidence(
            AuctionEvidence::Known(direction),
            AuctionEvidence::NoObservation,
            AuctionEvidence::NoObservation,
        )
    }

    fn enter_continuous(&mut self) {
        self.volatility = None;
        self.periodic_delayed = false;
    }

    fn book(&self, book: &BookRow) -> Result<CompleteBookSnapshot, MdError> {
        let (Some(bid), Some(ask)) = (&book.bid, &book.ask) else {
            return Err(self.invalid(book.seq, "row discloses no book"));
        };
        CompleteBookSnapshot::new(
            self.side(book.seq, BookSideKind::Bid, bid)?,
            self.side(book.seq, BookSideKind::Ask, ask)?,
        )
        .map_err(|_| self.invalid(book.seq, "book sides disagree"))
    }

    fn side(&self, seq: u64, kind: BookSideKind, side: &SideRow) -> Result<BookSide, MdError> {
        let mut slots = [None; BOOK_DEPTH];
        for (level, slot) in slots.iter_mut().enumerate().take(usize::from(side.levels)) {
            *slot = Some(BookLevel::new(
                self.price(seq, side.price[level])?,
                self.quantity(seq, side.quantity[level])?,
            ));
        }
        let market = side
            .market_quantity
            .map(|value| self.quantity(seq, value))
            .transpose()?;
        BookSide::from_parts(kind, market, slots)
            .map_err(|_| self.invalid(seq, "book side is not strictly price ordered"))
    }

    fn trade(
        &self,
        trade: &TradeRow,
        kind: TradeObservationKind,
    ) -> Result<ObservedTrade, MdError> {
        Ok(ObservedTrade::new(
            self.price(trade.seq, trade.price)?,
            self.quantity(trade.seq, trade.quantity)?,
            kind,
        ))
    }

    fn cumulative(&self, trade: &TradeRow) -> Observation<Volume> {
        trade
            .cumulative_quantity
            .map_or(Observation::NoObservation, |value| {
                Observation::Set(Volume::new(value, self.unit))
            })
    }

    fn price(&self, seq: u64, ticks: i64) -> Result<Price, MdError> {
        Price::positive(Decimal::from_atoms(i128::from(ticks) * ATOMS_PER_TICK))
            .map_err(|_| self.invalid(seq, "price is not positive"))
    }

    fn quantity(&self, seq: u64, value: u64) -> Result<Quantity, MdError> {
        Quantity::new(value, self.unit).map_err(|_| self.invalid(seq, "quantity is zero"))
    }

    fn invalid(&self, seq: u64, reason: &'static str) -> MdError {
        MdError::InvalidRow {
            symbol: *self.instrument.symbol(),
            seq,
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use market_types::{
        AuctionEvidence, AuctionPurpose, EventPayload, MarketSignal, Observation, Symbol,
        TradingDate, VolatilityDirection,
    };

    use super::{Message, SymbolMapper, merge_messages};
    use crate::{
        Feed, MdError,
        rows::{BookRow, Phase, SideRow, TradeRow},
    };

    fn symbol() -> Symbol {
        Symbol::new("2330").unwrap()
    }

    fn mapper(feed: Feed, interval: Option<i32>) -> SymbolMapper {
        SymbolMapper::new(
            feed,
            TradingDate::parse("2026-10-15").unwrap(),
            symbol(),
            interval,
        )
    }

    fn side(price: i64) -> SideRow {
        SideRow {
            levels: 1,
            price: [price, 0, 0, 0, 0],
            quantity: [3, 0, 0, 0, 0],
            market_quantity: None,
        }
    }

    fn book(seq: u64, phase: Phase) -> BookRow {
        BookRow {
            seq,
            exch_ts: i64::try_from(seq).unwrap() * 1_000,
            phase,
            has_trade: false,
            is_indicative: false,
            bid: Some(side(10_000_000)),
            ask: Some(side(10_050_000)),
            indicative_price: None,
            indicative_quantity: None,
            delay_open: Some(false),
            delay_close: Some(false),
            delay_state: 0,
        }
    }

    fn indicative(seq: u64, phase: Phase) -> BookRow {
        BookRow {
            is_indicative: true,
            indicative_price: Some(10_020_000),
            indicative_quantity: Some(5),
            ..book(seq, phase)
        }
    }

    fn delay(seq: u64, phase: Phase, state: i8) -> BookRow {
        BookRow {
            bid: None,
            ask: None,
            delay_state: state,
            ..book(seq, phase)
        }
    }

    fn trade(seq: u64, phase: Phase) -> TradeRow {
        TradeRow {
            seq,
            exch_ts: i64::try_from(seq).unwrap() * 1_000,
            price: 10_000_000,
            quantity: 2,
            cumulative_quantity: Some(seq),
            phase,
        }
    }

    fn signal(mapper: &mut SymbolMapper, message: Message) -> MarketSignal {
        let observed = i64::try_from(message.seq()).unwrap() * 1_000;
        let event = mapper.map(&message, observed).unwrap();
        let Observation::Set(signal) = event.payload().market_signal() else {
            panic!("every mapped event carries a signal");
        };
        signal
    }

    #[test]
    fn merge_pairs_trades_with_books_by_seq() {
        let traded = BookRow {
            has_trade: true,
            ..book(3, Phase::Continuous)
        };
        let messages = merge_messages(
            symbol(),
            vec![book(1, Phase::Continuous), traded],
            vec![trade(2, Phase::Continuous), trade(3, Phase::Continuous)],
        )
        .unwrap();
        assert!(matches!(messages[0], Message::Book(_, None)));
        assert!(matches!(messages[1], Message::TradeOnly(_)));
        assert!(matches!(messages[2], Message::Book(_, Some(_))));

        let missing = merge_messages(symbol(), vec![traded], Vec::new());
        assert!(matches!(missing, Err(MdError::InvalidRow { seq: 3, .. })));
    }

    #[test]
    fn volatility_interruption_round_carries_the_delay_direction() {
        let mut mapper = mapper(Feed::TwseF6, Some(0));
        let expected = |signal: MarketSignal| {
            let observation = signal.auction().unwrap();
            assert_eq!(
                observation.purpose(),
                AuctionEvidence::Known(AuctionPurpose::VolatilityInterruption)
            );
            assert_eq!(
                observation.direction(),
                AuctionEvidence::Known(VolatilityDirection::Up)
            );
        };
        let start = signal(
            &mut mapper,
            Message::Book(delay(1, Phase::Continuous, 1), None),
        );
        assert!(matches!(start, MarketSignal::AuctionCollecting(_)));
        expected(start);
        expected(signal(
            &mut mapper,
            Message::Book(indicative(2, Phase::PeriodicAuction), None),
        ));
        let uncross = signal(
            &mut mapper,
            Message::Book(book(3, Phase::PeriodicAuction), None),
        );
        assert!(matches!(uncross, MarketSignal::AuctionUncross(_)));
        expected(uncross);
        assert_eq!(
            signal(&mut mapper, Message::Book(book(4, Phase::Continuous), None)),
            MarketSignal::Continuous
        );
        // A periodic row with no preceding delay row has no established purpose.
        let unknown = signal(
            &mut mapper,
            Message::Book(indicative(5, Phase::PeriodicAuction), None),
        );
        assert_eq!(
            unknown.auction().unwrap().purpose(),
            AuctionEvidence::Unknown
        );
    }

    #[test]
    fn odd_lot_delay_marks_only_the_current_periodic_round() {
        let mut mapper = mapper(Feed::TwseF23, Some(0));
        let delayed = |signal: MarketSignal| {
            let observation = signal.auction().unwrap();
            assert_eq!(
                observation.purpose(),
                AuctionEvidence::Known(AuctionPurpose::Periodic)
            );
            assert_eq!(observation.direction(), AuctionEvidence::NoObservation);
            observation.delayed()
        };
        let start = signal(
            &mut mapper,
            Message::Book(delay(1, Phase::PeriodicAuction, -1), None),
        );
        assert_eq!(delayed(start), AuctionEvidence::Known(true));
        let uncross = signal(
            &mut mapper,
            Message::Book(book(2, Phase::PeriodicAuction), None),
        );
        assert_eq!(delayed(uncross), AuctionEvidence::Known(true));
        let next = signal(
            &mut mapper,
            Message::Book(book(3, Phase::PeriodicAuction), None),
        );
        assert_eq!(delayed(next), AuctionEvidence::Known(false));
    }

    #[test]
    fn opening_delay_flag_is_unknown_when_null_and_unobserved_on_odd_lots() {
        let row = BookRow {
            delay_open: None,
            ..indicative(1, Phase::OpenAuction)
        };
        let regular = signal(&mut mapper(Feed::TwseF6, Some(0)), Message::Book(row, None));
        assert_eq!(
            regular.auction().unwrap().delayed(),
            AuctionEvidence::Unknown
        );
        let odd = signal(
            &mut mapper(Feed::TwseF23, Some(0)),
            Message::Book(indicative(1, Phase::OpenAuction), None),
        );
        assert_eq!(
            odd.auction().unwrap().delayed(),
            AuctionEvidence::NoObservation
        );
    }

    #[test]
    fn indicative_without_price_clears_and_trades_map_by_row_type() {
        let mut mapper = mapper(Feed::TwseF6, Some(0));
        let row = BookRow {
            indicative_price: None,
            indicative_quantity: None,
            ..indicative(1, Phase::OpenAuction)
        };
        let event = mapper.map(&Message::Book(row, None), 1_000).unwrap();
        let EventPayload::IndicativeAuction(auction) = event.payload() else {
            panic!("indicative row maps to IndicativeAuction");
        };
        assert_eq!(auction.price(), &Observation::Clear);
        assert_eq!(auction.quantity(), &Observation::Clear);

        let traded = BookRow {
            has_trade: true,
            ..book(2, Phase::Continuous)
        };
        let quote = mapper
            .map(
                &Message::Book(traded, Some(trade(2, Phase::Continuous))),
                2_000,
            )
            .unwrap();
        assert!(matches!(quote.payload(), EventPayload::QuoteSnapshot(_)));
        let intermediate = mapper
            .map(&Message::TradeOnly(trade(3, Phase::Continuous)), 3_500)
            .unwrap();
        assert!(matches!(
            intermediate.payload(),
            EventPayload::TradeBatch(_)
        ));
        assert_eq!(intermediate.observed_time().as_unix_microseconds(), 3_500);
        assert_eq!(intermediate.source_sequence(), Some(3));
    }

    #[test]
    fn exch_ts_regression_within_a_symbol_is_rejected() {
        let mut mapper = mapper(Feed::TwseF6, Some(0));
        mapper
            .map(&Message::Book(book(5, Phase::Continuous), None), 5_000)
            .unwrap();
        let earlier = BookRow {
            exch_ts: 4_000,
            ..book(6, Phase::Continuous)
        };
        assert!(matches!(
            mapper.map(&Message::Book(earlier, None), 6_000),
            Err(MdError::InvalidRow { seq: 6, .. })
        ));
    }
}
