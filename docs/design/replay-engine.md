# Replay Engine 設計

## 1. 文件目的

本文件定義 `osmium-lab` 如何從 frozen execution plan 開啟必要 event streams、依
`OrderingRule` 合併事件、推進 replay clock、協調 `MarketState`／
`TradingContext`／strategy／simulation，並產生可重現 checksum 與執行摘要。

依據：

- [產品需求](../product-requirements.md)
- [Replay requirements](../requirements/replay.md)
- [Strategy requirements](../requirements/strategy.md)
- [Simulation requirements](../requirements/simulation.md)
- [Architecture data flow](../architecture/data-flow.md)
- [ADR-0001：match-time ordering](../architecture/decisions/0001-match-time-ordering.md)
- [ADR-0003：session windows](../architecture/decisions/0003-session-windows-and-strategy-activation.md)
- [ADR-0004：TradingContext](../architecture/decisions/0004-trading-context-and-eligibility.md)
- [ADR-0005：TWSE intermediate/final ordering](../architecture/decisions/0005-twse-intermediate-final-ordering.md)
- [Market types](market-types.md)
- [MarketState](market-state.md)

本版固定：

| Contract | Version |
| --- | ---: |
| `replay_engine` | 1 |
| `replay_plan` | 1 |
| `canonical_replay_event_stream` | 1 |
| `ordering_rule` | 2 |

本文件固定 logical behavior，不固定 Rust crate、async runtime、cache file layout、
CLI syntax 或 strategy/simulation 的完整 public API。

## 2. 責任與邊界

### 2.1 Replay Engine 負責

- 驗證 frozen replay plan 與所有 version dependencies。
- 只開啟 plan 指定的 instrument／trading date／session streams。
- 驗證 stream identity、lineage、completeness、event schema 與 ordering。
- 以 bounded-memory k-way merge 選出下一個 `DomainEvent`。
- 只以 `match_time` 推進 replay clock。
- 將 event 指派至 materialized session segment 與 phase。
- 協調 atomic MarketState transition 與 `TradingContext` evaluation。
- 依固定順序呼叫 strategy 與 simulation boundaries。
- 記錄 event count、warning、processed prefix、event checksum 與 final-state
  checksum。
- 對任何無法維持 atomicity、ordering 或 no-lookahead 的錯誤停止執行。

### 2.2 Replay Engine 不負責

- 呼叫 Teralion、處理 credential／cursor 或下載資料。
- 解碼 Teralion wire JSON 或猜測 market-specific raw fields。
- 在 runtime 自行選擇 `STOCK_SNAPSHOT`、`STOCK_REALTIME` 或其他替代 feed。
- 修補 corrupt／stale cache，或在 cache 失敗後靜默 fallback 到 raw source。
- 重建 order book、queue position、hidden liquidity 或 exchange matching。
- 定義 strategy parameter／intent schema。
- 決定 fill price、slippage、fee、tax、position 或 P&L。
- 因 clock 穿越 open／close 而合成 event、MarketState、`TradingContext` 或 callback。

data-sync 負責提供可驗證的 stream lineage；normalizer 負責產生合法
`DomainEvent`；strategy 與 simulation 各自擁有自己的 state。Replay Engine 只協調
這些邊界，不取得其 mutation authority。

## 3. Engine lifecycle

```text
Created
-> PlanValidated
-> Initialized
-> StreamsOpened
-> Running
-> Finalizing
-> Completed {
       completion_quality: Full | Degraded
   }

Created／PlanValidated／Initialized／StreamsOpened／Running／Finalizing
-> Failed

Running
-> Cancelled
```

規則：

1. `Completed` 只在所有 selected streams 到達合法 EOF、所有 event pipelines
   完成、finalization 成功且 checksums finalize 後成立。
2. `Strict` run 成功時是 `Full`；有任何 planned omission 的
   `ExplicitDegraded` run 成功時只能是 `Degraded`。
3. `Failed`／`Cancelled` 不得發布成 successful result。
4. 第 1 版不支援持久化 mid-run resume。failed／cancelled run 必須由相同 frozen
   plan 從頭重跑。
5. lifecycle transition 只允許向前；不能把 failed run 改標成 completed。
6. wall-clock start/end time 是 operations metadata，不參與 replay time 或 domain
   checksum。

### 3.1 Initialization boundary

plan validation 完成後、讀取第一個 head 前：

1. 依 canonical universe order 為每個 instrument/date 建立 state version `0` 的
   initial `MarketState`。
2. 建立 `ReplayClock::Unstarted` 與 event ordinal `0`。
3. 初始化 canonical replay event stream hasher header、deterministic counters 與
   warning collector。
4. 只用 frozen configuration 與 static reference data 初始化 strategy。
5. 若使用 simulation，先驗證 multiplier、currency、initial cash 與 model
   prerequisites，再初始化 simulation/accounting state。
6. 開啟 selected bindings，建立 bounded stream heads。

strategy/simulation initialization 不得讀第一個 event、first price、final stats 或
stream remaining count。任何 prerequisite failure 發生在第一個 callback 前，且
MarketState 仍維持 initial state。

