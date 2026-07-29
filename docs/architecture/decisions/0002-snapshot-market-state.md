# ADR-0002：以完整 snapshot 取代方式維護唯讀市場狀態

- 狀態：Accepted
- 決策日期：2026-07-29
- 最後修訂：2026-07-30（移除第一版不使用的統計狀態）
- 適用版本：`MarketStateSemanticsV1`
- 主要需求：`REPLAY-01`、`REPLAY-03`、`REPLAY-04`、`STRAT-01`

## 1. Context

Teralion 歷史資料提供 trade、trade batch、完整最佳五檔 snapshot、累計量及
flags，但不是逐筆委託資料。TAIFEX 另有 `close`／`stats` source records；第一版
backtest 不使用它們，因此不把它們正規化為 event 或保存進 MarketState。

系統無法從這些資料可靠得知：

- 每張委託。
- 新增、修改或取消委託的真實 sequence。
- queue position。
- hidden liquidity。
- exchange matching 過程。

若把連續五檔 snapshot 的差異解讀為 order-level delta，會製造來源不支持的精度，
並讓 fill model 看似比資料更真實。

同時，strategy 需要一個容易理解、可重現、無前視且不可修改的 current market view。

## 2. Decision

每個 execution universe instrument 維護一份獨立 `MarketStateV1`。狀態只由 accepted
domain events 經 reducer 更新，並以完整 snapshot replacement 作為 book 語意。

概念狀態：

```text
MarketStateV1 {
    instrument
    book_snapshot
    recent_trade_or_batch
    cumulative_volume
    raw_flags_and_known_status
    last_match_time
    state_version
}
```

這是 logical schema，不固定 Rust field names 或 container types。

### 2.1 State identity

MarketState 以可區分 market 與 symbol 的 domain `InstrumentId` 為 identity。

- 相同 symbol、不同 market 是不同 state。
- strategy universe 外 instrument 不建立可見 state。
- trading date 是 source partition／execution context，不取代 instrument identity。
- session 或 trading-date boundary 是否 reset 某欄位，必須由明確 market semantics
  與 replay plan 決定，不能依日曆午夜猜測。

### 2.2 Initial state

第一個 accepted event 前：

- `book_snapshot` unavailable。
- recent trade／batch unavailable。
- cumulative volume unavailable。
- flags／status unavailable。
- `last_match_time` unavailable。
- `state_version = 0`。

unavailable 不以零、空 book、昨日最後值或日終統計替代。

## 3. 五檔 snapshot replacement

### 3.1 Complete snapshot

domain book snapshot 表達來源在該 `match_time` 提供的完整最佳五檔 view：

- bid side 最多五個明確 slots。
- ask side 最多五個明確 slots。
- 無該檔位以 explicit empty／absent slot 表達。
- 每個存在 level 包含經驗證的 price 與 displayed quantity。

「完整」表示事件已完整表達來源當下所有五檔 slots，不表示每側必定有五個非空
levels。

### 3.2 Replacement rule

收到帶有完整 book 的 accepted event 時：

```text
new_state.book_snapshot = event.complete_book_snapshot
```

不得：

- 將舊 snapshot 中新事件未包含的 level 留在 book。
- 把新舊 displayed quantity 差異解讀為 order add／cancel／fill。
- 從 snapshot 推論 queue position。
- 合成來源未提供的第六檔以上。
- 合併不同 `match_time` 的 levels 成為一個 book。

### 3.3 Derived views

best bid、best ask、spread、mid 等可以由目前 snapshot 即時計算。

derived view：

- 不是較高精度的獨立市場事實。
- 不可反向修改 snapshot。
- 不應作為另一份可能與 snapshot 不一致的 mutable state。
- 缺少必要 level 時回傳 unavailable，不以零或上一價替代。

## 4. Event-to-state semantics

### 4.1 `QuoteSnapshot`

一次 `QuoteSnapshot` 原子套用：

- 完整 book replacement。
- event 中明確存在的 trade observation。
- event 中明確存在的 cumulative volume。
- event 中明確存在的 raw flags／known status。
- `last_match_time`。
- state version。

同一 source tick 的 book、trade、volume 與 flags 不拆成多次 transition。

### 4.2 `BookSnapshot`

一次 `BookSnapshot`：

