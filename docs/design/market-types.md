# Market Types 與 Event Schema 設計

## 1. 文件目的

本文件定義 `osmium-lab` 第一版 domain market types、標準事件 logical schema、
驗證 invariant 與 canonical encoding。目標是讓 normalizer、replayer、
MarketState、strategy 與 simulation 共用同一組不依賴 Teralion wire type 的資料
語意。

本文件固定第一版契約：

- market types schema version `1`
- event schema version `1`
- canonical event encoding version `1`
- exact time、decimal、quantity 與 instrument identity
- `QuoteSnapshot`、`BookSnapshot`、`TradeBatch` payload shape
- `Set`／`Clear`／`NoObservation`／`Unknown` update semantics
- TWSE quote flags 的 typed view
- canonical bytes 的欄位順序與 primitive encoding

本文件不固定 crate、module、Rust package 或 cache file layout。實作可以使用
newtype、enum、array、small-vector 或其他容器，只要 public semantics、validation
與 canonical bytes 完全符合本文件。

依據：

- [產品需求](../product-requirements.md)
- [回播需求](../requirements/replay.md)
- [M1 TWSE 增量](../increments/M1-twse-replay.md)
- [ADR-0001：match-time ordering](../architecture/decisions/0001-match-time-ordering.md)
- [ADR-0002：snapshot MarketState](../architecture/decisions/0002-snapshot-market-state.md)
- [ADR-0004：TradingContext 與 eligibility](../architecture/decisions/0004-trading-context-and-eligibility.md)
- [Teralion interface](../interfaces/teralion.md)
- [TWSE interface](../interfaces/twse.md)

## 2. 設計原則

### 2.1 Wire 與 domain 分離

Teralion JSON number、nullable field、format string 與 response object 不是 domain
type。資料流程必須是：

```text
Teralion wire type
-> market/format validator
-> validated domain types
-> DomainEvent
-> replay cache / replayer / strategy
```

strategy 與 simulation 不得引用 Teralion response type、JSON value、cursor、
`received_at` 或 API-specific enum。

### 2.2 Exact value

domain market value 不得使用：

- `f32`／`f64` 表示 price、strike、multiplier、money 或 canonical checksum input。
- local timezone、locale 或 formatted string 比較時間。
- 未帶 unit 的 generic integer 混用 book、trade、order 或 position quantity。
- Rust `Debug`、memory layout、enum declaration order 或 serializer default 作
  canonical encoding。

### 2.3 Source fidelity

domain type 只表達來源 interface 已證實的內容：

- book 是完整 snapshot，不是 order-level delta。
- trade 是 source observation，不推論 aggressor、order identity 或 queue position。
- raw flags 永遠保留；typed view 只能解碼已確認 bits。
- absence、clear、no observation 與 unknown 不得合併成 `Option<T>`。
- 不支援的 format 或 shape 不能靠 generic fallback 進入 timeline。

## 3. Version identity

每個 event stream、replay cache 與 run manifest 必須能識別：

```text
market_types_version       = 1
event_schema_version       = 1
canonical_event_version    = 1
ordering_rule_version      = 2
normalizer_mapping_name    = market-interface-specific name
normalizer_mapping_version = market-interface-specific integer
```

任一會改變 value semantics、event field、enum discriminant、canonical bytes 或
typed flag decoding 的變更，都需要新版本或明確 compatibility review。不同 event
schema／canonical version 不得出現在同一 stream。

## 4. Primitive types

### 4.1 `MarketId`

domain market 與 `OrderingRule.market_rank` 使用同一固定 identity：

| Market | Discriminant | Ordering rank |
| --- | ---: | ---: |
| `Twse` | 1 | 1 |
| `Tpex` | 2 | 2 |
| `Taifex` | 3 | 3 |

Teralion 的 `taifex_fut`／`taifex_opt` 都映射至 `Taifex`；商品種類由
`InstrumentKind` 區分。unknown market 不可臨時以字串進入 event。

### 4.2 `Symbol`

`Symbol` 是經 market interface 驗證後的 canonical UTF-8 bytes。

規則：

- 不可為空。
- 不做 locale collation。
- 不因看似數字而轉成 integer。
- 不自動 trim、大小寫轉換、移除前導零或替換字元。
- market-specific normalization 必須在 interface version 中明確定義。
- ordering 與 canonical encoding 使用 unsigned lexicographic UTF-8 bytes。

