use std::{collections::BTreeMap, error::Error, hint::black_box, time::Instant};

use market_state::{
    MarketState, MarketStateReducer, ReducerContext, SegmentBoundaryPolicy, SessionSegmentId,
};
use market_types::{
    BookLevel, BookSide, BookSideKind, CompleteBookSnapshot, DomainEvent, EventPayload,
    InstrumentId, MarketAnnotations, MarketId, MatchTime, Observation, ObservedTrade, Price,
    Quantity, QuantityUnit, QuoteSnapshot, SourceFormatId, Symbol, TradeObservationKind,
    TradingDate, TwseQuoteAnnotations, Volume,
};
use replay_engine::{
    EventStream, ReplayCore, ReplayPlan, ReplayStreamBinding, ReplayStreamFactory,
    StableStreamDescriptorId,
};

const TOTAL_EVENTS: usize = 49_152;
const STREAM_COUNTS: [usize; 3] = [1, 8, 32];
const ROUNDS: usize = 3;

struct VecStream(std::vec::IntoIter<DomainEvent>);

impl EventStream for VecStream {
    type Error = std::io::Error;

    fn next_event(&mut self) -> Result<Option<DomainEvent>, Self::Error> {
        Ok(self.0.next())
    }
}

#[derive(Clone)]
struct StreamInput {
    binding: ReplayStreamBinding,
    events: Vec<DomainEvent>,
}

struct MemoryFactory {
    streams: BTreeMap<StableStreamDescriptorId, Vec<DomainEvent>>,
}

impl MemoryFactory {
    fn from_inputs(inputs: &[StreamInput]) -> Self {
        Self {
            streams: inputs
                .iter()
                .map(|input| (input.binding.descriptor_id(), input.events.clone()))
                .collect(),
        }
    }
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
    plan: ReplayPlan,
    inputs: Vec<StreamInput>,
    instruments: Vec<InstrumentId>,
}

impl Case {
    fn new(stream_count: usize) -> Self {
        let date = TradingDate::parse("2026-08-12").unwrap();
        let per_stream = TOTAL_EVENTS.div_ceil(stream_count);
        let base_time = MatchTime::parse("2026-08-12T09:00:00+08:00")
            .unwrap()
            .as_unix_microseconds();
        let template_payload = fixture_payload();
        let mut bindings = Vec::with_capacity(stream_count);
        let mut inputs = Vec::with_capacity(stream_count);
        let mut instruments = Vec::with_capacity(stream_count);

        for stream_index in 0..stream_count {
            let symbol = format!("{:04}", stream_index + 1);
            let instrument = InstrumentId::new(
                MarketId::Twse,
                Symbol::new(symbol).expect("benchmark symbol is valid"),
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
                    let global_sequence = sequence * stream_count + stream_index;
                    DomainEvent::new(
                        instrument.clone(),
                        date,
                        SourceFormatId::new("STOCK_REALTIME").unwrap(),
                        MatchTime::from_unix_microseconds(base_time + global_sequence as i64),
                        Some(global_sequence as u64),
                        template_payload.clone(),
                    )
                })
                .collect();
            bindings.push(binding.clone());
            inputs.push(StreamInput { binding, events });
            instruments.push(instrument);
        }

        Self {
            plan: ReplayPlan::new_multi([7; 32], bindings).unwrap(),
            inputs,
            instruments,
        }
    }

    fn replay(&self) -> (u128, [u8; 32], [u8; 32]) {
        let date = TradingDate::parse("2026-08-12").unwrap();
        let states = self
            .instruments
            .iter()
            .cloned()
            .map(|instrument| MarketState::new(instrument, date))
            .collect();
        let reducers = self
            .instruments
            .iter()
            .cloned()
            .map(|instrument| (instrument, MarketStateReducer::twse_regular()))
            .collect();
        let contexts = self
            .instruments
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
        let mut core = ReplayCore::new_multi(states, reducers, contexts).unwrap();
        let mut factory = MemoryFactory::from_inputs(&self.inputs);
        let started = Instant::now();
        core.replay_frozen_multi(&self.plan, &mut factory).unwrap();
        let completed = core.complete().unwrap();
        let elapsed = started.elapsed().as_nanos();
        (
            elapsed,
            *completed.summary().event_checksum().as_bytes(),
            *completed.summary().final_state_checksum().as_bytes(),
        )
    }
}

fn fixture_payload() -> EventPayload {
    let unit = QuantityUnit::TradingUnit;
    let bids = [
        ("100.00", 73),
        ("99.50", 131),
        ("99.00", 211),
        ("98.50", 307),
        ("98.00", 401),
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
        ("100.50", 79),
        ("101.00", 137),
        ("101.50", 223),
        ("102.00", 313),
        ("102.50", 409),
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
        Observation::Set(ObservedTrade::new(
            Price::parse("100.50").unwrap(),
            Quantity::new(17, unit).unwrap(),
            TradeObservationKind::Regular,
        )),
        Observation::Set(Volume::new(12_345, unit)),
        MarketAnnotations::TwseQuote(TwseQuoteAnnotations::new(0, 0)),
    )
    .unwrap();
    EventPayload::QuoteSnapshot(quote)
}

fn main() -> Result<(), Box<dyn Error>> {
    println!(
        "multi-stream replay benchmark: target {}/{}, {TOTAL_EVENTS} events, {ROUNDS} rounds",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );

    for stream_count in STREAM_COUNTS {
        let case = Case::new(stream_count);
        let total_events = case
            .inputs
            .iter()
            .map(|input| input.events.len())
            .sum::<usize>();
        let mut measurements = Vec::with_capacity(ROUNDS);
        let mut reference = None;
        for _ in 0..ROUNDS {
            let (elapsed, event_checksum, final_state_checksum) = black_box(case.replay());
            if let Some((expected_events, expected_state)) = reference {
                assert_eq!(event_checksum, expected_events);
                assert_eq!(final_state_checksum, expected_state);
            } else {
                reference = Some((event_checksum, final_state_checksum));
            }
            measurements.push(elapsed);
        }
        measurements.sort_unstable();
        let median = measurements[ROUNDS / 2];
        println!(
            "{stream_count} streams: {total_events} events, median {median} ns, {:.0} events/s; checksums equivalent",
            total_events as f64 * 1_000_000_000.0 / median as f64,
        );
    }
    Ok(())
}
