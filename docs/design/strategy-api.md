# Strategy API 設計

## 1. 文件目的

本文件定義 `osmium-lab` 第一版 strategy logical API、生命週期、唯讀市場邊界、
deterministic output 與失敗語意。目標是讓 strategy 能在不依賴 Teralion wire
type、不改寫 replay state、也不取得未來資料的前提下，接收事件後狀態並產生可
重現的 observation；後續 execution simulation 再沿同一邊界接收 order intent
與回傳 feedback。

本文件固定：

```text
strategy_api_version    = 1
strategy_output_version = 1
```

本文件定義 logical contract，不固定 crate、module、trait 名稱、泛型參數或
serialization library。Rust 實作可以調整型別排列，但 public semantics、callback
順序、錯誤處理及 canonical output 必須符合本文件。

依據：

- [產品需求](../product-requirements.md)
- [Strategy 需求](../requirements/strategy.md)
- [User guide](../user-guide.md)
- [Replay engine 設計](replay-engine.md)
- [MarketState 設計](market-state.md)
- [ADR-0003：Session window 與 strategy activation](../architecture/decisions/0003-session-windows-and-strategy-activation.md)
- [ADR-0004：TradingContext 與 eligibility](../architecture/decisions/0004-trading-context-and-eligibility.md)

## 2. 邊界與責任

### 2.1 Strategy 可以做的事

strategy 可以：

- 在 replay plan 建立前，驗證參數並宣告完整 universe。
- 在 callback 讀取目前 event、已 commit 的目前商品 `MarketState`、
  `TradingContext` 與 session context。
- 修改自己的私有記憶體。
- 依 emission 順序輸出 indicator。
- 在 execution simulation 啟用後，輸出 order intent。
- 在 simulation stage 完成後，接收可追溯 feedback。
- 在 replay 正常結束時輸出 deterministic summary。

### 2.2 Strategy 不可以做的事

strategy 不得：

- 修改 `DomainEvent`、`ReplayClock`、`MarketState`、`TradingContext`、
  source data、replay cache 或其他 strategy 的狀態。
- 取得目前 event 之後的 event、state 或 feedback。
- 在執行中擴張 universe，迫使 replayer 開啟未宣告 stream。
- 直接解讀 Teralion JSON、cursor、`received_at` 或 raw status bits。
- 使用未記錄的 wall clock、process state、thread scheduling、filesystem、
  network、environment variable 或 entropy 作為決策輸入。
- 以 `f32`／`f64`、unordered collection iteration 或 serializer default
  產生 reproducibility-critical output。
- 將 callback-scoped view 保存至 callback 之外。

strategy-owned mutable state 與 replay-owned state 必須是不同 ownership
boundary。`&mut self` 只代表 strategy 私有狀態可變，不代表 market view 可變。

## 3. Strategy identity 與參數

### 3.1 `StrategyIdentity`

每個 run 必須保存：

```text
StrategyIdentity {
    strategy_id: Utf8String
    strategy_version: Utf8String
    binary_identity: BinaryIdentity {
        algorithm: Utf8String
        digest: bytes
    }
}
```

規則：

- `strategy_id` 在 repository／deployment scope 內穩定且不可為空。
- `strategy_version` 是 strategy author 明示的版本，不從 Rust type name 推導。
- `binary_identity` 是可回溯 build artifact 的 digest 或等價 immutable
  identifier；不得只記 branch name 或 dirty working tree label。
- 三者共同參與 run provenance；同一 `strategy_id` 的不同 binary 不可被當成
  相同 strategy execution。

### 3.2 Parameter schema 與 canonical config

strategy definition 提供 versioned parameter schema。執行前必須：

1. 拒絕 unknown、缺少、型別錯誤或超出範圍的參數。
2. 套用由 schema 明定的 default；不得依 serializer library 隱含 default。
3. 將參數正規化為 `CanonicalStrategyParams`。
4. 在 universe 宣告前 freeze；執行中不可修改。

price、ratio、money 或其他 exact decimal 使用
[Market Types](market-types.md) 的 `Decimal`；reproducibility-critical
參數不得先轉成 binary float。