## 4. Frozen ReplayPlan

### 4.1 Logical model

```text
ReplayPlan {
    plan_identity: ReplayPlanIdentity
    replay_plan_version: u16
    trading_dates: NonEmpty<TradingDate>
    universe: NonEmpty<CanonicalSet<InstrumentId>>
    stream_bindings: NonEmpty<ReplayStreamBinding>
    session_plans: CanonicalMap<InstrumentDate, SessionPlan>
    stream_composition_policies: CanonicalMap<InstrumentDate, StreamCompositionPolicy>
    data_policy: ReplayDataPolicy
    version_set: ReplayVersionSet
    market_state_profiles: CanonicalMap<InstrumentDate, MarketStateProfileRef>
    market_rule_refs: CanonicalMap<InstrumentDate, MarketRuleRef>
    strategy_binding: StrategyBindingRef
    simulation_binding: Optional<SimulationBindingRef>
}
```

這是 execution plan 中 replayer 所需的 frozen subset。`ReplayPlanIdentity` 必須由
上游 planner 的 canonical effective plan 產生；具體 plan encoding 由 operations／
planning design 定義。

`CanonicalSet`／`CanonicalMap` 表示 serialization、diagnostics 與 validation 使用
domain canonical order，不得依 hash-map insertion order。

### 4.2 ReplayVersionSet

至少固定：

```text
ReplayVersionSet {
    replay_engine_version
    replay_plan_version
    market_types_version
    event_schema_version
    canonical_event_version
    ordering_rule_version
    canonical_replay_event_stream_version
    market_state_version
    state_reducer_version
    canonical_market_state_version
    canonical_final_state_set_version
    session_window_policy_version
    strategy_session_policy_version
    segment_ownership_policy_versions
    calendar_versions
    normalizer_mapping_versions
    market_state_profile_versions
    trading_eligibility_policy_version
    market_rule_versions
}
```

strategy、fill model、accounting 及 result versions 由相依 binding 補入完整
execution plan。M1 沒有 simulation 時必須明確記為 `NotUsed`，不能與缺漏版本混淆。

### 4.3 Stream binding

```text
ReplayStreamBinding {
    binding_id: StableBindingId
    stream_descriptor_id: StableStreamDescriptorId
    instrument: InstrumentId
    trading_date: TradingDate
    selected_segment_ids: NonEmpty<CanonicalSet<SessionSegmentId>>
    selected_source_formats: NonEmpty<CanonicalSet<SourceFormatId>>
    coverage_claim: ReplayCoverage
    lineage_identity: StreamLineageIdentity
}
```

`binding_id` 是 plan 內穩定 identity，只用於 diagnostics 與 ordering-equivalent
duplicate 的 internal cursor scheduling。它不是 `OrderingKey`，也不代表 exchange
sequence。

### 4.4 Stream composition policy

同一 instrument/date 可能在本地同時存在多種 format 或同一 source 的不同 cache。
plan 必須明確選擇：

```text
StreamCompositionPolicy {
    SingleAuthoritative
    DisjointCoverage
    Complementary {
        policy_name
        policy_version
    }
}
```

| Policy | Contract |
| --- | --- |
| `SingleAuthoritative` | 每個 logical coverage 只選一條 authoritative stream |
| `DisjointCoverage` | 多 streams 的 segment／time coverage 不重疊 |
| `Complementary` | market design 已證實多 formats 是互補 observations，並具版本化組合規則 |

存在 `STOCK_SNAPSHOT` 與 `STOCK_REALTIME` 不代表兩者可以自動一起 replay。若 plan
沒有版本化 composition policy，重疊 coverage 是 planning error。這可防止同一市場
observation 因替代 feed 同時被讀取而重複 callback、state version 或 cumulative
volume update。

完全 duplicate records 若已合法存在 selected authoritative stream 中仍依
ADR-0001 保留；composition validation 不是 runtime deduplication。

### 4.5 ReplayDataPolicy

```text
ReplayDataPolicy {
    Strict
    ExplicitDegraded {
        allowed_omissions: CanonicalSet<DegradedScope>
    }
}
```

M1 只允許 `Strict`。`ExplicitDegraded` 從 M2 起才可使用，且 omission 必須在 plan
freeze 前按 instrument/date/segment/format 明確列出。runtime 不得自行擴大
`allowed_omissions`。

### 4.6 Plan validation

任何 stream 開啟前必須驗證：

1. plan version 與 engine 支援版本相容。
2. universe、dates、session kinds 與 stream bindings 非空且 identity 一致。
3. 每個 universe instrument/date 都有 materialized `SessionPlan`。
4. selected segments 存在於 profile，且 replay windows 可無歧義指派 event。
5. 每個未列為 degraded omission 的 required coverage 都有且只有一個合法
   composition route。
6. source formats、mapping versions、market state profiles 與 market rules 相容。
7. `Strict` plan 沒有 missing／incomplete／corrupt scope。
8. degraded omissions 已隔離且不會破壞 ordering、atomicity 或 strategy universe
   contract。
