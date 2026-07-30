# MarketState reducer 與 canonical state 設計

## 1. 文件目的

本文件定義 domain event 如何更新 `MarketState`，以及 strategy、simulation 與
checksum 如何讀取同一份可重現狀態。它承接：

- [產品需求](../product-requirements.md)
- [Replay requirements](../requirements/replay.md)
- [ADR-0001：match_time ordering](../architecture/decisions/0001-match-time-ordering.md)
- [ADR-0002：snapshot market state](../architecture/decisions/0002-snapshot-market-state.md)
- [ADR-0003：session windows](../architecture/decisions/0003-session-windows-and-strategy-activation.md)
- [ADR-0004：TradingContext](../architecture/decisions/0004-trading-context-and-eligibility.md)
- [Market types](market-types.md)

本版設計版本固定為：

| Contract | Version |
| --- | ---: |
| `market_state` | 1 |
| `state_reducer` | 1 |
| `canonical_market_state` | 1 |
| `canonical_final_state_set` | 1 |

版本是 run identity 與 manifest 的一部分。domain type 名稱不附加 `V1` 後綴。

本文件不決定 Rust crate、module layout、storage engine 或 public package API；這些應在
實作時以最小範圍承接本文契約。

## 2. 設計邊界

### 2.1 MarketState 是 source-derived facts

`MarketState` 只保存 accepted domain events 能直接支持的 facts：

- 最新完整最佳五檔 snapshot。
- 最近一次可觀察的單筆成交或成交 batch。
- 最近一次累計成交量 observation。
- 最近一次 market annotations observation，包括完整 raw flags。
- 最後套用 event 的 identity、`match_time` 與 state version。

`MarketState` 不保存或推導：

- order-level add、modify、cancel。
- queue position、hidden liquidity 或真實可成交量。
- `close`、`stats` 等不供 replay/backtest 使用的資料。
- `TradingContext`、strategy state、orders、fills 或 portfolio。
- 僅因時鐘越過 open／close 所產生的合成狀態。

完整五檔 snapshot 是當下 view，新的 snapshot 必須完整取代舊 snapshot。不得由相鄰
snapshots 的差異反推 order flow。

### 2.2 TradingContext 是 derived decision

是否允許 new order intent、目前是 continuous matching、call auction、indicative
matching 或 unknown，由 `TradingEligibilityPolicy` 在每個 accepted event 套用後計算
`TradingContext`。它不是 `MarketState` 的 source field，也不進 final-state checksum。

因此 `MarketState` 不提供 `is_tradeable: bool`。strategy 與 simulation 必須同時讀取：

- 更新後的 immutable `MarketStateView`。
- 與該 event、state version 綁定的 immutable `TradingContext`。

## 3. State scope 與生命週期

### 3.1 State identity

一個 replay run 對 execution plan universe 中的每個 `InstrumentId` 維護一份獨立
state。其 scope 是：

```text
(run_id, trading_date, instrument)
```

`InstrumentId` 只表示 market 與 symbol；`trading_date`、session segment 與 replay
phase 不塞進 instrument identity。

同一 instrument 的 `state_version` 從 `0` 開始。版本 `0` 表示尚未接受任何 event，
所有 observable fields 都是 initial unavailable。

### 3.2 Segment boundary policy

同一 trading date 可能包含多個 session segments，例如 TAIFEX `regular` 與
`after_hours`。跨 segment 的 state 行為必須由 market profile 明確指定：

```text
SegmentBoundaryPolicy {
    Carry
    ResetObservableFields
}
```

| Policy | 行為 |
| --- | --- |
| `Carry` | 保留 book、recent trade、cumulative volume 與 annotations，由下一個 event 繼續更新 |
| `ResetObservableFields` | 在下一個 segment 的第一個 event 前，將 observable fields 重設為 initial unavailable |

規則如下：

1. execution plan 選取多個 segments 時，缺少 boundary policy 是 planning error。
2. boundary action 與新 segment 的第一個 event 必須形成同一個 atomic transition。
3. 只因 clock 跨過 segment boundary，不產生 event、callback 或 state version。
4. `CoolDown` 是 execution phase，不是 market event 或獨立 state；不得在 close
   boundary 合成 transition。
