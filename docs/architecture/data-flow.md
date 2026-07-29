# 資料與執行流程

## 1. 文件目的

本文件描述資料從使用者設定、Teralion、本地來源 partition、replay cache，到
MarketState、strategy、simulation 與 run result 的端到端流程。

元件責任見[架構總覽](overview.md)，必要行為見
[資料需求](../requirements/data.md)、
[回播需求](../requirements/replay.md)、
[策略需求](../requirements/strategy.md)、
[模擬需求](../requirements/simulation.md)及
[操作需求](../requirements/operations.md)。

本文件描述 control flow、artifact lineage 與 failure boundary，不固定檔案格式、
Rust API 或 CLI syntax。

## 2. Artifact 與狀態

### 2.1 主要 artifact

| Artifact | 來源 | 是否可重建 | 是否可在 backtest 修改 |
| --- | --- | --- | --- |
| Run configuration | 使用者 | 是 | 否；execution plan 後固定 |
| Execution plan | configuration + 本地狀態 | 是 | 否 |
| Source staging data | Teralion page responses | 可重新下載 | 否；只由 sync 管理 |
| Verified source partition | staging + validation | 不應依 cache 重建 | 否 |
| Replay cache | verified source + versions | 是 | 只可在明確 build/rebuild 階段 |
| Event stream | cache 或 source normalization | 是 | 否 |
| MarketState | ordered events | 是 | 只由 reducer 更新 |
| Strategy state | callbacks／feedback | 是 | 只由 strategy instance 更新 |
| Order／fill ledger | intent + events + models | 是 | 只由 simulation 更新 |
| Run artifacts | execution outputs | 是 | 完成後唯讀 |

### 2.2 Source partition state

```text
missing
   │ sync requested
   v
building ───────────────> incomplete
   │                         │
   │ verified                │ explicit retry／repair
   v                         v
complete <─────────────── building
   │
   │ later checksum／parse failure
   v
corrupt ── explicit reacquire／repair ──> building
```

只有 `complete` 可由 default replay／backtest 使用。explicit degraded plan 可以
略過已隔離的 incomplete／corrupt scope，但不能把其內容送進 normalizer。

### 2.3 Replay cache state

cache 不沿用 source completeness state；它至少具有：

- absent
- building
- valid
- stale／incompatible
- corrupt

stale、incompatible 或 corrupt cache 可由 complete source partition 離線重建。
cache 狀態不得改變 source partition 狀態。

## 3. Plan flow

### 3.1 Input

Planner 接收：

- operation：plan、sync、verify、replay、backtest 或整合流程
- trading date／range
- strategy definition 與參數
- explicit universe
- data／degraded policy
- replay／simulation settings
- output settings

### 3.2 Planning sequence

```text
parse configuration
-> apply explicit defaults
-> validate strategy parameters
-> ask strategy for explicit universe
-> expand universe x trading dates into required partitions
-> inspect local source manifests
-> inspect replay cache lineage
-> classify reuse／download／verify／rebuild／reject
-> validate model and metadata prerequisites
-> freeze execution plan
```

Planner 不開啟 event payload，不執行 strategy callback，也不建立成功 result。

### 3.3 Plan decision table

| Source | Cache | Default plan action |
| --- | --- | --- |
| complete | valid／compatible | reuse cache |
| complete | absent | build cache 或直接 normalize；依 execution design |
| complete | stale／incompatible | rebuild cache offline |
| complete | corrupt | reject cache and rebuild offline |
| missing | any | sync required；offline execution rejects |
| building | any | resume／restart sync or reject |
| incomplete | any | reject；explicit degraded 只能略過 scope |
| corrupt | any | reacquire／repair or explicitly skip isolated scope |

即使 cache 看似 valid，source checksum lineage 不符時也必須視為 stale／incompatible。

## 4. Sync flow

### 4.1 Online boundary

只有 sync adapter 取得 Teralion credential：

```text
execution plan
-> credential provider
-> Teralion adapter
-> coverage／ticks／daily instruments
```