9. strategy 宣告的 universe/session 與 frozen plan 相同；runtime 不允許新增
   instrument。
10. plan 不包含 credential、unredacted cursor 或其他 secret。

validation failure 不開 stream、不建立 MarketState、不呼叫 strategy。

## 5. Event stream contract

### 5.1 StreamDescriptor

每條可開啟 stream 必須提供：

```text
EventStreamDescriptor {
    descriptor_id: StableStreamDescriptorId
    physical_identity: SafePhysicalIdentity
    coverage: ReplayCoverage
    instruments: CanonicalSet<InstrumentId>
    trading_dates: CanonicalSet<TradingDate>
    source_formats: CanonicalSet<SourceFormatId>
    source_lineage: NonEmpty<SourceChecksumRef>
    replay_cache_lineage: Optional<ReplayCacheChecksumRef>
    completeness: CompletenessState
    expected_event_count: Optional<u64>
    first_ordering_key: Optional<OrderingKey>
    last_ordering_key: Optional<OrderingKey>
    market_types_version: u16
    event_schema_version: u16
    canonical_event_version: u16
    ordering_rule_version: u16
    mapping_versions: CanonicalSet<MappingVersionRef>
}
```

`SafePhysicalIdentity` 可以是 manifest ID 或 redacted logical path；path 本身不能取代
checksum lineage。

default replay 只接受 `complete` source lineage 與 `valid` compatible cache。若 plan
選擇直接從 complete source normalization 讀取，產生的 logical stream 必須遵守完全
相同的 event、ordering 與 descriptor contract；replayer 仍不接觸 wire type。

### 5.2 Reader boundary

logical reader 最少提供：

```text
open(binding) -> EventStreamCursor
peek(cursor) -> EventRecord | EndOfStream | StreamError
advance(cursor) -> EventRecord | EndOfStream | StreamError
```

API 名稱可以不同，但必須具有下列語意：

- `peek` 不改變 cursor，不更新 state，也不對 strategy 可見。
- current record 成功完成 replay core commit 後，才在 engine 中標記 consumed。
- 為下一筆 event 進行的 I/O failure 只能停止後續處理，不能撤銷已完成 event。
- prefetch 可以提前 decode future record，但 future payload 不得進入 callback API。
- prefetch 提前發現的 future error 必須綁定其 logical cursor position，等所有更早
  records 完成後才回報；不能因 buffer depth 改變 processed prefix。
- reader 對 selected binding 之外的 payload 不得 materialize 成 strategy-visible
  event。

`EventRecord` 可以帶 stream-local ordinal 或 cache offset 作 diagnostics，但這些值
不能進 `OrderingKey`、replay clock 或 strategy view。

### 5.3 Stream-local invariants

每條 stream 必須：

- 只產生 binding 允許的 instrument、date、format 與 replay coverage。
- 產生通過 current event schema validation 的 `DomainEvent`。
- 依完整 `OrderingKey` non-decreasing。
- 保留 ordering-equivalent duplicate events。
- 在 EOF 驗證可用的 event count、checksum 與 descriptor bounds。

replayer 發現 stream-local OrderingKey regression 時立即失敗；不得局部排序後繼續，
因為這會隱藏 corrupt cache。建立 sorted stream 是 cache builder／normalization
preparation 的責任。

### 5.4 Version compatibility

同一 run 的 selected streams 必須使用 plan 固定的：

- `market_types_version`。
- `event_schema_version`。
- `canonical_event_version`。
- `ordering_rule_version`。
- applicable normalizer mapping versions。

第 1 版不在 replayer 內做 event schema conversion。stale／incompatible cache 必須
在 plan freeze 前由 complete local source rebuild，或停止執行；不重新下載 complete
source。

### 5.5 EOF

stream 只有在 reader 回報合法 EOF 且 descriptor validations 全部通過時才算
exhausted。read／decode／checksum failure 不能當成 EOF。

zero-event stream 只有在 complete lineage 與 plan 明確允許該 coverage 為空時才合法。
它不產生 callback 或 synthetic state transition。

## 6. Session assignment 與 phase

### 6.1 Segment ownership

每個 selected event 必須依 instrument/date 的 materialized `SessionPlan` 指派到恰好
一個 selected `SessionSegment`：

```text
segment.replay_window.start <= event.match_time
event.match_time < segment.replay_window.end
```

指派結果為 zero 或 multiple segments 都是 error：

- zero 表示 stream 洩漏 outside-window event 或 plan/descriptor 不一致。
- multiple 表示 replay windows 重疊且缺少版本化 ownership rule。

若 market profile 合法允許 replay windows 重疊，planner 必須先以版本化
`SegmentOwnershipPolicy` materialize 唯一 ownership；replayer 不按 discovery order
選 segment。

### 6.2 Phase calculation

對 segment open `O`、close `C`：

| Phase | Range |
| --- | --- |
| `WarmUp` | `[O - 5m, O)` |
| `Active` | `[O, C]` |
| `CoolDown` | `(C, C + 5m)` |