5. policy 及其版本必須記錄於 run manifest。修改 policy 必須改變 run identity。

這個 contract 不假設所有 market 都應 carry 或 reset；market-specific design 必須選擇。

## 4. Logical state model

### 4.1 MarketState

```text
MarketState {
    instrument: InstrumentId
    trading_date: TradingDate
    current_segment_id: Optional<SessionSegmentId>
    book: StateField<CompleteBookSnapshot>
    recent_trade: StateField<TradeObservation>
    cumulative_volume: StateField<Volume>
    last_annotations: StateField<MarketAnnotations>
    last_event: Optional<AppliedEventRef>
    state_version: u64
}
```

`last_match_time` 由 `last_event.match_time` 取得；version `0` 時不存在。

`current_segment_id` 是 reducer boundary context，不代表 exchange 狀態。它只在接受該
segment 的第一個 event 時更新。

### 4.2 StateField

State 必須保留 unavailable、known 與 unknown 的差異：

```text
StateField<T> {
    Unavailable {
        reason: UnavailableReason
    }
    Known {
        value: T
        observed_at: AppliedEventRef
    }
    Unknown {
        raw: UnknownValue
        observed_at: AppliedEventRef
    }
}

UnavailableReason {
    Initial
    Cleared {
        cleared_at: AppliedEventRef
    }
}
```

`UnknownValue` 直接重用 `market-types.md` 定義的 bounded、可無損 canonical encode
scalar，不是 debug-formatted string。TWSE reserved status／limit bits 仍屬於 known
`TwseQuoteAnnotations`：完整 raw bytes 會保存在 annotation value 中，並由 typed view
產生 warning；它們不會被改包成 `UnknownValue`。

`Observation<T>` 套用到 `StateField<T>` 的規則為：

| Observation | State transition |
| --- | --- |
| `NoObservation` | 保留原 field，包括原本的 origin |
| `Set(value)` | `Known { value, observed_at: current_event }` |
| `Clear` | `Unavailable { Cleared { current_event } }` |
| `Unknown(raw)` | `Unknown { raw, observed_at: current_event }` 並產生 structured warning |

`Unknown` 不得悄悄沿用上一個 known value；否則 strategy 會把 stale fact 誤認為 current
fact。consumer 若需要 fallback，必須在自己的版本化 policy 中明確處理。

### 4.3 AppliedEventRef

```text
AppliedEventRef {
    match_time: MatchTime
    source_format: SourceFormatId
    event_kind: EventKind
    source_sequence: Optional<u64>
    event_fingerprint: EventFingerprint
}
```

`event_fingerprint` 是 `market-types.md` 定義的 canonical event BLAKE3-256
fingerprint。`AppliedEventRef` 讓 field origin、state transition 與 callback trace 可
指向同一 event，而不複製整個 event payload。

### 4.4 TradeObservation

```text
TradeObservation {
    Single(TradePrint)
    Batch {
        trades: NonEmpty<TradePrint>
        trade_order: TradeOrder
    }
}
```

`recent_trade` 保存最近一次 observation，不是無界 trade history。

`last_trade()` 的 derived view 規則為：

| Value | Result |
| --- | --- |
| `Single(trade)` | 該 trade |
| `Batch` + `SourceOrdered` | source order 的最後一筆 |
| `Batch` + `Unspecified` | unavailable，reason 為 ambiguous batch order |
| field unavailable／unknown | 同樣回傳 unavailable／unknown，不補舊值 |

book 改變不能合成 trade；`TradeBatch` 也不能合成 book。

### 4.5 Annotation semantics

`last_annotations` 表示「最近一個 event 提供的 annotations observation」，不是持續
有效的 exchange condition。

- `MarketAnnotations::None` 是明確的 known value，不等於 `NoObservation`。
- raw status／limit flags 必須完整保存。
- opening／closing marker 等 event-scoped bit 不得被當成 sticky flag。
- unknown raw value 必須保留並產生 warning，不得猜測 session、halt 或 matching state。
- `TradingContext` evaluator 以 current event annotations 為主要輸入；若某 market
  condition 需要 carry，必須由版本化 market rule 明確定義。

## 5. Reducer input 與 validation