parameter schema 必須提供 deterministic canonical encoding；不得依賴 object
insertion order、Rust `Debug` 或 serializer default。run 保存：

```text
canonical_params_checksum =
    BLAKE3-256(CanonicalStrategyParams bytes)
```

canonical encoding 改變時必須更新 strategy version 或 parameter schema version。

## 4. Universe 宣告

### 4.1 `StrategyDeclaration`

```text
StrategyDeclaration {
    universe: OrderedSet<InstrumentId>
    requested_sessions: OrderedSet<SessionKind>
}
```

宣告必須在 `ReplayPlan` 建立與 stream open 之前完成。`universe`：

- 至少一個商品。
- 去除 duplicate 後以 `InstrumentId` canonical order 排序。
- 每個商品都必須可由 session plan 與 source catalog resolve。
- M1 固定為 `TWSE / 2330` 與 `regular` session。

`requested_sessions` 是 semantic session identity，不是 strategy 自訂任意時間
範圍。實際 replay window 由 market calendar 決定，固定為 session
`[O - 5m, C + 5m)`；strategy 不得用自己的時間範圍截斷 source stream。

若任何 instrument 或 session 無法 resolve，plan 建立失敗；不得在 replay
途中忽略。

### 4.2 Stream selection

replayer 只開啟 declaration universe 與 requested sessions 所需的 streams。
strategy API 不提供 runtime `subscribe`。未宣告商品即使存在本地資料，也不得
進入 callback 或可查詢 market state。

## 5. 生命週期

第一版生命週期依序為：

```text
validate_params
-> declare
-> build
-> initialize
-> on_event * N
-> on_feedback * M
-> finalize
```

其中 `on_feedback` 只在 simulation stage 產生 feedback 時發生；M1 的
`M = 0`。

### 5.1 `initialize`

`initialize` 在所有 streams 成功開啟、但第一個 event 尚未 commit 前呼叫一次。
context 只包含：

- strategy identity 與 canonical params。
- frozen universe。
- session plan。
- version manifest。
- deterministic run identifier。

它不得包含第一個 event、第一個 state 或 source iterator。初始化失敗時，run
在處理任何 event 前以 `Failed` 結束。

### 5.2 `on_event`

每個 accepted event 恰好呼叫一次：

```text
StrategyEventContext<'event> {
    occurrence: EventOccurrence
    event: &'event DomainEvent
    market_state: &'event MarketStateView
    trading_context: &'event TradingContext
    session: &'event SessionCallbackContext
}
```

context 的 reference 只在該 callback 有效，不得回傳、保存或轉移至 background
task。public API 不提供 replay-owned state 的 `Arc<Mutex<_>>`、raw pointer、
interior-mutability handle 或 mutable reference。

`event` 與 `market_state` 屬於同一 instrument。`market_state` 是目前 event 已
套用後的版本，且：

```text
market_state.version == occurrence.instrument_state_version
market_state.match_time == event.match_time
```

在 M1，strategy 只能讀取目前 instrument 的 state。未來若加入
`UniverseStateView`，所有其他商品的 view 也只能是截至目前 replay occurrence
已 commit 的版本，且必須透過新的 API version 引入；不得加入 `next`、peek 或
future iterator。

### 5.3 `on_feedback`

simulation 完成一個 event 的 validation、fill、accounting 後，才可依
[ADR-0004](../architecture/decisions/0004-trading-context-and-eligibility.md)
呼叫 `on_feedback`。feedback 必須帶：

- 產生 feedback 的 replay occurrence。
- order／fill／accounting identity。
- exact quantities、prices 與 reason code。
- simulation model 與 accounting version。

feedback callback 看到的資訊不得早於 simulation 實際產生時點。由 feedback
產生的新 intent，其 origin 是該 feedback occurrence，最早只能由之後的 eligible
event 評估，不得回填至同一 event。

M1 不建立 simulation，也不呼叫 `on_feedback`；完整 payload 由
`execution-sim.md` 固定。

### 5.4 `finalize`

只有 stream 正常耗盡且 strategy 未失敗時，才呼叫 `finalize` 一次。context
可包含：

- 最後已 commit 的 replay clock。
- 每個已宣告商品的唯讀 final state。
- processed event／callback count。
- strategy 私有累積結果。