exactly `C` 是 `Active`。phase 是由 current event `match_time` 純函式計算的 execution
context，不寫入 `DomainEvent` 或 canonical event bytes。

### 6.3 No synthetic boundary

沒有 event 時：

- clock 不因預定 open／close 自動推進。
- 不建立 session status event。
- 不更新 MarketState 或 state version。
- 不建立 `TradingContext`。
- 不呼叫 strategy。

segment boundary policy 只在該 instrument 的新 segment 第一個 accepted event 中，
與該 event 形成同一 MarketState transition。empty segment 不觸發 carry/reset。

## 7. Deterministic multi-stream merge

### 7.1 Head set

open 後每條 non-exhausted stream 最多提供一個 current head：

```text
heads = selected_cursors.peek_available()
next = minimum(heads, FullOrderingKey)
```

`FullOrderingKey` 使用 ADR-0001 version 2：

```text
(
    match_time,
    market_rank,
    symbol,
    source_format,
    source_phase_rank,
    event_kind_rank,
    source_sequence,
    event_fingerprint
)
```

TWSE `STOCK_REALTIME` source phase 依 ADR-0005，使 intermediate `TradeBatch` 先於
同 `match_time` final `QuoteSnapshot`。

### 7.2 Heap behavior

實作可以使用 binary heap、loser tree 或其他 bounded structure，只要：

- 比較完整 `OrderingKey`。
- 結果不依 stream open/discovery order。
- 每次只 consume 被選中的 cursor。
- buffer size、prefetch timing 與 worker count 不改變選擇。

對 `S` 條 streams，engine-owned merge state 應為 `O(S)`；每次 selection 應以
`O(log S)` 為目標。這是 design target，不取代後續 benchmark evidence。

### 7.3 Equal keys 與 duplicates

若兩個 heads：

- 完整 OrderingKey 相同。
- canonical event bytes 相同。

則為 ordering-equivalent duplicates。兩者都必須 replay，各自增加 event ordinal、
instrument state version 與 callback count。

internal cursor 可用 canonical `binding_id` 決定先 consume 哪一份 identical bytes，
只為讓 provenance trace 穩定；`binding_id` 不加入 domain `OrderingKey`，strategy
也看不到來源 cursor identity。

若完整 OrderingKey 相同但 canonical event bytes 不同，代表 fingerprint collision
或 invariant violation。第 1 版必須以 `EventFingerprintCollision` 停止，不得使用
file order 或額外未版本化欄位決定先後。

### 7.4 Global order validation

engine 保存上一個成功 core-committed `OrderingKey`。current key：

- 小於 previous：`GlobalOrderingRegression`。
- 等於 previous 且 canonical bytes 相同：合法 duplicate。
- 等於 previous 但 canonical bytes 不同：collision／invariant error。
- 大於 previous：正常。

任何 error 都不能 consume current event 或更新 clock/state。

## 8. ReplayClock 與 event occurrence

### 8.1 Logical clock

```text
ReplayClock {
    Unstarted
    At {
        match_time: MatchTime
        event_ordinal: u64
    }
}
```

第一個成功 event 的 `event_ordinal = 1`。每個成功 core commit 恰好增加一；同
`match_time` events 仍各自增加。ordinal overflow 是 fatal error。

`event_ordinal` 是 run-local trace identity，不是 source sequence、exchange
sequence 或 `OrderingKey` 欄位。

### 8.2 EventOccurrence

```text
EventOccurrence {
    run_event_ordinal: u64
    ordering_key: OrderingKey
    event_fingerprint: EventFingerprint
    instrument_state_version: u64
}
```

它用來關聯 callback、warning、intent、fill 與 diagnostics。完全 duplicate events
可有相同 OrderingKey/fingerprint，但具有不同 run ordinal 與 state version。

ADR-0004 的 origin-event fill rule 仍以 OrderingKey strictly later 判定，不以
run ordinal 將 ordering-equivalent duplicate 偽裝成較晚 market evidence。

### 8.3 Clock visibility

strategy callback 只看 current committed clock value 與 current occurrence。merge
heads、prefetched next event、next `match_time` 與 remaining count 都不在 strategy
API boundary。

## 9. Per-event coordinator transaction

### 9.1 Replay core prepare

對選出的 current head：

```text
1. validate stream record and full OrderingKey
2. validate global order
3. resolve SessionSegment and SessionPhase
4. propose ReplayClock(match_time, ordinal + 1)
5. propose complete post-event MarketState
6. validate proposed MarketState
7. evaluate TradingContext from event + phase + proposed state
```

步驟 1–7 任一失敗：

- current cursor 不 consume。
- clock／ordinal 不變。
- MarketState／state version 不變。
- event checksum accumulator 不加入 current event。
- 不發布 `TradingContext`。
- 不呼叫 strategy／simulation。

### 9.2 Replay core commit

prepare 全部成功後，一個不可部分觀察的 core commit 發布：

