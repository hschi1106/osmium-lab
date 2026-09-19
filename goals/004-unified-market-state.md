# 004：統一 Continuous / CallAuction 市場狀態模型

Status: pending
Depends on: 003-decouple-teralion-provider.md

## 目標

以 provider-neutral semantics 表達 TWSE/TPEx 股票的 opening、closing、delayed opening/closing、periodic/disposal auction、delayed periodic auction、intraday volatility interruption。

不要為每個名字建立獨立 event/handler/state machine。

成功路徑：

```text
provider-neutral DomainEvent
-> MarketState reducer
-> TradingContext
-> strategy / execution simulation
```

並刪既有重複 taxonomy/raw-flag decoding path。

## 抽象

### Matching mechanism

重用 `MatchingMethod::{Continuous, CallAuction}`。Unknown/Closed 不是第三種 matching method。

### Auction purpose

```rust
AuctionPurpose {
    Opening,
    Closing,
    Periodic,
    VolatilityInterruption,
}
```

### Delay

延後是同一輪 auction observation 的屬性，不建立 `DelayedOpening/Closing/Disposal` variants。

### Disposal identity

與 purpose 正交：

```text
處置股開盤     = Opening + disposal=true
處置股盤中分盤 = Periodic + disposal=true
處置股收盤     = Closing + disposal=true
```

## Event 與 state

保留 `QuoteSnapshot`、`BookSnapshot`、`TradeBatch`、`MarketStatus`、`IndicativeAuction`。

收斂 neutral market signal，概念：

```rust
MarketSignal {
    Continuous,
    AuctionCollecting(AuctionObservation),
    AuctionUncross(AuctionObservation),
    Closed,
}
```

`AuctionUncross` 是正式 observation semantic，不新增人工 event stream。

### Observation

必須區分：

```text
NoObservation
Unknown
Set(value)
```

不能壓成 bool/Option 混用。

### State

概念：

```rust
MarketPhase {
    Continuous,
    Auction(AuctionState),
    Closed,
}
```

MarketState 是唯一跨事件 reducer owner。

不另存可推導的 `is_auction`、`is_continuous`、`is_delayed_open` 等 flags。

## Reducer 規則

- Continuous 結束先前 auction context。
- AuctionCollecting 更新同輪 auction。
- repeated delayed=true 是 reassertion，不是 counter。
- Closed 不清最後 firm observation。
- new auction/trading date/session 正確 reset。
- 不看未來回填 earlier state。
- 不以 wall-clock 推測 auction 結束。

### Auction result vs post-state

```text
Opening result: event matching=CallAuction; post-state=intraday regime/evidence
Closing result: event matching=CallAuction; post-state=Closed
Periodic result: event matching=CallAuction; post-state=next Periodic/evidence
```

不能用 post-state 改本次 fill eligibility。

## Strategy / execution

只讀 neutral MarketState/TradingContext，不讀 provider/raw flags。

一般與 scheduled execution 共用 matching/order-entry/auction context。

Indicative price/book 不當 firm fill evidence。

看到 result 後建立的 order 不可回填同次 auction。

## Provider mapping

Teralion mapping 只在 `providers/teralion`。可映射既有證據，但不為湊案例發明欄位。

六種核心情境用 neutral synthetic Rust input 驗。

## 不做

- 不重算 exchange trigger threshold。
- 不寫 resume scheduler。
- 不重建 queue/order book。
- 不加 IOC/FOK。
- 不建 FSM/rule engine/provider framework/profile service。

## 必要案例

1. normal opening -> uncross -> Continuous
2. delayed opening -> uncross -> Continuous
3. normal closing -> Closed
4. delayed closing 跨 nominal close
5. periodic -> next Periodic
6. delayed periodic -> next Periodic
7. disposal + delayed open/close
8. volatility interruption trigger -> trial -> formal result
9. repeated delay 不增加 phantom round
10. NoObservation vs Unknown
11. indicative 不覆寫 firm
12. post-result new order 不回填
13. multi-instrument isolation
14. replay determinism

## 應收斂的重複概念

執行前搜尋並判讀 `IndicativeAuctionKind`、`IndicativeReason`、market-specific TradingContext evaluator 中 purpose/delay 判斷、core 對 raw annotations 的直接解碼。

不能三套 taxonomy 並存。

## 驗收

- [ ] 一個 neutral model 表達必要情境。
- [ ] reducer 是跨事件 state 唯一 owner。
- [ ] strategy/sim 不解 raw provider flags。
- [ ] event matching 與 post-state 分離。
- [ ] core cases 不依賴 Teralion fixture。
- [ ] provider mapping 只在 provider boundary。
- [ ] 舊重複 taxonomy/evaluator 已刪或有不同語義理由。
- [ ] replay/simulation/scheduled regression 通過。
- [ ] fmt/test/clippy 通過。
- [ ] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- Replaced concepts：
- New minimal contract：
- Validation：
- Rust LOC after / delta：
- Breaking versions/checksums：
- 剩餘風險：
- 下一步：005-close-market-background-gap.md