- 完整取代 book。
- 更新同一 event 中明確提供且 schema 支援的 flags。
- 不從 book difference 產生 synthetic trades。

### 4.3 `TradeBatch`

一次 `TradeBatch`：

- 保存完整 batch 作為 recent trade observation。
- 可以提供由 batch 直接取得的 recent individual trade view；若順序由來源確認。
- 更新 event 明確提供的 cumulative volume。
- 不修改 book；除非同一原子 event schema 本身包含完整 book。

batch 內多筆成交仍只造成一次 MarketState transition。

### 4.4 `MarketStatus`

只有 interface mapping 明確確認的 status 語意可以更新 known status。

未知 flags：

- 保存 raw value 或無損 representation。
- 產生 warning。
- 不觸發未確認的 session、limit、halt 或 auction 行為。

## 5. Optional field update semantics

Reducer 不得從 `Option`、零值或欄位缺漏自行猜測 update 語意。normalizer／domain
event 必須明確區分：

| 語意 | State action |
| --- | --- |
| `Set(value)` | 以新值取代 |
| `Clear` | 明確設為 unavailable；只有來源語意證實時 |
| `NoObservation` | 保留最近已觀察值，不聲稱目前 event 更新了它 |
| `Unknown(raw)` | 保存 raw、產生 warning，不套用未知 domain 語意 |

具體 enum／type 由 [market types 設計](../../design/market-types.md)決定；上述四種
語意不可被單一模糊 nullable field 混在一起。

book snapshot 不使用 `NoObservation` 合併 individual levels：只要 event 宣告帶有
完整 book，就依第 3 節整體 replace。

## 6. Atomic reducer

### 6.1 Conceptual operation

Reducer 概念上執行：

```text
reduce(current_state, accepted_event)
-> validate preconditions
-> build complete proposed_state
-> validate proposed_state invariants
-> commit proposed_state once
```

不要求實作一定 clone 整份 state；只要求任何 observer 看不到 partial update。

### 6.2 Preconditions

至少驗證：

- event instrument 等於 target state instrument。
- event schema／kind 受支援。
- event `match_time` 合法。
- event ordering 不早於 `last_match_time`。
- price、quantity、book shape 及 payload 已符合 domain invariant。
- update semantics 可安全套用。

market-specific crossed book、auction 或特殊 flags 是否合法，由 interface／normalizer
fixture 決定，不能由通用 reducer 以一般連續撮合假設拒絕。

### 6.3 Failure atomicity

任何 precondition 或 proposed-state validation 失敗時：

- MarketState 不改變。
- `state_version` 不改變。
- strategy callback 不發生。
- replay clock 不留下 observer 可見的 advance。
- run 依 `REPLAY-06` failed；或在能隔離 event 且 explicit degraded policy 允許時
  由 replay layer 決定，但不得套用 partial state。

Replay Engine 可以先驗證再推進 clock，或以 transaction-like coordination 回復；
對外可觀察結果必須相同。

## 7. State version

`state_version`：

- 初始為 0。
- 每個 accepted event 成功 commit 後增加 1。
- 同一原子 event 只增加 1。
- event 即使沒有改變某個顯示值，仍增加 1，因它是新的已處理 observation。
- 完全 duplicate event 仍各增加 1；ordering layer 不自動去重。
- 不因 strategy read、derived view 或 simulation read 增加。
- overflow 必須明確失敗，不可 wrap。

state version 是 callback／trace identity 的一部分，但不代表 exchange sequence。

## 8. Read-only visibility

處理順序：

```text
select event
-> advance clock
-> commit state transition
-> create immutable state view
-> invoke strategy
```

strategy：

- 看到目前 event 已套用後的 state。
- 可以讀 universe 內其他商品截至其最後已處理 event 的 state。
- 不能取得 mutable reducer handle。
- 不能修改 `last_match_time`、version、book 或任何歷史 observation。
- 不能讀下一 event。

Simulation 可以讀目前 event／state 判定既有 orders，但同樣不能修改 MarketState。

唯讀邊界必須由 Rust ownership／type system 或等價 compile-time boundary 強制，
不能只靠 convention。

## 9. Same-time 與 multi-symbol semantics

相同 `match_time` 依
[ADR-0001](0001-match-time-ordering.md)逐一處理：