- proposed replay clock 與 ordinal。
- proposed MarketState 與新 state version。
- current `TradingContext`。
- `TransitionReceipt` 與 `EventOccurrence`。
- current canonical event 對 processed-prefix checksum 的一筆 append。
- current event 的 deterministic counts 與 accepted warnings。
- current record 的 logical consumed position。

這個 commit 的 externally observable order 仍是：

```text
clock advanced
-> MarketState updated
-> TradingContext available
```

實作可以用 ownership transfer、transaction object 或其他方式達成，但 strategy
不能看到 clock 已前進而 state 尚未完成的中間值。

### 9.3 Callback and decision order

core commit 後依 ADR-0004：

```text
1. invoke strategy callback(current event, state view, TradingContext)
2. collect deterministic indicator records and order intents
3. validate／create／reject new orders from this callback
4. evaluate previously pending orders whose origin OrderingKey is strictly earlier
5. allocate fills and update accounting
6. emit order／fill／accounting feedback
```

由步驟 3 新建的 order 不參與同 occurrence 的步驟 4。若相同 `match_time` 的下一個
event 具有 strictly later OrderingKey，它可以成為 subsequent-event candidate。

M1 strategy 不送單，因此只執行 callback 與 deterministic output recording；
simulation binding 是 `NotUsed`。

### 9.4 Downstream failure after core commit

strategy callback、intent validation、simulation 或 accounting 可能在 core commit
後失敗。此時：

- 已 committed market event 不 rollback。
- 不再選取下一 event。
- lifecycle status 是 `Failed`，failure stage 是 `AfterReplayCoreCommit`。
- processed event count／prefix checksum 包含 current event。
- result 不得標示 completed，也不得發布完整 strategy／ledger checksum。
- error 必須記錄 current occurrence 與完成到哪一個 pipeline stage。

這個邊界避免假裝可以 rollback arbitrary strategy-local side effects。未來若 strategy
API 提供 transactional state，也不能改變 MarketState 已接受 current observation
的事實。

### 9.5 Successful event completion

只有 downstream stages 全部成功後，engine 才開始選下一 event。由此保證：

- strategy callback sequence 與 global OrderingKey 完全一致。
- next event 不會在 current output 尚未處理完時可見。
- accounting feedback 不早於其 origin event。

## 10. Strategy 與 simulation boundary

### 10.1 Strategy view

每次 callback 至少提供：

```text
ReplayCallback {
    occurrence: EventOccurrence
    event: Immutable<DomainEvent>
    market_state: Immutable<MarketStateView>
    trading_context: Immutable<TradingContext>
    session: Immutable<SessionCallbackContext>
}
```

`SessionCallbackContext` 包含 trading date、segment id/kind、phase 與 policy
versions；不包含 future segment event counts。

callback 不能：

- 修改 event、clock、MarketState 或 `TradingContext`。
- 開啟額外 market streams。
- 查詢 next event。
- 改變 frozen universe／session／format selection。

完整 Rust trait、lifecycle hooks 與 output type 由 `strategy-api.md` 定義。

### 10.2 Simulation boundary

Replay Engine 只保證呼叫時序與輸入 identity：

- new intent 綁定 current occurrence。
- previously pending order 才能檢查 current event。
- target instrument 必須等於 current event instrument。
- `TradingContext`、phase 與 current observable evidence 一起傳入。
- fill/accounting result 必須可追溯到 current occurrence。

fill eligibility、price、quantity、order lifecycle 與 accounting atomicity 由
`execution-sim.md` 定義。

### 10.3 Lifecycle hooks

strategy API 未來可以定義 start／segment-end／finalize hooks，但：

- hook 不是 `DomainEvent`。
- 不得增加 MarketState version 或 event ordinal。
- 不得把預定 boundary 當成 market observation。
- 若 hook 產生 output，其 deterministic order 與允許能力必須由 strategy contract
  版本化。

第 1 版 replay core 不為 empty segment 合成 callback。

## 11. Canonical replay event stream

### 11.1 Purpose

event stream checksum 固定「這次 run 實際成功 core-commit 的 ordered canonical
events」。它不直接 hash raw JSON、cache bytes、Rust memory layout 或無 framing 的
event concatenation。

### 11.2 Streaming frame

`CanonicalReplayEventStream` version 1 使用 streaming-friendly layout：

```text
magic                                  [4]byte = "OSRS"
canonical_replay_event_stream_version  u16 = 1
event_schema_version                   u16
canonical_event_version                u16
ordering_rule_version                  u16

records                                repeated {
    record_tag                         u8 = 1
    canonical_event_length             u32
    canonical_event                    [canonical_event_length]byte
}

end_tag                                u8 = 0
event_count                            u64
```

primitive integer 使用 big-endian。`canonical_event` 完整重用
`market-types.md` 的 `CanonicalEvent` bytes。length 超過 `u32::MAX` 的 event 是
validation error。

end tag 讓 hasher 可一邊 replay 一邊 append records，最後才寫入 count；不需把全部
events 載入記憶體或預先知道數量。