credential 不寫入 request identity、staging metadata、manifest、log 或 error。

### 4.2 Cursor pagination

單一 partition 的同步流程：

```text
confirm coverage and closed trading date
-> create isolated staging area
-> request first page
-> persist page payload + safe page metadata
-> validate response and opaque next cursor
-> request next page
-> repeat until service reports no next cursor
-> fetch／associate daily instrument data
-> validate counts, formats, times and required metadata
-> calculate source checksum
-> write complete manifest
-> atomic publish
```

opaque cursor 只從前一 response 傳入下一 request，不解析或自行產生。page size、
retry 或 checkpoint strategy 不得改變 published content。

### 4.3 Failure paths

| Failure | Staging result | Published source |
| --- | --- | --- |
| Network／service failure | building 或 incomplete | 不變 |
| Cursor 不前進／不合法 | incomplete + error context | 不變 |
| Process interruption | building | 不變 |
| Disk／write failure | building／incomplete | 不變 |
| Validation failure | incomplete 或 corrupt staging | 不變 |
| Existing complete checksum differs | conflict／new revision required | 不靜默覆寫 |

retry 可以重用已驗證 checkpoint 或重頭開始；未驗證 page 不得成為 complete。

### 4.4 Atomic publish

publish 是 source repository 對外可見性的切換點：

```text
staging payload + complete manifest + verified checksum
                         │
                         v
                atomic visibility change
                         │
                         v
               complete source partition
```

實際 rename、transaction 或 pointer swap 由 data-sync design 決定。需求是 observer
只能看到先前 complete revision 或新的 complete revision，不能看到半套內容。

## 5. Verify flow

Verify 完全本地執行：

```text
read source manifest
-> verify identity and schema
-> recalculate／check source checksum
-> verify count and payload readability
-> verify cursor completion evidence
-> verify instrument metadata association
-> classify completeness
-> inspect cache lineage and checksum
-> report source and cache status separately
```

source corruption 不能用 cache rebuild 掩蓋。若 source complete、cache invalid，
verify 可以建議或依明確 plan 執行 cache rebuild。

## 6. Normalization 與 cache build flow

### 6.1 Normalizer selection

```text
source partition
-> read tick
-> select normalizer by market + format
-> validate match_time／price／quantity／shape
-> map confirmed fields
-> preserve unknown raw values + warning
-> emit one atomic domain event per source tick mapping
```

unknown format 沒有 generic fallback。default mode 停止；degraded mode 可以記錄並
略過已隔離內容，但不得猜測 event。

### 6.2 Cache lineage

每個 cache partition 建立 lineage：

```text
source partition identity
source checksum
source／mapping version
event schema version
ordering dependency version
cache format version
        │
        v
replay cache identity + checksum
```

任何參與 event 語意或 cache 解讀的 identity 不相容，cache 即失效。

### 6.3 Cache publish

```text
build in isolated staging
-> normalize all selected source ticks
-> validate event invariants
-> order／index as cache format requires
-> calculate checksum
-> write lineage manifest
-> atomic cache publish
```

cache build 失敗不修改 source。相同 source 與版本必須建立相同 canonical events
及 checksum。

## 7. Replay-only flow

Replay-only 不需要 strategy 或 simulation：

```text
frozen execution plan
-> verify selected source／cache compatibility
-> open only universe streams
-> deterministic streaming merge
-> for each event:
     select next event
     validate time monotonicity
     advance replay clock
     atomically reduce MarketState
     append canonical event checksum input
-> finalize event checksum
-> calculate final-state checksum
-> write replay result
```

若 reducer 失敗，clock 與 state transition 不得部分可見，run status 為 failed。

## 8. Backtest flow

### 8.1 Initialization

```text
frozen execution plan
-> load effective strategy and parameters
-> confirm strategy universe equals planned universe
-> initialize empty MarketState per symbol
-> initialize strategy instance
-> initialize simulation／ledger
-> open selected streams
```