`finalize` 可輸出 summary indicator，但不可產生 order intent。final state 只在
replay 已結束後可見，不能回寫或改變先前 callback output。若沒有 event，
final state set 為空，clock 明示 `NoEventProcessed`。

## 6. 每個 event 的處理順序

第一版順序固定為：

```text
1. ReplayEngine 選出下一個 DomainEvent
2. MarketState reducer commit event
3. derive TradingContext
4. Strategy.on_event
5. validate and create new order intents
6. evaluate previously pending orders
7. commit fills and accounting
8. Strategy.on_feedback
```

M1 只執行 1–4；5–8 回報 `SimulationStage::NotUsed`。strategy callback 因此
永遠看到 post-event state，但任何由目前 callback 產生的 intent 都不能用目前
event 成交。

pre-open trial、pre-close trial 或緩漲／緩跌等狀態，strategy 仍接收 callback；
是否可下單、是否可成交由 `TradingContext` 的 typed eligibility 決定。strategy
不得自行以 `status_flags` 猜測。

## 7. Output sink

### 7.1 Transactional callback output

`on_event` 與 `on_feedback` 透過 callback-scoped sink 輸出：

```text
StrategyOutputSink {
    emit_indicator(Indicator)
    emit_order_intent(OrderIntent)  // M2 起可用
}
```

sink 先緩存在目前 callback 的 transaction：

- callback 成功才 commit output batch。
- callback 回傳 error 或 panic 時，丟棄該 callback 尚未 commit 的全部 output。
- 已 commit 的 replay event 與 MarketState 不 rollback；run 以 `Failed` 結束。
- 先前 callback 已 commit 的 output 保留為 failure diagnostics，不得標示成成功
  run。

同一 callback 的 output order 就是 `emit_*` 呼叫順序。strategy 若從 map 或 set
產生多筆 output，必須先按明確 canonical key 排序。

### 7.2 `Indicator`

```text
Indicator {
    name: Utf8String
    value: IndicatorValue
}

IndicatorValue =
    Bool(bool)
  | Signed(i64)
  | Unsigned(u64)
  | Decimal(Decimal)
  | Text(Utf8String)
```

規則：

- `name` 非空、UTF-8，並在單一 strategy version 中維持穩定語意。
- 不接受 `f32`／`f64`、NaN、infinity、arbitrary JSON 或 serializer-specific
  number。
- `Text` 是 exact UTF-8 bytes，不做 locale normalization。
- engine 為每筆 committed output 加上 occurrence、state version、
  `output_sequence` 與 strategy identity；strategy 不得自行偽造。

### 7.3 `OrderIntent` 邊界

strategy API 只擁有「strategy 可以表達 intent」的邊界；price type、quantity、
time-in-force、eligibility validation、pending order 與 rejection reason 的完整
schema 由 `execution-sim.md` 決定。

在該文件完成前：

- M1 sink 的 `emit_order_intent` 必須不可用或明確回傳
  `CapabilityUnavailable`。
- 不得以暫時 JSON、generic map 或 market-specific flags 穿越邊界。
- 新增 intent schema 需要 compatibility review，但不改變本文件的 callback
  ordering。

## 8. Canonical strategy output

M1 的 deterministic observation 使用
`CanonicalStrategyOutput(strategy_output_version = 1)`。每筆 committed
indicator 依 callback commit order 編碼為其中一種 record：

```text
EventIndicatorRecord {
    run_event_ordinal: u64
    event_fingerprint: [u8; 32]
    instrument_state_version: u64
    output_sequence: u32
    indicator_name: bytes
    indicator_value: IndicatorValue
}

FinalizeIndicatorRecord {
    output_sequence: u32
    indicator_name: bytes
    indicator_value: IndicatorValue
}
```

primitive encoding：

- record discriminant：`EventIndicator = 1`、`FinalizeIndicator = 2`。
- unsigned integer：fixed-width big-endian。
- string：`u32 byte_length` 加 exact UTF-8 bytes。
- fingerprint：32 raw bytes。
- value discriminant：`Bool = 1`、`Signed = 2`、`Unsigned = 3`、
  `Decimal = 4`、`Text = 5`。