### 11.3 Checksum

```text
ReplayEventStreamChecksum =
    BLAKE3-256(CanonicalReplayEventStream)
```

完全 duplicate event 各自有一個 record frame。warning、stream ID、cache offset、
session phase、`TradingContext`、strategy output、wall clock 與 host metadata 不進
event stream bytes。

zero-event stream 的 canonical bytes 是 header + `end_tag=0` + `event_count=0`。

### 11.4 Processed prefix

hasher 只在 replay core commit append current event。

- completed run：checksum 覆蓋完整 selected ordered event sequence。
- failed/cancelled run：以同一 end framing finalize
  `ProcessedEventPrefixChecksum`，並明確標記它不是完整 event stream checksum。
- downstream failure after core commit：prefix 包含 current event。
- pre-commit failure：prefix 不包含 current event。

若 cache descriptor 另有 cache-content checksum，它是 data lineage，不取代 run 所選
events 的 canonical checksum。

### 11.5 Final-state checksum

all streams exhausted 後，使用 `market-state.md` 的：

```text
FinalStateChecksum =
    BLAKE3-256(CanonicalFinalStateSet)
```

state 依 canonical `InstrumentId` order framing。`TradingContext`、strategy 與
simulation state 不進 final-state checksum。

final-state checksum 只有在 replay core 達到合法 EOF 時才能標示 `complete`。
failed/cancelled run 可以輸出 diagnostic state-prefix checksum，但名稱及 status
必須明確區分。

## 12. Warning、error 與 degraded execution

### 12.1 Warning records

可繼續的 warning 必須是 schema 已知且 raw 可無損保存的情況：

```text
ReplayWarning {
    code: WarningCode
    scope: WarningScope
    occurrence: Optional<EventOccurrence>
    raw_value: Optional<UnknownValue>
    safe_context
}
```

event warnings 依 `(run_event_ordinal, WarningCode enum order, local_index)` 記錄；
plan/stream warnings 依 canonical scope identity、code 排序。human-readable message
不參與 deterministic identity。

warning 不進 event/state checksum，但 count、codes、safe raw representation 與
scope 必須進 run summary。

### 12.2 Fatal categories

至少區分：

| Category | Example |
| --- | --- |
| `InvalidReplayPlan` | universe、segment、composition 或 required binding 不完整 |
| `DataUnavailable` | source missing／incomplete／corrupt |
| `IncompatibleVersion` | event／canonical／ordering／mapping／profile 不相容 |
| `StreamIntegrityError` | checksum、decode、unexpected EOF |
| `StreamIdentityMismatch` | event instrument/date/format 不屬於 binding |
| `StreamOrderingRegression` | 單 stream full key 倒退 |
| `GlobalOrderingRegression` | current key 小於 committed key |
| `EventFingerprintCollision` | key 相同但 canonical bytes 不同 |
| `SessionAssignmentError` | event 無 segment 或同時屬於多 segments |
| `StateTransitionError` | reducer validation／atomic transition 失敗 |
| `TradingContextError` | policy／rule 無法安全判定 |
| `StrategyError` | callback／output validation 失敗 |
| `SimulationError` | order／fill／accounting stage 失敗 |
| `Cancelled` | user cancellation at safe point |

error context 盡可能包含 safe plan、binding、market、symbol、trading date、format、
`match_time`、occurrence 與建議處理方式。不得包含 credential、完整 cursor 或 secret。

### 12.3 Strict behavior

`Strict` 是 default：

- 任一 fatal category 停止 run。
- 不跳過 current event。
- 不切換 source/cache。
- 不發布 completed result。
- 保留可安全檢查的 prefix diagnostics。

M1 所有 `REPLAY-06` errors 都使用此行為。

### 12.4 Explicit degraded behavior

degraded mode 只能在 plan freeze 前選擇，且只能省略能完整隔離的 scope。例如一個
明確不完整 instrument/date binding 可以整體不進 merge；plan 與 strategy 必須能看見
該 omission。

runtime 不允許將下列錯誤降級成 skip-current-event：

- ordering regression。
- checksum／decode failure。
- event identity mismatch。
- invalid payload。
- state/context transaction failure。
- 無法唯一指派 segment。

因為跳過它們會破壞 event atomicity、state continuity 或 no-lookahead。

known unsupported format 可以由 normalizer 在建立 stream 前產生 deterministic
`KnownSkipped` summary；replayer 不把 raw record 包成 placeholder event。

### 12.5 Cancellation

cancellation 只在 safe point 生效：

- event pipeline 之間。
- 所有 core/downstream stages 已完成後。

若 request 在 core prepare／commit 或 callback 中到達，engine 先完成或失敗該不可
中斷 stage，再轉成 `Cancelled`；不得留下半個 state transition。cancelled result
輸出 processed prefix count/checksum，不輸出 complete final result。

## 13. Finalization 與 RunReplaySummary

### 13.1 Finalization sequence