### 5.1 Reducer context

```text
ReducerContext {
    trading_date: TradingDate
    session_segment_id: SessionSegmentId
    segment_boundary_policy: SegmentBoundaryPolicy
    market_profile: MarketStateProfile
}
```

`MarketStateProfile` 至少定義：

- 該 market／source format 接受的 event kinds。
- cumulative volume unit 與 validation policy。
- segment boundary policy version。
- annotation compatibility。

它不能包含 strategy-specific logic。

### 5.2 Event preconditions

在提出 transition 前必須驗證：

1. event instrument 與 state instrument 相同。
2. event trading date 與 state scope 相同。
3. event kind 與 source format 被 profile 支援。
4. event 已通過 `market-types.md` 的 payload invariants。
5. event ordering key 不小於該 instrument 的 `last_event` ordering key。
6. `match_time` 不小於目前 `last_match_time`。
7. segment id 位於 execution plan，且 boundary action 符合 profile。
8. state version 尚未達 `u64::MAX`。

完全相同的 duplicate accepted events 可以再次套用；每次都形成獨立 transition 並使
version 增加一。deduplication 若需要，必須發生在 reducer 前且留下可稽核紀錄。

### 5.3 Cumulative volume

`Set(0)` 是合法 observation。generic reducer 不假設所有 markets 或 segments 的
cumulative volume 都具有相同 reset 規則。

market profile 可宣告：

```text
CumulativeVolumePolicy {
    Unconstrained
    NonDecreasingWithinSegment {
        unit: VolumeUnit
    }
}
```

使用 `NonDecreasingWithinSegment` 時：

- 同 segment 內 known value regression 必須拒絕整個 event。
- unit 不一致必須拒絕整個 event。
- `Clear` 或 `Unknown` 依 observation semantics 套用，不做數值比較。
- 跨 segment 是否 carry/reset 仍由 `SegmentBoundaryPolicy` 決定。

Reducer 絕不能用 `max(previous, current)`、clamp 或排序猜測修復來源資料。TWSE
`STOCK_REALTIME` intermediate/final pair 必須先由 mapping version 2 驗證 group，
再依 `OrderingRule` version 2 的 source phase rank 進入 reducer。

## 6. Event reduction

### 6.1 QuoteSnapshot

一次 `QuoteSnapshot` transition：

1. 將 `book` 完整替換為 current event 的 `CompleteBookSnapshot`。
2. 依 `Observation<TradePrint>` 更新 `recent_trade`：
   - `Set` 轉為 `TradeObservation::Single`。
   - 其他 variants 依第 4.2 節處理。
3. 依 observation 更新 `cumulative_volume`。
4. 將 current event 的 annotations 寫入 `last_annotations`。
5. 更新 `last_event` 與 `current_segment_id`。
6. `state_version` 恰好增加一。

book、trade、volume、annotations、last event 與 version 是同一 atomic transition，
consumer 不能看到其中一部分已更新。

### 6.2 BookSnapshot

一次 `BookSnapshot` transition：

1. 完整替換 `book`。
2. 保留 `recent_trade`。
3. 保留 `cumulative_volume`。
4. 將 current event 的 annotations 寫入 `last_annotations`。
5. 更新 `last_event`、segment 與 version。

不得把 book price/quantity change 解讀為成交。

### 6.3 TradeBatch

一次 `TradeBatch` transition：

1. 保留 `book`。
2. 將非空 trades 與 `TradeOrder` 寫為一個 `TradeObservation::Batch`。
3. 依 observation 更新 `cumulative_volume`。
4. 將 current event 的 annotations 寫入 `last_annotations`。
5. 更新 `last_event`、segment 與 version。

batch 內有多筆 trades 仍只造成一次 state version 增加。

### 6.4 No-op values 與 duplicates

event 即使寫入與現有 state 完全相同的 value，仍是 accepted observation，因此：

- field origin 更新為 current event。
- `last_event` 更新。
- state version 增加一。
- strategy 收到一次 callback。

Reducer 不以 value equality 省略 transition。

## 7. Atomic transition protocol

### 7.1 Conceptual API