M1 的 `2330` 仍是一般合法 symbol；domain type 不把它硬編碼成唯一值。

### 4.3 `InstrumentId`

```text
InstrumentId {
    market: MarketId
    symbol: Symbol
}
```

identity comparison 先比 market discriminant，再比 symbol canonical bytes。name、
root、expiry、filesystem path 或 Teralion display label 都不參與 identity。

### 4.4 `TradingDate`

`TradingDate` 表示 exchange business date，不是 tick 的 local calendar date。
logical value 是 Gregorian `YYYY-MM-DD`；canonical representation 是從
1970-01-01 起算的 signed day count：

```text
TradingDate = i32 epoch_days
```

解析必須拒絕不存在日期。TAIFEX 夜盤由 SessionPlan／calendar 指派 trading date，
不能只從 `match_time` 的日曆日期建構。

### 4.5 `MatchTime`

```text
MatchTime = i64 unix_microseconds_utc
```

規則：

- 從含 offset 的 ISO-8601 `match_time` 解析。
- 將 instant 正規化為 UTC microseconds。
- 保留 Teralion 已確認的 microsecond 精度。
- fraction 超過 6 位且被截斷部分非 zero 時拒絕，不可 round。
- leap second、invalid offset、overflow 或不存在 local time 拒絕。
- total order 使用 signed integer order。
- formatted offset 不參與 equality；同一 instant 的不同合法 offset 表示相等。

`received_at` 不屬於 `MatchTime`，也不放入 domain event。source provenance 仍
保存其原始值。

### 4.6 `Decimal`

所有 exact decimal 使用固定 18 位小數的 signed atoms：

```text
Decimal {
    atoms: i128
}

value = atoms / 10^18
```

選擇固定 scale 的目的：

- equality 與 ordering 不需處理不同 scale。
- `2350`、`2350.0` 與 `2.350e3` 產生相同 domain value。
- 18 位足以容納目前市場 price、strike、multiplier、fee rate 與 currency
  conversion 的預期精度，同時保留 checked `i128` arithmetic。

wire decoder 必須從 JSON numeric lexeme 或 decimal string 直接解析，不能先經
`f64`。超過 18 位且非 zero 的小數、NaN、infinity 或 `i128` overflow 必須拒絕。
所有 arithmetic 使用 checked operation；overflow 是 error，不 wrap、不 saturate。

canonical encoding 是 `atoms` 的 16-byte two's-complement big-endian。generic
decimal 可以為負或 zero；具體 newtype 另加 invariant。

### 4.7 `Price`

```text
Price(Decimal)
```

invariant：

- `atoms > 0`
- exact、finite、可 canonical encode
- 不在 generic type 中套用 tick size；tick-size validation 由 instrument／market
  interface 在資料可用時處理

wire 中用來表示「尚無價格」的 `0` 必須在 normalizer 轉為 absence／
`NoObservation`，不能建構 `Price(0)`。

### 4.8 Quantity types

quantity unit 必須顯式存在：

| `QuantityUnit` | Discriminant | 語意 |
| --- | ---: | --- |
| `SourceUnit` | 0 | 來源只稱 quantity，尚未證實 shares／trading units／contracts |
| `Share` | 1 | 一股；只用於已確認以股計數的 equity quantity |
| `TradingUnit` | 2 | 一個交易所定義的交易單位；每單位包含多少股／受益權單位由 instrument metadata 決定 |
| `Contract` | 3 | 一口衍生品契約；契約乘數由 instrument metadata 決定 |

book level、single trade 與 order size 使用：

```text
Quantity {
    value: u64        // 必須 > 0
    unit: QuantityUnit
}
```

cumulative volume 允許合法 zero：

```text
Volume {
    value: u64
    unit: QuantityUnit
}
```

規則：

- 不同 unit 不可直接相加、比較 fill quantity 或計算 position。
- `SourceUnit` 不得在 simulation 中自動當成 `Share`／`TradingUnit`／`Contract`。
- unit conversion 必須由 market interface 或帶 provenance 的 instrument
  configuration 提供。
- checked add／subtract；overflow、underflow 或 unit mismatch 是 error。

