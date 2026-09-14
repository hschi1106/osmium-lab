use std::{collections::BTreeMap, hint::black_box, time::Instant};

use execution_sim::{
    AccountingModel, ChargeBasis, ChargeModel, ChargeSides, EvidenceMode, FillModel,
    InstrumentEconomics, InstrumentLedgerConfig, MultiLedger, MultiSimulator, QuantityPolicy,
    RoundingPolicy,
};
use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{
    BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, Decimal, DomainEvent, EventPayload,
    InstrumentId, MarketAnnotations, MarketId, MatchTime, Observation, ObservedTrade, Price,
    PricePolicy, Quantity, QuantityUnit, QuoteSnapshot, SourceFormatId, Symbol,
    TradeObservationKind, TradingDate, TwseQuoteAnnotations, Volume,
};
use osmium_runner::{MultiSessionSchedule, run_multi_backtest};
use replay_engine::{
    EventStream, ReplayCore, ReplayPlan, ReplayStreamBinding, ReplayStreamFactory,
    StableStreamDescriptorId,
};
use strategy_api::{AcceptanceStrategy, SessionKind, SessionSegment};

const STREAM_COUNT: usize = 8;
const TOTAL_EVENTS: usize = 49_152;
const ROUNDS: usize = 21;

struct VecStream(std::vec::IntoIter<DomainEvent>);

impl EventStream for VecStream {
    type Error = std::io::Error;

    fn next_event(&mut self) -> Result<Option<DomainEvent>, Self::Error> {
        Ok(self.0.next())
    }
}

struct MemoryFactory {
    streams: BTreeMap<StableStreamDescriptorId, Vec<DomainEvent>>,
}

impl ReplayStreamFactory for MemoryFactory {
    type Stream = VecStream;
    type Error = std::io::Error;

    fn open(&mut self, binding: &ReplayStreamBinding) -> Result<Self::Stream, Self::Error> {
        self.streams
            .remove(&binding.descriptor_id())
            .map(|events| VecStream(events.into_iter()))
            .ok_or_else(|| std::io::Error::other("missing benchmark stream"))
    }
}

struct Case {
    date: TradingDate,
    segment: SessionSegment,
    plan: ReplayPlan,
    streams: BTreeMap<StableStreamDescriptorId, Vec<DomainEvent>>,
    instruments: Vec<InstrumentId>,
}

impl Case {
    fn new() -> Self {
        let date = TradingDate::parse("2026-08-12").unwrap();
        let segment = SessionSegment::new(
            SessionSegmentId::new("regular").unwrap(),
            SessionKind::Regular,
            date,
            MatchTime::parse("2026-08-12T09:00:00+08:00").unwrap(),
            MatchTime::parse("2026-08-12T13:30:00+08:00").unwrap(),
        )
        .unwrap();
        let per_stream = TOTAL_EVENTS / STREAM_COUNT;
        let base_time = segment.open().as_unix_microseconds();
        let payload = fixture_payload();
        let mut bindings = Vec::with_capacity(STREAM_COUNT);
        let mut streams = BTreeMap::new();
        let mut instruments = Vec::with_capacity(STREAM_COUNT);

        for stream_index in 0..STREAM_COUNT {
            let instrument = InstrumentId::new(
                MarketId::Twse,
                Symbol::new(format!("{:04}", stream_index + 1)).unwrap(),
            );
            let descriptor_id = StableStreamDescriptorId::from_bytes(
                [u8::try_from(stream_index + 1).expect("stream count fits in u8"); 32],
            );
            let binding = ReplayStreamBinding::new(
                descriptor_id,
                instrument.clone(),
                date,
                [stream_index as u8 + 1; 32],
                [stream_index as u8 + 2; 32],
            );
            let events = (0..per_stream)
                .map(|sequence| {
                    let global_sequence = sequence * STREAM_COUNT + stream_index;
                    let quote = QuoteSnapshot::new(
                        match &payload {
                            EventPayload::QuoteSnapshot(quote) => quote.book().clone(),
                            _ => unreachable!("benchmark payload is a quote"),
                        },
                        Observation::Set(ObservedTrade::new(
                            Price::parse("100.50").unwrap(),
                            Quantity::new(1, QuantityUnit::TradingUnit).unwrap(),
                            TradeObservationKind::Regular,
                        )),
                        Observation::Set(Volume::new(
                            sequence as u64 + 1,
                            QuantityUnit::TradingUnit,
                        )),
                        MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0x10, 0)),
                    )
                    .unwrap();
                    DomainEvent::new(
                        instrument.clone(),
                        date,
                        SourceFormatId::new("STOCK_REALTIME").unwrap(),
                        MatchTime::from_unix_microseconds(base_time + global_sequence as i64),
                        Some(global_sequence as u64),
                        EventPayload::QuoteSnapshot(quote),
                    )
                })
                .collect();
            bindings.push(binding);
            streams.insert(descriptor_id, events);
            instruments.push(instrument);
        }

        Self {
            date,
            segment,
            plan: ReplayPlan::new_multi([7; 32], bindings).unwrap(),
            streams,
            instruments,
        }
    }

    fn run(self) -> ([u8; 32], [u8; 32], usize, usize) {
        let Case {
            date,
            segment,
            plan,
            streams,
            instruments,
        } = self;
        let states = instruments
            .iter()
            .cloned()
            .map(|instrument| MarketState::new(instrument, date))
            .collect();
        let reducers = instruments
            .iter()
            .cloned()
            .map(|instrument| (instrument, MarketStateReducer::twse_regular()))
            .collect();
        let contexts = instruments
            .iter()
            .cloned()
            .map(|instrument| {
                (
                    instrument,
                    ReducerContext::new(
                        date,
                        SessionSegmentId::new("regular").unwrap(),
                        SegmentBoundaryPolicy::Carry,
                        1,
                    ),
                )
            })
            .collect();
        let core = ReplayCore::new_multi(states, reducers, contexts).unwrap();
        let schedule = MultiSessionSchedule::new(
            instruments
                .iter()
                .cloned()
                .map(|instrument| (instrument, vec![segment.clone()])),
        )
        .unwrap();
        let strategy = AcceptanceStrategy::new(
            AcceptanceStrategy::source_binary_identity().unwrap(),
            instruments.iter().cloned(),
            [SessionKind::Regular],
        )
        .unwrap();
        let simulator = MultiSimulator::new(instruments.iter().cloned().map(|instrument| {
            (
                instrument,
                QuantityUnit::TradingUnit,
                FillModel {
                    evidence: EvidenceMode::TopOfBook,
                    quantity: QuantityPolicy::Displayed,
                    adverse_price_delta: Decimal::ZERO,
                    market_data_latency_ms: 0,
                    order_latency_ms: 0,
                },
                PricePolicy::PositiveOnly,
            )
        }))
        .unwrap();
        let zero_charge = ChargeModel {
            basis: ChargeBasis::NotionalRate,
            rate: Decimal::ZERO,
            sides: ChargeSides::Both,
            minimum: Decimal::ZERO,
            precision: 0,
            rounding: RoundingPolicy::Down,
        };
        let ledger = MultiLedger::new(
            Decimal::parse("100000000").unwrap(),
            instruments.iter().cloned().map(|instrument| {
                InstrumentLedgerConfig::new(
                    instrument,
                    QuantityUnit::TradingUnit,
                    AccountingModel::EquityV1,
                    InstrumentEconomics {
                        units_per_trading_unit: 1000,
                        multiplier: Decimal::parse("1").unwrap(),
                        provenance: "synthetic benchmark economics".into(),
                    },
                    zero_charge,
                    zero_charge,
                )
            }),
        )
        .unwrap();
        let mut factory = MemoryFactory { streams };
        let completed = run_multi_backtest(
            core,
            strategy,
            &plan,
            &mut factory,
            &schedule,
            simulator,
            ledger,
            false,
        )
        .unwrap();
        (
            *completed.replay.summary().event_checksum().as_bytes(),
            *completed.replay.summary().final_state_checksum().as_bytes(),
            completed.simulator.fill_count(),
            completed.strategy_output.records().len(),
        )
    }
}