```text
propose(
    current_state,
    event,
    reducer_context
) -> Result<ProposedTransition, StateTransitionError>

validate(proposed_transition) -> Result<(), StateTransitionError>

evaluate_trading_context(
    event,
    proposed_transition.view(),
    session_context,
    market_rule,
    eligibility_policy
) -> Result<TradingContext, TradingContextError>

commit(
    current_state,
    proposed_transition
) -> TransitionReceipt
```

這是行為 contract，不強制以 clone 整份 state 實作。copy-on-write、transactional
builder 或其他方式都可以，只要外部可見結果相同。

### 7.2 Observable processing order

每個 event 的 coordinator transaction 為：

```text
accepted DomainEvent
-> validate ordering and reducer inputs
-> propose replay clock position = event.match_time
-> propose complete post-event MarketState
-> validate complete proposed state
-> derive TradingContext from current event + proposed state
-> atomically publish state, context and replay clock position
-> invoke strategy callback with immutable views
-> evaluate intents and existing-order fills
```

`TradingContext` 必須讀 post-reducer proposed state，但在它計算成功前 state 不得對
strategy 或其他 consumer 可見。clock proposal 在 reducer 前符合 replay 的邏輯順序，
但 clock、state 與 context 的成功結果必須一起對外發布，避免 failure 留下半個 event。

任一步驟失敗時：

- 原 state 與 state version 不變。
- replay clock 不前進到該 event。
- 不發布 `TradingContext`。
- 不呼叫 strategy。
- 不接受 intent 或產生 fill。
- 回傳 structured error；runner 依 policy fail-fast 或停止該 run。

### 7.3 TransitionReceipt

```text
TransitionReceipt {
    instrument: InstrumentId
    previous_version: u64
    new_version: u64
    event: AppliedEventRef
    boundary_action: Optional<SegmentBoundaryAction>
    changed_fields: OrderedSet<StateFieldName>
    warning_codes: OrderedSet<WarningCode>
}
```

`changed_fields` 表示 event 寫入或 boundary reset 的 fields，不以 value equality 判斷。
集合必須依固定 enum order 輸出，不能依 hash iteration order。

warning message text 不是 deterministic identity；machine-readable `WarningCode` 才能進
trace。warnings 不進 canonical market state。

## 8. Immutable MarketStateView

Strategy 與 simulation 取得 callback-scoped immutable view：

```text
MarketStateView {
    instrument()
    trading_date()
    current_segment_id()
    state_version()
    last_event()
    last_match_time()
    book()
    best_bid()
    best_ask()
    recent_trade()
    last_trade()
    cumulative_volume()
    last_annotations()
}
```

每個 field accessor 都必須保留 `Known`、`Unavailable`、`Unknown` 與 origin，不能把：

- unavailable book 當 empty book。
- unavailable volume 當 zero。
- unknown value 換成前一個 known value。
- ambiguous batch 任選一筆當 last trade。

View 不提供 interior mutation，callback 也不能保存能在 reducer 下一次更新時改變的
borrowed object。實作可用 callback lifetime、owned snapshot 或 immutable shared
snapshot 達成。

`best_bid` 與 `best_ask` 只讀完整 snapshot 的第一個 available level。`spread` 若提供，
使用 checked decimal subtraction，允許 source data 產生 zero 或 negative value；
validation policy 是否拒絕 crossed book 是另一個版本化 market rule。本版不定義會牽涉
rounding 的 `mid_price`。

## 9. Canonical market state

### 9.1 Primitive encoding

Canonical state 沿用 `market-types.md` 的 primitive rules：

- multi-byte integer 使用 big-endian。
- enum 使用文件固定的 discriminant。
- string 使用 UTF-8 並以前置長度 framing。
- list 以前置 item count，依既定順序編碼。
- optional value 先編 presence discriminant。
- decimal、price、quantity、volume 與 domain payload 重用 CanonicalEvent encoding。
- 禁止 serializer defaults、map iteration order、locale、timezone database 或
  debug/display formatting。

### 9.2 CanonicalMarketState framing

單一 instrument state 的 byte layout 依序為：