初始化不得讀取第一個 event 的未來內容。需要 instrument multiplier 等 prerequisite
必須在第一個 callback 前驗證。

### 8.2 Per-event sequence

```text
1. merge selects next event by OrderingRule
2. replay validates event and monotonic time
3. replay clock advances to event.match_time
4. reducer atomically updates that symbol's MarketState
5. pending simulated orders inspect the event if eligible
6. resulting order／fill／accounting feedback becomes deterministic
7. strategy receives current event + updated read-only state + allowed feedback
8. strategy emits indicator／order intent
9. new intents are validated and registered
10. new orders wait for a subsequent eligible event
```

步驟 5 至 9 的精確 callback API 由 replay、strategy 與 execution-sim design 決定，
但必須維持：

- 目前 event 先更新 MarketState。
- 既有 pending orders 可以使用目前 event；由目前 callback 新建的 order 不行。
- strategy 看不到下一 event。
- 同一 `match_time` 仍依 tie-break 一個 event 一個 event 處理。
- accounting feedback 不早於產生它的 market event。

### 8.3 Origin 與 eligible event

```text
event E1
  -> state E1
  -> strategy emits order O1

event E2 for another symbol
  -> O1 does not obtain target-symbol price

event E3 for O1 target symbol with required price
  -> O1 first becomes fill-eligible
```

若 E3 與 E1 具有相同 `match_time`，只要 E3 在 deterministic order 中位於 E1
之後，仍是 subsequent event。這是平台順序，不代表真實 exchange causality。

### 8.4 Finalization

```text
all streams exhausted
-> finish pending order policy
-> calculate legal final marks
-> reconcile orders／fills／cash／positions／P&L
-> finalize strategy summary
-> finalize event／state／strategy／ledger checksums
-> write run manifest and artifacts
-> publish successful／degraded result
```

reconciliation 或 strategy finalize 失敗時只能發布 failed／partial artifacts。

## 9. Multi-stream merge

### 9.1 Stream selection

Planner 將 explicit universe 映射為：

```text
(market, trading_date, symbol) -> one or more compatible event streams
```

replayer 不列舉 universe 外 payload。若實體 cache 合併多商品，index／reader 仍必須
能只讀選定範圍。

### 9.2 Merge contract

每個 input stream：

- 事件依同一 OrderingRule 相容順序提供。
- 暴露目前 head event。
- 不要求把 stream 全部載入記憶體。
- 發現內部時間倒退時回報錯誤。

merge：

```text
open N selected streams
-> keep bounded head／buffer per stream
-> choose minimum full OrderingKey
-> emit event
-> advance only selected stream
-> repeat
```

buffer size、I/O completion 或 stream discovery order 不得改變結果。

### 9.3 Duplicate events

完全相同 canonical events 不因 merge 自動去重。它們各自形成 accepted event、
state version 及 callback。因內容完全相同，其彼此相對位置不影響 domain sequence
的可觀察內容。

## 10. MarketState flow

```text
accepted event
-> validate reducer preconditions
-> compute complete proposed state
-> commit one atomic transition
-> increment state version once
-> expose immutable view
```

依 event kind：

- `QuoteSnapshot`：完整取代 book；同一 event 的 trade、volume、flags 一起套用。
- `BookSnapshot`：完整取代 book。
- `TradeBatch`：保存 batch 並更新明確提供的累計／stats。
- `MarketStat`：只更新具有合法 timing 的統計。
- `MarketStatus`：只更新已確認語意的 status／flags。

optional field 的 absent／clear／unknown／unchanged 語意由 normalizer 明確表達，不能
在 reducer 中依值猜測。

## 11. Strategy 與 simulation feedback flow

### 11.1 Strategy output

```text
event identity + state version
-> strategy callback
-> indicator records
-> order intents
```

每個 output 保存 origin identity。universe 外或 invalid intent 產生 rejection，
不改成其他 order type。

### 11.2 Simulation trace