```text
all selected streams report validated EOF
-> verify expected counts/bounds/checksums
-> apply pending-order end policy, if simulation is used
-> finalize strategy output, if strategy contract defines it
-> finalize event stream checksum
-> encode CanonicalFinalStateSet
-> finalize final-state checksum
-> assemble replay summary and provenance
-> hand artifacts to atomic result publisher
```

任一步驟失敗只產生 failed/partial diagnostics。Replay Engine 不自行決定 filesystem
publish protocol；operations/result design 必須確保 incomplete artifacts 不冒充
successful result。

### 13.2 RunReplaySummary

至少包含：

```text
RunReplaySummary {
    run_status
    completion_quality
    replay_plan_identity
    version_set
    selected_bindings_and_lineage
    selected_universe_and_sessions
    data_policy_and_omissions
    processed_event_count
    per_instrument_event_count
    per_event_kind_count
    first_match_time: Optional<MatchTime>
    last_match_time: Optional<MatchTime>
    final_event_ordinal
    warning_count_and_codes
    known_skipped_count_and_scopes
    replay_event_stream_checksum: Optional<Checksum>
    final_state_checksum: Optional<Checksum>
    processed_event_prefix_checksum: Optional<Checksum>
    diagnostic_state_prefix_checksum: Optional<Checksum>
    failure_stage_and_occurrence: Optional<FailureContext>
    operational_timing
}
```

completed run 必須有完整 event／final-state checksums。failed/cancelled run 使用
明確的 prefix field，不得將 prefix 填進 complete checksum field。

`operational_timing` 可含 elapsed wall time 與 throughput，但不進 deterministic
domain checksums。

### 13.3 Provenance

summary／manifest 至少保留：

- source partition checksums。
- replay cache lineage/checksum；若使用。
- mapping、event schema、canonical event、ordering rule versions。
- session/calendar、state reducer、market rule、eligibility versions。
- strategy/simulation bindings 及其 effective settings。
- degraded/skipped scopes。

只記錄 path 不足以建立 provenance。secret-bearing request、API key 與 full cursor
不得保存。

## 14. Determinism、concurrency 與 resource contract

### 14.1 Semantic serialization

第 1 版 strategy-visible event pipelines 全域 serialized：

- 一次只 publish 一個 occurrence。
- 一次只呼叫一個 current-event callback。
- 下一 occurrence 等目前 downstream stages 結束後才開始。

這是 semantic contract，不代表 I/O、decode、checksum preparation 或 immutable
event validation 不能並行。

### 14.2 Allowed parallelism

可以：

- bounded asynchronous prefetch。
- 不改變 output 的 parallel cache read/decompression。
- 對 future immutable records 預先 decode/validate。
- 在 finalization 平行計算彼此獨立且有固定 join order 的 diagnostics。

不能：

- 依 worker completion order publish event。
- 讓 strategy 看 future head。
- 平行執行會彼此觀察順序的 strategy callbacks。
- 讓 concurrent state updates 繞過 global OrderingKey。
- 使用 unordered aggregation 直接產生 warning、count 或 checksum bytes。
- 讓 future prefetch error 因 buffer depth 提前截斷較早的合法 events。

### 14.3 Memory and I/O

engine-owned working set 應近似：

```text
O(selected streams + universe states + bounded prefetch + strategy/simulation state)
```

不得與全市場或完整期間 event count 線性成長。實作可以為 diagnostics 保存 bounded
recent context，但無界 event history 不屬於 Replay Engine。

physical cache 若混合多商品，reader/index 必須能證明只讀 selected ranges；僅在
decode 後丟棄 universe 外 records 不符合 `REPLAY-05`。

### 14.4 Offline boundary

`replay`／`backtest`：

- 不建立網路 client。
- 不讀 Teralion API key。
- 不因資料缺少自動 sync。
- 只使用 plan 已驗證的 local lineage。

需要下載／重建的 action 必須在 planning/sync/cache-build 階段完成，再 freeze 新
plan。

## 15. Verification contract

### 15.1 Plan and stream tests

至少覆蓋：

1. universe 外 stream 從未被 open。
2. missing／duplicate required coverage 在 open 前失敗。
3. `STOCK_SNAPSHOT` 與 `STOCK_REALTIME` 重疊且無 composition policy 時失敗。
4. incompatible event／canonical／ordering／mapping versions 失敗。
5. stale cache 可在 plan 前 rebuild，但 replayer runtime 不 fallback。
6. event identity、date、format 洩漏被拒絕。
7. unexpected EOF／checksum mismatch 不被當成合法 EOF。
8. legitimate complete zero-event stream 不合成 callback。

### 15.2 Ordering tests

至少覆蓋：

- shuffled stream discovery order 產生相同 global sequence/checksum。
- multi-instrument interleaved `match_time`。
- same-time market／symbol／format／phase／kind／sequence／fingerprint tie-break。
- TWSE intermediate `TradeBatch` 先於同時間 final `QuoteSnapshot`。
- exact duplicates 全部保留。
- equal fingerprint/key but unequal bytes fail。
- stream-local/global ordering regression fail。
- buffer size、prefetch timing、worker count 不改變結果。