```text
magic                                [4]byte = "OSMS"
canonical_market_state_version       u16 = 1
market_state_version                 u16 = 1
instrument                           CanonicalInstrumentId
trading_date                         i32
current_segment_id                   Optional<CanonicalString>
state_version                        u64
book                                 CanonicalStateField<CompleteBookSnapshot>
recent_trade                         CanonicalStateField<TradeObservation>
cumulative_volume                    CanonicalStateField<Volume>
last_annotations                     CanonicalStateField<MarketAnnotations>
last_event                           Optional<CanonicalAppliedEventRef>
```

`trading_date` 使用 proleptic Gregorian `days since 1970-01-01`，不使用 formatted date
string。`SessionSegmentId` 是 market profile 中具版本的 canonical UTF-8 identifier。

### 9.3 CanonicalStateField

`StateField` discriminants 固定為：

| Variant | Discriminant | Following payload |
| --- | ---: | --- |
| `Unavailable(Initial)` | `0` | 無 |
| `Unavailable(Cleared)` | `1` | `CanonicalAppliedEventRef` |
| `Known` | `2` | `CanonicalAppliedEventRef` + canonical value |
| `Unknown` | `3` | `CanonicalAppliedEventRef` + canonical `UnknownValue` |

`TradeObservation` discriminants 固定為：

| Variant | Discriminant | Following payload |
| --- | ---: | --- |
| `Single` | `1` | canonical `TradePrint` |
| `Batch` | `2` | canonical `TradeOrder` + non-empty canonical trade list |

book、trade、volume 與 annotations 的 payload encoding 重用相同版本
`CanonicalEvent` 的對應 encoding，不建立第二套 decimal 或 flag 表達。

### 9.4 CanonicalAppliedEventRef

layout 依序為：

```text
match_time              i64
source_format           CanonicalString
event_kind              u8
source_sequence         Optional<u64>
event_fingerprint       [32]byte
```

`event_kind` discriminant 與 `CanonicalEvent` 相同。`match_time` 使用
`market-types.md` 定義的 canonical epoch/unit，不受本機 timezone 影響。

### 9.5 Final-state checksum

單一 state fingerprint：

```text
StateFingerprint =
    BLAKE3-256(CanonicalMarketState(state))
```

run-level final state 即使 M1 只有一個 instrument，也使用 collection framing：

```text
magic                                [4]byte = "OSMF"
canonical_final_state_set_version    u16 = 1
state_count                          u32
states                               repeated {
    state_byte_length                u32
    canonical_market_state           [state_byte_length]byte
}

FinalStateChecksum =
    BLAKE3-256(CanonicalFinalStateSet)
```

states 必須按 `InstrumentId` 的 canonical order 排序；不得使用 execution insertion
order 或 hash-map iteration order。

Final-state checksum 包含 source-derived state、field origin、last event、segment id
與 state version，因此能偵測 observation semantics 或 transition count 的差異。它不
包含：

- `TradingContext`、eligibility result 或 phase。
- strategy、simulation、orders、fills 或 portfolio。
- warning 顯示文字或 logger state。
- replay cache offset、file path、host、thread count 或 wall-clock time。

eligibility policy 與 market rule 的版本仍須進 run manifest、decision trace 與
order/fill result identity；只是不能污染 source-derived final-state checksum。

## 10. Determinism 與 concurrency

每個 instrument 的 reducer 只能依賴：

- current state。
- current accepted event。
- execution plan 已固定的 reducer context 與版本。

它不能讀 wall clock、randomness、network、unordered shared map 或另一 instrument 的
未同步 state。

多 instrument replay 可以平行 prepare transitions，但 publish/callback 順序仍必須
符合 `OrderingKey` 的 deterministic total order。每個 instrument 有自己的 state
version；run-level event ordinal 由 replayer 另行維護，不能冒充 exchange sequence。

相同 verified source、normalizer version、event order、execution plan、market profile
與 reducer versions 必須得到相同：

- event stream checksum。
- 每個 instrument 的 state version 與 canonical bytes。
- run-level final-state checksum。

## 11. Error 與 warning taxonomy

至少需要下列 stable machine-readable categories：