fn fixture_payload() -> EventPayload {
    let unit = QuantityUnit::TradingUnit;
    let bids = [
        ("100.00", 10_000),
        ("99.50", 10_000),
        ("99.00", 10_000),
        ("98.50", 10_000),
        ("98.00", 10_000),
    ]
    .into_iter()
    .map(|(price, quantity)| {
        BookLevel::new(
            Price::parse(price).unwrap(),
            Quantity::new(quantity, unit).unwrap(),
        )
    })
    .collect();
    let asks = [
        ("100.50", 10_000),
        ("101.00", 10_000),
        ("101.50", 10_000),
        ("102.00", 10_000),
        ("102.50", 10_000),
    ]
    .into_iter()
    .map(|(price, quantity)| {
        BookLevel::new(
            Price::parse(price).unwrap(),
            Quantity::new(quantity, unit).unwrap(),
        )
    })
    .collect();
    let book = CompleteBookSnapshot::new(
        BookSide::new(BookSideKind::Bid, bids).unwrap(),
        BookSide::new(BookSideKind::Ask, asks).unwrap(),
    )
    .unwrap();
    let quote = QuoteSnapshot::new(
        book,
        Observation::NoObservation,
        Observation::NoObservation,
        MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0x10, 0)),
    )
    .unwrap();
    EventPayload::QuoteSnapshot(quote)
}

fn main() {
    println!(
        "full multi-backtest: target {}/{}, {STREAM_COUNT} TWSE instruments, {TOTAL_EVENTS} events, {ROUNDS} rounds",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    let mut measurements = Vec::with_capacity(ROUNDS);
    let mut checksum_pair = None;
    let mut observed_result = None;
    for _ in 0..ROUNDS {
        let case = Case::new();
        let started = Instant::now();
        let result = black_box(case.run());
        let elapsed = started.elapsed().as_nanos();
        let pair = (result.0, result.1);
        if let Some(expected) = checksum_pair {
            assert_eq!(pair, expected, "backtest checksums must be reproducible");
        } else {
            checksum_pair = Some(pair);
        }
        if let Some(expected) = observed_result {
            assert_eq!((result.2, result.3), expected);
        } else {
            observed_result = Some((result.2, result.3));
        }
        measurements.push(elapsed);
    }
    measurements.sort_unstable();
    let median = measurements[ROUNDS / 2];
    println!(
        "median {median} ns, {:.0} backtest events/s, {} fills, {} strategy output records; event and final-state checksums equivalent",
        TOTAL_EVENTS as f64 * 1_000_000_000.0 / median as f64,
        observed_result.unwrap().0,
        observed_result.unwrap().1,
    );
}
