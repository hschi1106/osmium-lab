# 004：統一 Continuous / CallAuction 市場狀態模型

Status: done
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

- [x] 一個 neutral model 表達必要情境。
- [x] reducer 是跨事件 state 唯一 owner。
- [x] strategy/sim 不解 raw provider flags。
- [x] event matching 與 post-state 分離。
- [x] core cases 不依賴 Teralion fixture。
- [x] provider mapping 只在 provider boundary。
- [x] 舊重複 taxonomy/evaluator 已刪或有不同語義理由。
- [x] replay/simulation/scheduled regression 通過。
- [x] fmt/test/clippy 通過。
- [x] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：`ea18253` / clean。
- Rust LOC before：`~/cloc/cloc --include-lang=Rust crates`：111 files / 3,596 blank / 267 comment / 41,769 code。
- Replaced concepts：刪除 `IndicativeAuctionKind`／`StabilityDirection` 與 `IndicativeReason`；TWSE／TPEx 分開的 TradingContext evaluator 收斂為單一 `MarketTradingContextEvaluator`。market-state、strategy-api、execution-sim 不再解碼 `MarketAnnotations` raw flags；raw wire mapping 僅保留在 `providers/teralion` boundary。
- New minimal contract：新增 `AuctionPurpose`、`AuctionObservation`、`MarketSignal` 與 reducer-owned `MarketPhase`／`AuctionState`。`QuoteSnapshot`、`BookSnapshot`、`TradeBatch`、`MarketStatus` 以 `Observation<MarketSignal>` 攜帶 neutral signal；`IndicativeAuction` 直接攜帶 `AuctionObservation`。reducer 明確處理 NoObservation／Unknown／Set、delayed reassertion、opening／closing／periodic／VI uncross post-state、session reset 與 multi-instrument isolation。
- Validation：`cargo fmt --check`、`cargo test --workspace`（324 passed / 54 suites）、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 均 exit 0。新增 provider-neutral synthetic tests：market-state 7 cases、strategy-api 4 cases；Teralion TWSE／TPEx／TAIFEX fixture regression 全部通過。fixture generation、compact/bundle/license verifier 與 acceptance Python tests（8 passed）通過。release CLI／fixture builder build 通過；無 `TERALION_API_KEY` 的 offline `data verify`、`cache prepare`、`replay`、`backtest`、`inspect` 通過。Goal 004 smoke replay 2 events 的 `event_checksum=a796e6ab3494f338254ef7312e0a30e25964288f2cc3a62d906456f9022a7239`、`final_state_checksum=b03ad9a67e8f0590ce572038913afb4806ea1a12e0441f1e1736488d5028733b`。
- Rust LOC after / delta：`~/cloc/cloc --include-lang=Rust crates`：113 files / 3,637 blank / 270 comment / 42,008 code；相較 before 為 `+2 files / +41 blank / +3 comment / +239 code`。
- Breaking versions/checksums：`MARKET_TYPES_VERSION` 7→8、event schema/canonical 6→7、`MARKET_STATE_VERSION` 5→6、state reducer 4→5、canonical market/final state 5→6；TWSE mapping 9→10、TPEx mapping 6→7、TradingContext rule 1→2。event／state canonical frame 新增 market signal／phase，故 replay event、final-state 與 cache identity checksum 改變，舊 derived cache 必須重建。
- 剩餘風險：Teralion 目前沒有足夠來源證據可填 `disposal=true`，因此 provider mapping 不臆造處置欄位；neutral synthetic contract 已覆蓋 disposal／delayed combination。VI trial 仍保守映射為 `Periodic`，只有已證實的 status-only trigger 映射為 `VolatilityInterruption`。
- 下一步：005-close-market-background-gap.md