### 15.3 Clock、session and no-lookahead tests

至少覆蓋：

- clock 從 `Unstarted` 到第一 event，且永不倒退。
- same-time events ordinal/state version 各自增加。
- `WarmUp`、`Active`、`CoolDown` boundary，exact close 為 `Active`。
- outside-window 與 ambiguous segment assignment fail。
- empty/open/close boundary 不合成 event/context/callback。
- strategy view 無 next-event API，prefetch 不造成 leakage。
- new order 不能使用 origin event；strictly later same-time event 可以依 policy
  評估。

### 15.4 Atomicity and failure tests

至少覆蓋：

- reducer/context failure 前 clock、state、cursor、checksum 都不變。
- core commit 同時發布 clock、state version、context、occurrence。
- callback failure 產生 `failure_stage=AfterReplayCoreCommit`，且 prefix 包含
  current event。
- next event 不在 current downstream completion 前 publish。
- cancellation 只在 safe point 生效。
- failed/cancelled result 不具有 complete checksum fields。

### 15.5 Checksum golden tests

至少提供：

- zero-event canonical replay stream。
- M1 single-stream `QuoteSnapshot` sequence。
- duplicate event sequence。
- same-time TWSE intermediate/final sequence。
- multi-stream interleaved sequence。
- failed-before-commit 與 failed-after-commit prefix。

同一 golden vectors 在支援的 OS／CPU／locale／timezone、不同 stream discovery
order 與 worker count 必須得到相同 BLAKE3-256。

### 15.6 M1 acceptance mapping

| Acceptance | Replay Engine evidence |
| --- | --- |
| `M1-AC-03` | `match_time` ordering、clock monotonicity、event stream checksum |
| `M1-AC-04` | same-time deterministic tie-break |
| `M1-AC-05` | complete snapshot state replacement integration |
| `M1-AC-06` | post-event state callback 與 no-lookahead |
| `M1-AC-07` | invalid `match_time` fail-fast |
| `M1-AC-08` | unknown format fail-fast |
| `M1-AC-09` | deterministic warning aggregation |
| `M1-AC-10` | repeat run checksums and strategy output |

## 16. Increment delivery

### M1

- single TWSE 2330 `STOCK_SNAPSHOT` fixture stream。
- `Strict` plan、one instrument/date/regular segment。
- OrderingRule version 2；source phase rank 對此 format 為 0。
- replay clock、MarketState、ExampleStrategy callback。
- canonical replay event stream 與 final-state checksums。
- 不包含 simulation、degraded mode 或 multi-stream benchmark。

### M2

- complete local source/cache binding 與 offline replay。
- `TradingContext` + strategy intent + simulation coordinator stages。
- explicit degraded plan 與 result provenance。
- TWSE `STOCK_REALTIME` mapping version 2 intermediate/final path。

### M3

- multi-instrument k-way merge。
- TAIFEX `regular`／`after_hours` segments 與跨日 trading date。
- multi-segment state boundary policies。
- bounded-memory benchmark 與 multi-stream property tests。

### M4

- TPEx、warrant、option mappings 與 market-specific composition/profile versions。
- 維持同一 plan、stream、merge、clock 與 transaction contract。

## 17. 相依文件邊界

本文件刻意不固定下列尚未完成的設計：

- `data-sync.md`：source/cache manifest schema、atomic publish、cache file/index layout。
- `strategy-api.md`：Rust trait、strategy-local state、lifecycle hooks、output schema。
- `execution-sim.md`：order lifecycle、fill allocation、accounting transaction。
- operations/CLI design：configuration、plan canonical encoding、result directory layout。

這些文件可以選擇最簡單實作，但不得改變：

- frozen plan 後不新增 stream。
- strategy-visible event pipeline 的 global deterministic order。
- replay clock 只使用 `match_time`。
- state/context pre-callback atomicity。
- origin event 不可 fill。
- event/final-state checksum framing。
- failed/degraded result 不冒充 completed。

## 18. Traceability

- `DATA-03`／`DATA-04`：stream completeness、lineage、cache compatibility 與 offline
  rebuild boundary。
- `REPLAY-01`：只消費 validated `DomainEvent`，不接觸 wire format。
- `REPLAY-02`：OrderingRule version 2、duplicates 與 canonical event stream。
- `REPLAY-03`：每 instrument state ownership 與 atomic reducer。
- `REPLAY-04`：clock、state、context、callback 與 no-lookahead。
- `REPLAY-05`：explicit universe、selected streams、bounded k-way merge。
- `REPLAY-06`：strict/degraded policy、warning、failure 與 cancellation。
- `STRAT-01`：immutable callback boundary。
- `SIM-01`：origin-event 與 subsequent-event coordinator order。
- `OPS-02`：run summary、version/checksum/data provenance。
- `NFR-01`：同 plan/input/version 產生相同 sequence 與 checksums。
- `NFR-02`：bounded streams/buffers，不載入全部 events。
- `NFR-03`：offline、secret-free、version incompatibility boundary。