```text
event A -> state A -> callback A
event B -> state B -> callback B
```

callback A 看不到 B。callback B 可以看到 A 對其商品或其他商品已造成的狀態。

每個 instrument 有獨立 state version。平台可以另有 run-level event ordinal，但
不能拿它冒充 source sequence。

## 10. State checksum

final-state checksum 以所有 universe states 的 canonical encoding 計算。

canonical state：

- 先依 `InstrumentId` 的固定 order 排列。
- 每個 state 依 schema field order 編碼。
- 包含 current domain values、`last_match_time` 及 state version。
- 明確編碼 unavailable、unknown raw values 及 empty book slots。
- 不包含 memory address、cache、lazy derived view、log 或 wall clock。

state schema／canonical encoding 改變時必須更新 version。具體 bytes 由
[market types](../../design/market-types.md)及
[market state 設計](../../design/market-state.md)固定。

## 11. Consequences

### 11.1 正面結果

- 狀態忠於 Teralion snapshot 精度。
- reducer 規則簡單且可由 fixture 單元測試。
- strategy 取得一致的 current view。
- 不需要保存或維護虛構 order-level book。
- final state 可以由 ordered events 完整重建。
- concurrency 可以發生在 event boundary 外，不改變 sequential state semantics。

### 11.2 成本與限制

- 無法模擬真實 queue priority。
- snapshot 間的 order activity 不可知。
- 每個 accepted event 都增加 version，包括 duplicate。
- `NoObservation` 與 `Clear` 必須在 domain event 中明確表達，types 較單純 `Option`
  更嚴格。
- 策略若需要 bars／indicators，必須在 strategy 自己的狀態中由已看見 events 建立。

### 11.3 Fill model 影響

simulation 只能把 snapshot 顯示量當成明確命名模型的估算限制。它不能因 MarketState
保存五檔就宣稱知道 queue position 或真實可成交量。

## 12. Considered alternatives

### 12.1 由 snapshot 差異重建 order book

拒絕。缺少 order identity、cancel、priority 及 hidden liquidity，重建結果不可驗證。

### 12.2 逐 level merge 新舊 snapshot

拒絕。新 snapshot 已是完整 view，merge 會保留已不存在的 stale levels。

### 12.3 Strategy 直接維護共享 MarketState

拒絕。會破壞 deterministic reducer、唯讀邊界及多 strategy isolation。

### 12.4 只保存 raw latest event，不建立 typed state

拒絕。每個 strategy 會重複解讀 source semantics，並讓 wire format 穿透 domain
boundary。

### 12.5 每個 payload field 分別 callback

拒絕。會拆開同一 source tick，產生來源不存在的中間市場狀態與時間點。

### 12.6 未變更值不增加 version

拒絕。state version 也代表 accepted observation sequence；以值比較決定版本會讓
duplicate、warning 或 non-book event 的 trace 變得不一致。

## 13. Verification

至少需要：

- initial unavailable state test。
- `QuoteSnapshot` 完整 book replacement test。
- empty level 清除舊 level test。
- trade／volume／flags 同一原子 transition test。
- `TradeBatch` 只增加一次 version test。
- `Set`／`Clear`／`NoObservation`／`Unknown` semantics tests。
- reducer failure 不留下 partial state／clock test。
- strategy 只能取得 read-only updated view 的 boundary test。
- same-time events 逐一 callback test。
- duplicate events 各增加 version test。
- final-state canonical checksum golden test。

M1 對應 `M1-AC-02`、`M1-AC-05`、`M1-AC-06` 與 `M1-AC-09`；M3 補
`TradeBatch`、`BookSnapshot` 及 multi-symbol state。

## 14. Traceability

- [產品需求](../../product-requirements.md)：`REPLAY-01`、`REPLAY-03`、`REPLAY-04`
- [回播需求](../../requirements/replay.md)：event atomicity、snapshot semantics、
  state transition、no-lookahead
- [策略需求](../../requirements/strategy.md)：read-only MarketState
- [模擬需求](../../requirements/simulation.md)：observable snapshot 與 queue 限制
- [M1 增量](../../increments/M1-twse-replay.md)：QuoteSnapshot reducer 驗收
- [資料與執行流程](../data-flow.md)：per-event sequence