market／mechanism application：

| Market／mechanism | Domain unit | Evidence／boundary |
| --- | --- | --- |
| TWSE regular `STOCK_SNAPSHOT`／`STOCK_REALTIME` | `TradingUnit` | B.12.13 明定成交量、五檔量與累計成交量每一數量單位為一交易單位；2330 fixture magnitude 相符 |
| TWSE intraday odd-lot equity | `Share` | B.12.13 明定每一數量單位為一股；目前仍是 unsupported format |
| TPEx regular equity | `TradingUnit` | [TPEx 交易制度](https://www.tpex.org.tw/zh-tw/mainboard/trading/rules/system.html)明定一般有價證券通常每交易單位 1,000 股／單位，但有例外；Teralion format mapping 待 fixture 固定 |
| TPEx odd-lot equity | `Share` | TPEx 明定零股以一股為交易單位；Teralion format mapping 待 fixture 固定 |
| TAIFEX futures／options | `Contract` | TAIFEX futures 與 options 行情分別以「口」表示成交量與未沖銷契約量；Teralion format mapping 待 fixture 固定 |

TWSE／TPEx regular 的 `TradingUnit` 不等於硬編碼的 1,000 股。TWSE 普通股票通常
為 1,000 股，但 secondary-listed foreign stock、offshore ETF 或其他商品可有
不同單位；TPEx 也有第二上櫃外國股票與加掛 ETF 等例外。每一交易單位所含的
security units 必須由具日期與 provenance 的 instrument metadata 提供。

同理，TAIFEX `Contract` quantity 不包含 contract multiplier。股票期貨一口可
表彰 2,000 股或其他經公告／調整後數值，該 multiplier 與 quantity 分開保存。
官方 futures／options quantity evidence：

- [TAIFEX futures daily market report](https://www.taifex.com.tw/cht/3/futDailyMarketReport)
- [TAIFEX options daily market report](https://www.taifex.com.tw/cht/3/optDailyMarketReport)

### 4.9 `SourceFormatId`

`SourceFormatId` 是 normalizer registry 中具版本的 canonical UTF-8 identifier，
例如 `STOCK_SNAPSHOT`。

- 必須可回溯到 source payload。
- unknown identifier 不可進入 accepted event。
- 不使用 Rust type name、module path 或 runtime registration index。
- comparison／encoding 使用 exact UTF-8 bytes。

## 5. Instrument reference types

### 5.1 `InstrumentKind`

| Kind | Discriminant |
| --- | ---: |
| `Equity` | 1 |
| `Warrant` | 2 |
| `Future` | 3 |
| `Option` | 4 |
| `Unknown` | 255 |

`Unknown` 必須附帶原始 source kind string；它表示 metadata 尚未能分類，不代表
normalizer 可以猜測商品種類。M1 可以回播已由 execution plan 明確選定的 TWSE
equity fixture，但不能因 Teralion `kind=""` 就把所有空 kind 商品泛化成 equity。

### 5.2 `OptionSide`

```text
Call = 1
Put  = 2
```

非 option 的 `call_put` 必須 unavailable。unknown source value 不得默認成 Call
或 Put。

### 5.3 `InstrumentReference`

logical schema：

```text
InstrumentReference {
    instrument: InstrumentId
    trading_date: TradingDate
    kind: KnownOrUnknown<InstrumentKind>
    root: OptionalSourceText
    expiry: Optional<TradingDate>
    strike: Optional<Price>
    option_side: Optional<OptionSide>
    multiplier: SourcedOptional<Decimal>
    currency: SourcedOptional<CurrencyCode>
}
```

`Optional` 在此只表達 static／daily reference field 是否存在，不是 MarketState
update，因此不使用第 6 節的 `Observation`。`multiplier` 與 `currency` 必須保留
Teralion、user config 或其他 reference source 的 provenance；具體 provenance
record 由 data-sync／execution-sim design 定義。

Instrument reference 不直接進 `CanonicalEvent`；event 只包含
`InstrumentId`。cache／run manifest 另外綁定適用 metadata checksum，避免
metadata 更新悄悄改變 P&L。

## 6. Observation semantics

### 6.1 `Observation<T>`

```text
Observation<T> {
    NoObservation
    Set(T)
    Clear
    Unknown(UnknownValue)
}
```

固定 discriminant：

| Variant | Discriminant | Reducer action |
| --- | ---: | --- |
| `NoObservation` | 0 | 保留既有 state；本 event 沒有更新該欄位 |
| `Set` | 1 | 以新 value 取代 |
| `Clear` | 2 | 明確設為 unavailable |
| `Unknown` | 3 | 保存 raw、產生 warning，不套用未確認 domain 語意 |

不得用 `Option<T>`、zero、empty string 或空 array替代上述語意。

### 6.2 `UnknownValue`

unknown raw value 只允許可無損 canonical encode 的 bounded scalar：

| Variant | Discriminant | Payload |
| --- | ---: | --- |
| `Unsigned` | 1 | `u64` |
| `Signed` | 2 | `i64` |
| `Decimal` | 3 | `Decimal` |
| `Text` | 4 | length-prefixed UTF-8 |
| `Bytes` | 5 | length-prefixed bytes |

完整 unknown JSON object 不進 domain event；它留在 source partition。unknown
format、missing required object 或不合法 nested shape 必須拒絕，不得包成
`Unknown(Bytes)` 繞過 validator。

### 6.3 Book observation

`QuoteSnapshot` 與 `BookSnapshot` 的 book 一定是
`CompleteBookSnapshot`，不使用 `NoObservation`。

trade-only source record 應映射成 `TradeBatch`，而不是：

- `QuoteSnapshot` + empty book
- `QuoteSnapshot` + 沿用舊 book
- partial book delta

這項選擇維持產品需求中「QuoteSnapshot 代表完整最佳五檔」的 invariant。

## 7. Book 與 trade primitives

### 7.1 `BookLevel`

```text
BookLevel {
    price: Price
    displayed_quantity: Quantity
}
```

`displayed_quantity.value > 0`。它只代表該 snapshot 顯示量，不代表單一 order、
可成交總量或 queue position。

### 7.2 `BookSide`

```text
BookSide = [Optional<BookLevel>; 5]
```

invariant：

- present slots 必須從 index 0 連續排列；第一個 empty 後不可再有 present slot。
- bids 依 price strictly descending。
- asks 依 price strictly ascending。
- 相鄰 level price 不可相同。
- 所有 present levels 使用相同 quantity unit。
- 0 至 5 個 present levels 都是合法完整 side。

### 7.3 `CompleteBookSnapshot`

```text
CompleteBookSnapshot {
    bids: BookSide
    asks: BookSide
}
```

通用 validator 不要求 `best_bid < best_ask`。locked、crossed、trial、auction 或特殊
市場狀態是否合法，由 market interface 與 fixture 決定；generic type 不用連續
撮合假設拒絕。

### 7.4 `TradePrint`

```text
TradePrint {
    price: Price
    quantity: Quantity
    print_kind: TradePrintKind
}
```

| Kind | Discriminant | 語意 |
| --- | ---: | --- |
| `Regular` | 0 | source 沒有更細 qualifier 的成交 observation |
| `Intermediate` | 1 | 同一撮合結果中的非最後成交揭示，不帶完整 book |

`Intermediate` 不是 synthetic trade，也不表示 incomplete source page。它仍保存
來源明確提供的 price／quantity，但 fill model 是否可使用由 market annotations
與 simulation policy 決定。

### 7.5 `TradeList`

`TradeBatch.trades` 必須非空。batch order semantics：

| `TradeOrder` | Discriminant | 語意 |
| --- | ---: | --- |
| `Unspecified` | 0 | 來源未證實 batch 內真實順序；canonical encoding 使用已保存順序，但不宣稱市場因果 |
| `SourceOrdered` | 1 | interface 與 fixture 明確證實來源順序 |

normalizer 不得只為了 deterministic output 將 `Unspecified` 改成
`SourceOrdered`。若來源是 unordered collection，mapping 文件必須定義 deterministic
canonical order並保持 `Unspecified`。

## 8. Market annotations

### 8.1 `MarketAnnotations`

```text
MarketAnnotations {
    None
    TwseQuote(TwseQuoteAnnotations)
    TpexQuote(...)     // 只有 interface 完成後才能啟用
    Taifex(...)        // 只有 interface 完成後才能啟用
}
```

`EventSchema` 只固定 `None=0`、`TwseQuote=1`。未完成 interface 的 variant 不得
先分配 runtime discriminant；新增時必須更新 schema version。

### 8.2 `TwseQuoteAnnotations`

```text
TwseQuoteAnnotations {
    status_flags_raw: u8
    limit_flags_raw: u8
}
```

canonical event 只編碼兩個 raw bytes。typed fields 全部由 raw bytes 與
`TeralionTwseQuote` 純函式解碼，避免 raw／decoded state 不一致。

`status_flags_raw` typed view：

```text
trial                 = raw & 0x80 != 0
delayed_open          = trial && raw & 0x40 != 0
delayed_close         = trial && raw & 0x20 != 0
matching_method       = (raw & 0x10 != 0) ? Continuous : CallAuction
opening_marker        = raw & 0x08 != 0
closing_marker        = raw & 0x04 != 0
reserved_status_bits  = raw & 0x03
```

`limit_flags_raw` typed view：

```text
trade_limit   = (raw >> 6) & 0x03
best_bid_limit= (raw >> 4) & 0x03
best_ask_limit= (raw >> 2) & 0x03
instant_trend =  raw       & 0x03
```

前三組使用：

```text
0 = Normal
1 = LowerLimit
2 = UpperLimit
3 = Reserved
```

`instant_trend` 使用：

```text
0 = Normal
1 = VolatilityInterruptionDown  // 緩跌
2 = VolatilityInterruptionUp    // 緩漲
3 = Reserved
```

reserved value 不使 event payload 遺失；raw byte 繼續保存並產生 warning。typed
view 不建立 standalone status event，也不改寫 SessionPlan phase。

## 9. `EventSchema`

### 9.1 Envelope

```text
DomainEvent {
    instrument: InstrumentId
    trading_date: TradingDate
    source_format: SourceFormatId
    match_time: MatchTime
    source_sequence: Optional<u64>
    payload: EventPayload
}
```

event schema version 不重複存進每個 in-memory value 也可以，但 cache stream、
canonical bytes 與 run manifest 必須明確包含 `event_schema_version = 1`。

`trading_date` 加入 envelope 是為了：

- 驗證 event 沒有跨 source partition 洩漏。
- 正確表達 TAIFEX 夜盤歸屬。
- 讓 cache record 可獨立診斷。

它不取代 `match_time`，也不加入 `OrderingRule` 的第一排序鍵之前。

`source_sequence` 只有 interface 證實的 source counter 才能 `Some`。API page、
file line、worker ordinal、`received_at` 或自行產生的 sequence 一律禁止。

### 9.2 Payload variants

| Variant | Discriminant／ordering rank | Payload |
| --- | ---: | --- |
| `QuoteSnapshot` | 10 | `QuoteSnapshot` |
| `BookSnapshot` | 20 | `BookSnapshot` |
| `TradeBatch` | 30 | `TradeBatch` |

discriminant 與 ADR-0001 `event_kind_rank` 共用固定值。第一版不分配 standalone
status event discriminant。

### 9.3 `QuoteSnapshot`

```text
QuoteSnapshot {
    book: CompleteBookSnapshot
    trade: Observation<TradePrint>
    cumulative_volume: Observation<Volume>
    annotations: MarketAnnotations
}
```

規則：

- book 必定完整，不能 `NoObservation`。
- `trade=NoObservation` 表示該 tick 沒有新 deal，不清除 recent trade。
- `cumulative_volume=Set(0)` 是合法值。
- annotations 與 book／trade／volume 同一 event 原子更新。
- TWSE `open_price`／`high_price`／`low_price` 不在第一版 payload。

M1 `STOCK_SNAPSHOT` 必須映射至此 variant。

### 9.4 `BookSnapshot`

```text
BookSnapshot {
    book: CompleteBookSnapshot
    annotations: MarketAnnotations
}
```

它完整取代 book，不含 synthetic trade。TAIFEX mapping 只有在
[TAIFEX interface](../interfaces/taifex.md)由真實 fixture 固定後才能啟用。

### 9.5 `TradeBatch`

```text
TradeBatch {
    trades: NonEmpty<TradePrint>
    trade_order: TradeOrder
    cumulative_volume: Observation<Volume>
    annotations: MarketAnnotations
}
```

它不含 book，因此 reducer 必須保留既有 complete book。單筆 trade 使用長度 1
的 batch，不新增 `SingleTrade` event kind。

同一 source record 提供的 trades、cumulative volume 與 annotations 必須保留在
同一 `TradeBatch`，state version 只增加一次。

## 10. TWSE intermediate print

### 10.1 Type representation

TWSE `STOCK_REALTIME`：

```text
intermediate_print = true
deal               = present
bids               = []
asks               = []
```

在 type system 中只能表示為：

```text
TradeBatch {
    trades: [
        TradePrint {
            price
            quantity
            print_kind: Intermediate
        }
    ]
    trade_order: SourceOrdered
    cumulative_volume: Set(source_cum_volume)
    annotations: TwseQuote(raw flags)
}
```

它不是 `QuoteSnapshot`，也不清除或合成 book。這解決「trade observation +
NoBookObservation」的型別表達問題，同時維持第一版既有 event kind 集合。

### 10.2 Intermediate/final ordering

TWSE `STOCK_REALTIME` 的同 `match_time` intermediate/final pair 依
[ADR-0005](../architecture/decisions/0005-twse-intermediate-final-ordering.md)處理。

`OrderingRule` version 2 在 event kind 前先比較 source phase：

```text
intermediate TradeBatch phase rank = 10
final QuoteSnapshot phase rank     = 20
```

因此 reducer 先保存 intermediate trade/cumulative volume，再以 final
`QuoteSnapshot` 更新最後成交、final cumulative volume 與 complete book。

mapping version 2 只接受 fixture 已證實的一筆 intermediate + 一筆 final group；
其他 group shape 以 schema error 拒絕。不得使用 `max(cumulative_volume)`、忽略
intermediate、複製舊 book 或 API input order 掩蓋問題。

phase rank 可由既有 canonical event fields 純函式重建，所以
`event_schema_version` 與 `canonical_event_version` 維持 `1`。

## 11. Validation invariants

### 11.1 Event envelope

accepted event 必須：

- market、symbol、trading date 與 source partition identity 相符。
- source format 已在 registry 中支援。
- `match_time` 有效且位於 execution replay window。
- event schema／mapping／ordering dependency compatible。
- `source_sequence` presence 符合 interface。

### 11.2 Numeric

- price strictly positive。
- book／trade quantity strictly positive。
- cumulative volume non-negative。
- quantity units 在同一 book／batch／state update 中 compatible。
- decimal parse 及 arithmetic 無 precision loss／overflow。

### 11.3 Book

- 每側最多五個連續 present slots。
- bid price strictly descending。
- ask price strictly ascending。
- 不依 generic rule 拒絕 locked／crossed book。
- complete snapshot 到達時整體 replacement。

### 11.4 Observation

- `Clear` 只有 interface 明確定義時合法。
- `Unknown` 必須帶無損 raw scalar 與 deterministic warning context。
- required book 不接受 `NoObservation`。
- unknown nested shape／format 不接受包裝成 unknown event。

### 11.5 Event-specific

- `TradeBatch.trades` 非空。
- TWSE annotation raw bytes 與 typed decoding 必須一致。
- volatility interruption event 不因 type 本身產生 fill；fill behavior 由
  simulation policy 處理。

validation failure 不得留下 replay clock、MarketState 或 state version 的 partial
update。

## 12. `CanonicalEvent`

### 12.1 Primitive encoding

所有 multi-byte integer 使用 big-endian：

| Type | Encoding |
| --- | --- |
| `u8` | 1 byte |
| `u16` | 2 bytes unsigned |
| `u32` | 4 bytes unsigned |
| `u64` | 8 bytes unsigned |
| `i32` | 4 bytes two's complement |
| `i64` | 8 bytes two's complement |
| `i128` | 16 bytes two's complement |
| bool | `0x00` false、`0x01` true |
| string／bytes | `u32` byte length + exact bytes |
| optional | `0x00` absent、`0x01` present + payload |
| fixed array | slot 依 index 編碼，不加 length |
| vector | `u32` element count + elements |

禁止 variable-width integer、platform `usize`、locale string、JSON object key order
或 Rust serializer default。

### 12.2 Event frame

fixed field order：

```text
1. magic bytes              = "OSME"
2. canonical version        = u16(1)
3. event schema version     = u16(1)
4. market discriminant      = u8
5. symbol                   = string
6. trading_date epoch days  = i32
7. source_format            = string
8. match_time micros UTC    = i64
9. source_sequence          = optional<u64>
10. event kind              = u8
11. payload                 = variant-specific bytes
```

`received_at`、cursor、API page、source filepath、download time、warning text、
thread id、cache offset 與 runtime type name 不進 frame。

### 12.3 Payload encoding

`Price`：

```text
i128 decimal atoms
```

`Quantity`／`Volume`：

```text
u8 unit discriminant
u64 value
```

book side 每個五 slots：

```text
slot 0 optional<BookLevel>
...
slot 4 optional<BookLevel>
```

`Observation`：

```text
u8 observation discriminant
variant payload, if any
```

`MarketAnnotations`：

```text
u8 annotation discriminant
TwseQuote => status_flags_raw u8 + limit_flags_raw u8
```

event payload 依第 9 節 schema field order 遞迴編碼。任何新增 field、改變順序或
discriminant 都需要新的 canonical event／event schema version。

### 12.4 Fingerprint 與 equality

```text
EventFingerprint = BLAKE3-256(CanonicalEvent(event))
```

domain event equality 必須與 canonical significant fields 一致：

- canonical bytes 相同的合法 events domain-equal。
- 完全 duplicate events 仍是兩個 stream records，不因 equality 自動去重。
- source JSON whitespace、number spelling 或 object key order 不影響 equality。
- unknown raw value 不同會改變 canonical bytes。

event stream checksum 的 framing 與 final-state canonical encoding 由 replay／
market-state design 定義；不得直接 concat 無 length 的 variable records。

## 13. Public API boundary

### 13.1 Normalizer

輸入：

- validated source partition identity
- Teralion market／format wire record
- applicable mapping version

輸出：

```text
Accepted(DomainEvent)
KnownSkipped(reason)
Rejected(context)
```

normalizer 不回傳 partial event。`KnownSkipped` 只適用產品明確排除的
format／kind，例如 `INTRADAY_ODDLOT_REALTIME`；不能用於掩蓋 invalid supported
payload。

### 13.2 Replayer

replayer 只讀 `DomainEvent`：

- 不持有 wire JSON。
- 不解析 raw Teralion format。
- 只以 `MatchTime` 推進 clock。
- 依 `OrderingRule` 比較 full ordering key。

### 13.3 Strategy

strategy 收到 immutable event／MarketState view。public type：

- 不提供 interior mutability。
- 不暴露 source buffer ownership。
- typed flags view 是 pure read-only projection。
- derived best bid／ask／spread 不可反向修改 book。

`TradingContext` 由 session 與 market rules 在 event 更新後建立，不屬於 market
type、DomainEvent 或 canonical event bytes。

### 13.4 Simulation

simulation 必須檢查：

- quantity unit compatibility。
- session phase。
- source event 是否提供合法 fill evidence。
- TWSE trial、緩跌／緩漲及其他 annotations 的 versioned fill rule。
- `TradingContext.matching` 與 new-order-entry restriction。

market type 只表達 observation，不自行成交。

## 14. Error contract

至少區分：

| Error | Example |
| --- | --- |
| `InvalidIdentity` | market／symbol／trading date 與 partition 不符 |
| `UnsupportedFormat` | registry 無 mapping |
| `InvalidMatchTime` | parse、precision、range 或 replay window error |
| `InvalidDecimal` | precision loss、zero price、overflow |
| `InvalidQuantity` | zero display quantity、overflow、unit mismatch |
| `InvalidBook` | depth、slot continuity、price ordering error |
| `InvalidObservation` | required field 使用 `NoObservation`、不合法 `Clear` |
| `UnknownReservedValue` | raw 保存但產生 warning；是否 reject 由 requirement policy |
| `IncompatibleVersion` | event／mapping／ordering／canonical version mismatch |
| `UnsupportedRealtimeMatchGroup` | TWSE realtime group 缺 final、多筆 intermediate/final 或 cumulative volume 不一致 |

error context 至少包含 market、symbol、trading date、source format 與可用的
`match_time`。不得包含 API key 或完整 credential-bearing request。

## 15. Verification

### 15.1 Primitive tests

- ISO-8601 offset normalization 與 microsecond boundary。
- sub-microsecond non-zero rejection。
- decimal equivalent lexical forms 產生相同 atoms。
- decimal precision／overflow negative tests。
- price zero／negative rejection。
- quantity zero、volume zero 與 unit mismatch tests。
- symbol exact bytes／leading-zero test。

### 15.2 Book／observation tests

- 0 至 5 levels complete side。
- non-contiguous slots、duplicate price、wrong bid／ask order rejection。
- locked／crossed book 不由 generic validator 誤拒絕。
- `Set`／`Clear`／`NoObservation`／`Unknown` distinct encoding。
- `deal=null` 對應 `NoObservation`。
- `cum_volume=0` 對應 `Set(Volume(0))`。

### 15.3 Event tests

- TWSE `STOCK_SNAPSHOT` golden `QuoteSnapshot`。
- 同 source tick 的 book／trade／volume／flags 單一 atomic event。
- `TradeBatch` non-empty invariant。
- trade-only intermediate 不可建構 `QuoteSnapshot`。
- TWSE realtime `1+1` group 產生 intermediate `TradeBatch` 與 final
  `QuoteSnapshot`。
- source phase rank 固定 intermediate 在同 `match_time` final 前。
- missing／multiple／volume-mismatch realtime group 整組拒絕。
- unknown format 不能產生 event。
- `close`／`stats` 不產生 timeline event。

### 15.4 Flags tests

- `status_flags` 4／8／16／128 typed decoding。
- combined status bits independent decode。
- `limit_flags=0` 全部 normal。
- 緩跌 `raw & 0x03 == 1`、緩漲 `== 2`。
- reserved bits raw preservation 與 warning。
- typed view re-derivation 不可能與 raw bytes 分歧。

### 15.5 Canonical golden tests

- 每個 primitive／enum discriminant golden bytes。
- M1 fixture canonical event bytes 與 BLAKE3 fingerprint。
- JSON number spelling／key order／whitespace 不改變 bytes。
- unknown raw value 改變會改變 fingerprint。
- duplicate events canonical-equal 但不被去重。
- schema／canonical version mismatch 拒絕。
- x86_64／aarch64、debug／release、不同 locale／timezone golden values 一致。

## 16. Compatibility 與 delivery

### 16.1 M1

M1 只需要實作：

- primitive identities／time／decimal／quantity
- complete five-level book
- `QuoteSnapshot`
- `TwseQuoteAnnotations`
- `Observation`
- `CanonicalEvent`
- `STOCK_SNAPSHOT` fixture normalization

不需要為 M1 實作 `TradeBatch` runtime path、simulation quantity conversion 或
TWSE intermediate ordering。

### 16.2 M2

M2 加入：

- cache serialization 與 lineage
- complete 2330 session source normalization
- trading-unit-size metadata 與 simulation quantity conversion
- market／limit fill evidence

完整 `STOCK_REALTIME` 使用 `TeralionTwseQuote` mapping version 3 與
`OrderingRule` version 2；未知 match-group shape 必須拒絕。

### 16.3 M3／M4

TAIFEX、TPEx、warrant 與 option 只能在各 interface 由真實 fixture 固定後：

- 啟用對應 annotation variant。
- 確認 Teralion wire quantity 對官方 market unit 的 mapping、price precision
  與 instrument metadata。
- 增加 mapping／canonical golden vectors。
- review event／canonical compatibility。

## 17. Traceability

- `DATA-04`：event schema／canonical version 與 cache compatibility。
- `DATA-05`：instrument identity、kind、date 與 optional reference fields。
- `REPLAY-01`：標準事件、wire/domain 分離、atomicity。
- `REPLAY-02`：exact `MatchTime`、ordering fields 與 canonical fingerprint。
- `REPLAY-03`：complete book、trade、volume、flags 與 observation semantics。
- `REPLAY-06`：invalid／unknown／unsupported error contract。
- `SIM-01`：quantity unit 與 fill-evidence boundary。
- `SIM-02`：exact decimal 與 multiplier／currency provenance boundary。
- `OPS-02`：schema／canonical version provenance。
- `NFR-01`：跨平台 deterministic canonical bytes。
- `NFR-03`：version compatibility 與 cache rebuild boundary。