```text
intent
-> simulated order
-> subsequent eligible event
-> fill decision
-> slippage／quantity allocation
-> fee／tax／multiplier
-> cash／position transition
-> feedback
```

每一箭頭都必須可追溯。無 source market event 的 fill 或無 fill 的 ledger change
是 invariant violation。

## 12. Result flow

### 12.1 Manifest inputs

Run manifest 聚合：

- source／cache identities and checksums
- event／ordering／model versions
- effective strategy／simulation configuration
- warning／skipped scope
- counts and timing
- domain checksums
- order／fill／ledger artifacts
- successful／failed／degraded status

### 12.2 Checksum lineage

```text
source bytes ----------------------> source checksum
canonical ordered events ----------> event checksum
final canonical MarketState -------> state checksum
strategy outputs ------------------> strategy checksum
orders／fills／ledger --------------> accounting checksum
selected domain results -----------> result checksum
```

wall clock、absolute path、process ID、log timestamp 及 debug output 不進入 domain
checksum。

### 12.3 Publish

successful 或 degraded result 只有在所有必要 artifact 寫入並驗證後發布。failed run
可以保存 diagnostics，但 manifest 必須標示 partial，且不能出現成功績效外觀。

## 13. Degraded flow

Degraded execution 不是自動 error recovery：

```text
user explicitly enables degraded policy
-> planner identifies exact skipped partitions／ranges
-> plan displays impact
-> invalid／corrupt content is not normalized
-> remaining compatible streams replay normally
-> result records skipped scope and warnings
-> status = degraded
```

下列 invariant 不可降級：

- valid `match_time`
- monotonic replay clock
- deterministic ordering
- event／state atomicity
- strategy no-lookahead
- ledger reconciliation
- secret handling

無法隔離錯誤而仍維持 invariant 時，run 必須 failed。

## 14. Recovery matrix

| 問題 | 可恢復來源 | 動作 | 需要網路 |
| --- | --- | --- | --- |
| Staging 中斷 | checkpoint 或重抓 | resume／restart sync | 是 |
| Source incomplete | Teralion | sync missing scope | 是 |
| Source corrupt | Teralion 或明確 repair | reacquire／new revision | 是；通常 |
| Cache absent | complete source | build | 否 |
| Cache stale | complete source | rebuild | 否 |
| Cache corrupt | complete source | discard and rebuild | 否 |
| Run failed | immutable inputs | fix config/code then rerun | 否；資料完整時 |
| Result artifact corrupt | immutable inputs | rerun | 否；資料完整時 |

任何 recovery 都不能以靜默覆寫 complete source 或變更舊 run manifest 達成。

## 15. 里程碑資料流

### M1

```text
versioned 2330 fixture
-> TWSE normalizer
-> QuoteSnapshot stream
-> ordering
-> MarketState
-> ExampleStrategy
-> event／state checksum
```

### M2

```text
Teralion
-> sync／verify
-> complete source
-> replay cache
-> offline replay
-> strategy
-> market／limit fills
-> ledger／manifest
```

### M3

```text
TWSE stream ─┐
             ├-> deterministic merge -> multi-symbol state／strategy／simulation
TAIFEX stream┘
```

### M4

新增 normalizer、interface fixture 與 metadata mapping；既有 source、event、replay、
strategy 及 result flows 不改變。

## 16. 驗證重點

- Sync：cursor 全部走完，中斷不發布 complete。
- Source：相同資料重用，不同 checksum 不靜默覆寫。
- Cache：失效可離線重建，不重新下載。
- Selection：universe 外 stream 不開啟。
- Ordering：打亂 input／stream order 仍產生相同 checksum。
- State：snapshot replacement 與每 event 一次 atomic transition。
- Strategy：updated state 可見，future event 不可見。
- Simulation：origin event 不 fill 新 order。
- Result：reconciliation、provenance 與 status 一致。
- Security：offline path 無 credential，artifact 無 secret。

詳細測試由[驗證計畫](../verification/plan.md)與
[驗收規格](../verification/acceptance.md)定義。
