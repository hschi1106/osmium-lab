use market_types::{InstrumentId, MarketId, MarketIdError, Symbol, SymbolError};

#[test]
fn market_id_has_fixed_discriminants_and_ordering_ranks() {
    let cases = [
        (MarketId::Twse, 1),
        (MarketId::Tpex, 2),
        (MarketId::Taifex, 3),
    ];

    for (market, value) in cases {
        assert_eq!(market.discriminant(), value);
        assert_eq!(market.ordering_rank(), value);
        assert_eq!(u8::from(market), value);
        assert_eq!(MarketId::try_from(value), Ok(market));
    }

    assert_eq!(
        MarketId::try_from(0),
        Err(MarketIdError::UnknownDiscriminant(0))
    );
    assert_eq!(
        MarketId::try_from(4),
        Err(MarketIdError::UnknownDiscriminant(4))
    );
    assert!(MarketId::Twse < MarketId::Tpex);
    assert!(MarketId::Tpex < MarketId::Taifex);
}

#[test]
fn symbol_is_non_empty_and_byte_exact() {
    assert_eq!(Symbol::try_from(""), Err(SymbolError::Empty));

    let symbol = Symbol::try_from(String::from("02330 ")).unwrap();
    assert_eq!(symbol.as_str(), "02330 ");
    assert_eq!(symbol.as_bytes(), b"02330 ");
    assert_eq!(symbol.to_string(), "02330 ");
    assert_ne!(symbol, Symbol::try_from("2330").unwrap());
}

#[test]
fn instrument_identity_orders_by_market_then_symbol_bytes() {
    let twse_2330 = InstrumentId::new(MarketId::Twse, Symbol::try_from("2330").unwrap());
    let twse_0050 = InstrumentId::new(MarketId::Twse, Symbol::try_from("0050").unwrap());
    let tpex_0050 = InstrumentId::new(MarketId::Tpex, Symbol::try_from("0050").unwrap());

    assert_eq!(twse_2330.market(), MarketId::Twse);
    assert_eq!(twse_2330.symbol().as_str(), "2330");
    assert!(twse_0050 < twse_2330);
    assert!(twse_2330 < tpex_0050);

    let mut instruments = [tpex_0050.clone(), twse_2330.clone(), twse_0050.clone()];
    instruments.sort();
    assert_eq!(instruments, [twse_0050, twse_2330, tpex_0050]);
}