| Category | Severity | Required behavior |
| --- | --- | --- |
| instrument/date mismatch | error | reject whole event |
| unsupported source/event kind | error | reject whole event |
| invalid complete book | error | reject whole event |
| ordering regression | error | reject whole event |
| cumulative volume regression under declared policy | error | reject whole event |
| volume unit mismatch | error | reject whole event |
| missing/invalid segment policy | error | reject before publish |
| state version overflow | error | reject whole event |
| unknown raw observation | warning | preserve raw and accept transition |
| ambiguous `last_trade` view | view result | return unavailable; do not mutate state |

錯誤與 warning 的 human-readable wording 可演進；category code 與影響 transition 的
規則若改變，必須提高對應 reducer／profile version。

## 12. Verification contract

### 12.1 Reducer unit tests

至少覆蓋：

1. version `0` 所有 fields 都是 initial unavailable。
2. `QuoteSnapshot` 同時原子更新 book、trade、volume 與 annotations。
3. 新 complete book 完整取代舊 book，不殘留舊 levels。
4. `BookSnapshot` 不修改 trade 與 cumulative volume。
5. `TradeBatch` 不修改 book，且不論 batch size 只增加一個 version。
6. `NoObservation` 保留 value 與 origin。
7. `Set`、`Clear`、`Unknown` 產生正確 field state。
8. unknown raw 無損保存並產生 stable warning code。
9. same-value event 與 duplicate accepted event 各自增加 version。
10. invalid event、ordering regression 與 version overflow 都不留下 partial state。
11. opening／closing marker 不會因 `last_annotations` 被誤當成 sticky matching state。
12. `TradeOrder::Unspecified` batch 不會任選 last trade。

### 12.2 Boundary 與 coordinator tests

至少覆蓋：

- `Carry` 與 `ResetObservableFields` 都只在新 segment 第一個 event 的 transition 生效。
- clock 跨 open／close／segment boundary 不合成 event、callback 或 version。
- `TradingContext` 讀到 current event 套用後的 proposed state。
- context evaluation failure 時 state、clock、callback、intent 與 fill 都不變。
- strategy callback 只能讀 immutable view，且看不到 future event。

### 12.3 Canonical encoding tests

至少提供 checked-in golden vectors：

- initial state。
- M1 TWSE `QuoteSnapshot`／`TradeBatch` 後的 state。
- 包含 `Clear` 與 `Unknown(raw)` 的 state。
- source-ordered 與 unspecified `TradeBatch` state。
- multi-instrument final-state set。

同一 vectors 必須在支援的 OS／CPU architecture／timezone 產生相同 canonical bytes 與
BLAKE3 checksum。刻意打亂 map insertion 或 input discovery order 後，排序結果仍須
相同。

## 13. Increment delivery

| Milestone | Scope |
| --- | --- |
| M1 | TWSE `QuoteSnapshot`／`TradeBatch` reducer、完整五檔 replacement、intermediate trade 保留 book、trade/volume/raw flags、immutable view、state version、canonical final-state checksum |
| M2 | `TradingContext` coordinator transaction、strategy/simulation integration、完整 trace |
| M3 | TAIFEX `BookSnapshot`／`TradeBatch`、multi-segment boundary policy、multi-instrument final-state set |
| M4 | TPEx 與其他 market profiles，維持相同 generic reducer contract |

若前一 milestone 提前實作後續 event kind，仍必須符合本文件的 atomicity、view 與
canonical encoding；不能用 milestone 名稱降低 invariant。

## 14. TWSE realtime ordering dependency

TWSE intermediate/final 順序由
[ADR-0005](../architecture/decisions/0005-twse-intermediate-final-ordering.md)固定：

```text
intermediate TradeBatch
-> final QuoteSnapshot
```

Reducer 不自行辨認 raw `intermediate_print`，也不重新 grouping；它只接受已由
`TeralionTwseQuote` mapping version 3 產生、且與 `OrderingRule` version 2 相容的
domain events。mapping version 3 保留 version 2 的 intermediate/final ordering，
只固定 quantity-unit semantics。

已驗證的 `1+1` group 會形成兩個 atomic transitions。若 source 出現多筆
intermediate、缺少 final 或 cumulative volume 關係不符，normalizer 必須拒絕整個
group，不能讓 reducer 以 clamp、deduplication 或 synthetic trades 代替 mapping
決策。
