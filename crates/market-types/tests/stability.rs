use market_types::{
    AuctionObservation, DomainEvent, EventPayload, IndicativeAuction, InstrumentId,
    MarketAnnotations, MarketId, MatchTime, Observation, Price, Quantity, QuantityUnit,
    SourceFormatId, Symbol, TradingDate, VolatilityDirection,
};

#[test]
fn indicative_auction_roundtrips_without_becoming_a_trade() {
    let auction = IndicativeAuction::new(
        AuctionObservation::volatility_interruption(VolatilityDirection::Up, false, false),
        Observation::Set(Price::parse("100").unwrap()),
        Observation::Set(Quantity::new(2, QuantityUnit::Contract).unwrap()),
        Observation::NoObservation,
        Observation::NoObservation,
        MarketAnnotations::None,
    )
    .unwrap();
    let event = DomainEvent::new(
        InstrumentId::new(MarketId::Taifex, Symbol::new("TXFH6").unwrap()),
        TradingDate::parse("2026-07-20").unwrap(),
        SourceFormatId::new("I022").unwrap(),
        MatchTime::parse("2026-07-20T08:40:00+08:00").unwrap(),
        None,
        EventPayload::IndicativeAuction(auction),
    );
    let canonical = event.to_canonical_bytes().unwrap();
    assert_eq!(
        DomainEvent::from_canonical_bytes(&canonical).unwrap(),
        event
    );
    assert_eq!(
        event.payload().kind(),
        market_types::EventKind::IndicativeAuction
    );
    let EventPayload::IndicativeAuction(auction) = event.payload() else {
        unreachable!()
    };
    assert_eq!(
        auction.observation(),
        AuctionObservation::volatility_interruption(VolatilityDirection::Up, false, false)
    );
}