- `Bool` payload：`false = 0`、`true = 1`。
- signed integer：two's-complement big-endian。
- `Decimal`：沿用 market types canonical `i128 atoms`。

stream framing：

```text
magic                = ASCII "OSSO"
strategy_output_ver  = u16(1)
strategy_id
strategy_version
binary_identity.algorithm
binary_identity.digest = u32 byte_length + raw bytes
canonical_params_checksum = [32]byte
record_count         = u64
records              = ordered discriminant + record payload[]
```

checksum 是上述完整 bytes 的 BLAKE3-256，小寫 hex 呈現。任何會改變 framing、
欄位、discriminant 或順序的變更都需要新的 `strategy_output_version`。

order intent、feedback 與 accounting 不混入此 stream；它們由 execution
simulation 的 versioned trace 擁有。finalize summary 若要進入 canonical output，
必須使用 `FinalizeIndicatorRecord`，並在所有 event records 之後依 emission
order 出現。

## 9. Error 與 panic

strategy error 必須包含穩定 category 與可診斷 message，但只有 category 參與
machine-readable acceptance：

| Category | 發生階段 |
| --- | --- |
| `InvalidParameters` | parameter validation |
| `InvalidDeclaration` | universe／session declaration |
| `InitializationFailed` | initialize |
| `CallbackFailed` | on_event／on_feedback |
| `FinalizeFailed` | finalize |
| `StrategyPanic` | strategy boundary 捕捉到 unwind |
| `CapabilityUnavailable` | 使用目前 run 未啟用的能力 |

callback error 發生時：

- run lifecycle 為 `Failed`。
- `failure_stage = AfterReplayCoreCommit`。
- failure occurrence、event fingerprint 與 state version 必須記錄。
- 不再呼叫後續 event callback 或 `finalize`。

Rust adapter 在支援 unwind 的 build 必須於 strategy boundary 捕捉 panic 並轉為
`StrategyPanic`。若 build 使用 `panic = "abort"`，不得宣稱具備 in-process panic
recovery；需要 process isolation 或拒絕該 execution profile。

## 10. Determinism 與 capability policy

第一版是 compile-time linked Rust strategy，不載入 runtime plugin。public
strategy context 不提供 clock、RNG、filesystem、network、environment 或 thread
pool capability。

Rust type system 無法阻止 strategy 直接呼叫所有 `std` API，因此符合
`NFR-01` 還需要：

- strategy code review／allowlist policy。
- offline acceptance job。
- 相同 input、params、binary 與 versions 的 repeated-run comparison。
- run manifest 明示 strategy binary identity。

平台已知 strategy 使用外部未記錄 capability 時，不得把該 run 標示成
reproducible。

## 11. M1 `ExampleStrategy`

M1 的 reference strategy：

```text
strategy_id      = "example.twse-post-state-observer"
strategy_version = "1"
universe         = { InstrumentId(Twse, "2330") }
session          = { Regular }
```

每個 event callback 依序輸出：

1. `state_version`：目前 `MarketState.version`。
2. `cum_volume`：若 state 有有效 cumulative volume 則輸出 exact unsigned
   value，否則不輸出。

它不產生 order intent、不讀 raw source、不使用外部 I/O。integration test 必須
證明：

- callback 次數等於 accepted event 次數。
- 第一筆 output 已反映第一個 event 更新後的 state。
- output occurrence 與 event fingerprint 一一對應。
- 相同 fixture 重跑得到完全相同 output records 與 checksum。
- strategy API 沒有取得 next event 或 mutable MarketState 的路徑。

## 12. Compatibility 與未決邊界

以下變更需要提升 `strategy_api_version`：

- callback 可見資料或生命週期改變。
- post-event／pre-event semantics 改變。
- universe 能在 runtime 擴張。
- output transaction 或 error boundary 改變。
- feedback 相對於 fill／accounting 的順序改變。

只新增不改變 API semantics 的 indicator name 不需要提升 API version，但屬於
strategy 自己的 version change。`OrderIntent`、feedback、accounting 與 result
artifact 的細節在 M2 文件中完成；在此之前不以臨時型別實作。
