# 系統架構總覽

## 1. 文件目的

本文件定義 `osmium-lab` 的高階系統邊界、邏輯元件、責任分工及依賴方向，作為
後續 module／crate 設計的共同基準。

產品範圍及需求以[產品需求](../product-requirements.md)與
[詳細需求](../requirements/replay.md)為準。本文件描述 logical architecture，
不直接指定：

- crate 數量或名稱
- Rust module layout
- trait 或 function signature
- 本地檔案格式
- CLI syntax
- concurrency implementation

同一 logical component 可以在 M1 先與其他元件位於同一 crate，之後再依清楚的
依賴邊界拆分。不得只為了符合架構圖而提前建立沒有獨立責任的 crate。

## 2. 架構目標

架構必須優先支持：

1. 一份設定可建立 plan、準備資料並執行 backtest。
2. 已驗證來源資料下載一次後可跨多次回測重用。
3. replay cache 可以刪除並由本地來源資料重建。
4. 相同資料、版本及設定產生相同事件、策略輸出、fill 與 P&L。
5. `match_time` 是唯一 replay time。
6. Teralion wire format 不洩漏至 domain event、策略或模擬層。
7. MarketState 只表達 trade 與完整五檔 snapshot 可支持的精度。
8. 策略讀取市場狀態但不能修改。
9. replayer 只開啟 explicit universe 需要的 streams。
10. 資料準備完成後，replay／backtest 預設完全離線。
11. 每個 market／instrument session 以自己的開收盤時間建立前後五分鐘的下載及
    回播 window。

## 3. 系統 context

```text
                         online preparation

使用者設定 ──> osmium-lab planner ──> Teralion Feed Archive
                    │                         │
                    │                         v
                    │                  source responses
                    │                         │
                    v                         v
                execution plan ──> sync／verify／local source repository
                                             │
                                             v
                                    rebuildable replay cache

                         offline execution

使用者設定 ──> execution plan ──> selected event streams
                                        │
                                        v
                                  replay engine
                                        │
                         ┌──────────────┴──────────────┐
                         v                             v
                  read-only MarketState          current event
                         └──────────────┬──────────────┘
                                        v
                                     strategy
                                        │
                                 indicator／intent
                                        v
                                execution simulation
                                        │
                                  fill／accounting
                                        v
                              run artifacts／inspection
```

`osmium-lab` 第一版是本機執行的平台，不需要分散式服務、message broker 或遠端
strategy runtime。圖中的邏輯邊界可以存在同一 process，但資料、依賴及可見性仍
必須遵守本文件。

## 4. 核心架構原則

### 4.1 Source 與 derived artifact 分離

已驗證本地來源資料是可重用事實；replay cache 是加速回播的衍生 artifact。

```text
verified source data --normalize/build--> replay cache
       │                                      │
       │                                      └── 可刪除、失效、重建
       └── 不因 cache 失效而重新下載或覆寫
```

source checksum、event schema、normalizer mapping 及相關版本建立 lineage。
cache 不得成為唯一能重建 domain events 的資料。

### 4.2 Wire 與 domain 分離

Teralion endpoint、cursor、JSON 欄位及各 market／format wire type 屬於 adapter
邊界。replay、MarketState、strategy 與 simulation 只依賴標準 domain types。

```text
Teralion wire type
       │
       v
market／format normalizer
       │
       v
versioned domain event
```

normalizer 是唯一把來源格式語意轉入 domain 的位置。未知 format 不得直接穿透，
也不得在 strategy 中臨時解讀。

### 4.3 Event time 單一來源

`match_time` 是 replay clock 唯一輸入。download time、file order、thread completion
或 wall clock 只可作為操作 metadata，不得改變 domain result。

相同 `match_time` 依
[ADR-0001](decisions/0001-match-time-ordering.md)的版本化 tie-break 排序。平台
排序只保證重跑一致，不代表交易所真實全域封包順序。

### 4.4 Snapshot state

完整五檔事件取代同商品舊 snapshot，不合併成推測的 order-level book。

MarketState reducer 遵守
[ADR-0002](decisions/0002-snapshot-market-state.md)：

- 每個 accepted event 形成一次原子 state transition。
- 策略取得更新完成後的唯讀 view。
- 不推論 queue position、hidden liquidity 或逐筆委託。
- 衍生 best bid／ask 只能來自目前完整 snapshot。

### 4.5 Explicit universe 與 selective I/O

strategy 在 execution plan 建立前宣告 explicit market／symbol universe。planner
將它與 strategy 選取的 semantic session kinds 轉成需要的 source／cache
partitions，replayer 只開啟對應 streams。

market／instrument session、前後五分鐘的 download／replay windows，以及
WarmUp／Active／CoolDown strategy phases 依
[ADR-0003](decisions/0003-session-windows-and-strategy-activation.md)建立。strategy
不自行指定 Teralion query 或絕對 session 時鐘。

filesystem 上存在其他商品，不代表 execution 可以掃描、解析或載入它們。

### 4.6 Default offline execution

只有 plan／sync 路徑可以使用 Teralion credential 及網路。verify 本地資料、
replay、backtest 與 inspect 不依賴網路。

若離線 execution 缺少資料，系統回報需要另行 sync；不得在 backtest 途中自動
下載。

## 5. 邏輯元件

### 5.1 Configuration 與 Planner

責任：

- 解析並驗證 effective configuration。
- 取得 strategy universe。
- 解析 strategy session selection，並以版本化 calendar／profile 建立 SessionPlan。
- 對每個 session open／close 固定加入前後五分鐘的 download／replay windows。
- 比較所需 partition 與本地狀態。
- 建立固定的 sync／execution plan。
- 標示 reuse、download、rebuild、incomplete、corrupt 及 degraded scope。

Planner 不負責：

- 下載 tick payload。
- 正規化 source format。
- 排序 market events。
- 執行策略或模擬 fill。

### 5.2 Teralion Adapter

責任：

- 實作 coverage、ticks、每日商品資料及 opaque cursor。
- 將 request／response 限制在 Teralion interface 邊界。
- 提供同步層可驗證的 page result 與安全錯誤 context。
- redaction credential。

Teralion wire types 不得成為 replay 或 strategy 的 public API。

### 5.3 Source Data Repository

責任：

- 以 `market + trading_date + symbol` 管理來源 partition。
- 保存 source payload、manifest、筆數、checksum、format 及商品資料。
- 區分 missing、building、complete、incomplete、corrupt。
- atomic publish，且不靜默覆寫 complete partition。
- 提供本地 verify 與 offline read。

Repository 不解讀 domain event 語意，也不維護 MarketState。

### 5.4 Normalizer Registry

責任：

- 依 market／format 選擇明確 normalizer。
- 驗證來源欄位及 `match_time`。
- 把 wire payload 轉成少量 versioned domain events。
- 保存 unknown raw values 並產生 warning。
- 辨識第一版刻意排除的 `close`／`stats` source kinds，保留可檢查的略過摘要但不建立
  domain event。
- 使用真實 source fixtures 固定 mapping。

Registry 不提供 unknown format fallback，不從 snapshot 反推逐筆委託。

### 5.5 Replay Cache Builder 與 Repository

責任：

- 從 complete source partition 建立 canonical domain event streams。
- 綁定 source checksum、schema、mapping、ordering dependency 及 cache format。
- 依 symbol／trading date 提供 selective stream read。
- 發現不相容或損壞 cache 時拒絕並允許離線重建。

cache builder 不修改或刪除來源資料。

### 5.6 Replay Engine

責任：

- 只開啟 execution plan universe streams。
- 驗證 stream schema、ordering compatibility 及時間單調性。
- 只接受位於 planned replay windows 的 `match_time`，並提供 session phase context。
- 以 bounded streaming merge 選出下一事件。
- 推進 replay clock。
- 依序協調 MarketState、strategy 與 strategy output。
- 產生 event stream 及 final-state checksum 所需的 canonical sequence。

Replay Engine 不呼叫 Teralion、不解讀 wire payload，也不決定 fill price。

### 5.7 MarketState Store 與 Reducer

責任：

- 為 universe 內每個商品保存獨立狀態。
- 原子套用 accepted domain event。
- 完整取代 book snapshot。
- 保存最近 trade／batch、累計量、flags、`last_match_time` 及 state version。
- 向 strategy 暴露唯讀 view。

Reducer 不知道下一事件，不接受 strategy mutation，也不重建 queue。

### 5.8 Strategy Runtime

責任：

- 建立編譯期連結的 Rust strategy instance。
- 驗證參數並取得 explicit universe 及 semantic session selection。
- 依 replay order 呼叫 event callback。
- 提供 WarmUp／Active／CoolDown context；WarmUp／Active 接受新 order intent，
  CoolDown 拒絕。
- 提供目前 event 與更新後 read-only MarketState。
- 收集 indicator、order intent 及 simulation feedback。
- 將 strategy error／panic 轉成 failed run。

Strategy Runtime 不讓策略取得 future event、source repository 或可修改 state handle。

### 5.9 Execution Simulation 與 Accounting

責任：

- 驗證 order intent。
- 只以 WarmUp／Active phase 中、位於 origin event 之後且符合 fill model 的目標商品
  eligible event 判定 fill。
- 套用 versioned fill、slippage、fee、tax 及 multiplier。
- 更新 order、fill、cash、position 與 P&L ledger。
- 執行 reconciliation。
- 向 strategy 回傳 deterministic feedback。

Simulation 不修改 market event／state，不宣稱 queue position 或真實 exchange matching。

### 5.10 Result Writer 與 Inspector

責任：

- 保存 run status、effective configuration 及 execution plan。
- 保存 source／cache checksums 與所有影響結果的版本。
- 保存 warning、strategy output、orders、fills、positions、P&L 及 domain checksums。
- 清楚區分 successful、failed、partial 與 degraded artifact。
- 在不重跑 backtest、不存取 Teralion 的情況下 inspect。

操作 timestamp 不得混入要求跨執行相同的 domain checksum。

## 6. 依賴方向

邏輯依賴必須朝 domain core 收斂，不能讓外部 adapter 反向污染核心：

```text
CLI／orchestration
  ├── data sync ──> Teralion adapter
  │       └───────> source repository
  └── execution
          ├───────> cache/event stream ports
          ├───────> replay engine
          │           └──> domain events + MarketState
          ├───────> strategy runtime
          ├───────> simulation/accounting
          └───────> result writer
```

必要限制：

- domain event 與 MarketState 不依賴 Teralion response types。
- strategy API 不依賴 source repository 或 cache implementation。
- replay engine 透過 event stream boundary 讀取資料，不依賴來源 JSON。
- simulation 依賴 domain event／state view，不依賴 Teralion adapter。
- infrastructure 可以依賴 domain interfaces；domain 不反向依賴 infrastructure。
- result serialization 不成為執行中 domain state 的唯一 representation。

具體 dependency graph 由 workspace design 決定並以 compile-time boundary 驗證。

## 7. Online 與 offline 邊界

| 能力 | 網路 | API key | 可修改來源資料 | 可執行策略 |
| --- | --- | --- | --- | --- |
| plan（純本地） | 否 | 否 | 否 | 否 |
| plan（需要 coverage refresh） | 是 | 是 | 否 | 否 |
| sync | 是 | 是 | 只可發布新 revision／partition | 否 |
| verify | 否 | 否 | 否；derived cache repair 需明確記錄 | 否 |
| cache build／rebuild | 否 | 否 | 否 | 否 |
| replay | 否 | 否 | 否 | 可選 |
| backtest | 否 | 否 | 否 | 是 |
| inspect | 否 | 否 | 否 | 否 |

credential 只能進入 online adapter 的執行 context，不得進入 source payload、
manifest、cache、log、result 或 strategy context。

## 8. 原子性與失敗邊界

### 8.1 Data publish

同步寫入 staging；只有 cursor 完成、必要 metadata 齊全及 checksum 驗證通過後，
才發布 complete source partition。失敗保留 building／incomplete，不得看似完整。

### 8.2 Cache publish

cache 完整建立並驗證 lineage 後才可取代既有有效 cache。cache build 失敗不影響
source partition。

### 8.3 Event processing

accepted event 的 clock advance、state transition、strategy callback 依固定順序
協調。若 state transition 失敗，不得留下策略可見的 partial state 或 clock advance。

### 8.4 Accounting

fill record、cash、position、fee、tax 與 P&L transition 必須原子一致。reconciliation
失敗使 run failed，不得發布 successful performance。

## 9. Version 與 provenance

至少下列 identity 必須沿資料流保存：

- source schema／format
- source checksum
- normalizer mapping
- domain event schema
- ordering rule
- canonical encoding／checksum
- replay cache format
- strategy implementation
- fill／slippage／fee／tax model
- accounting／marking policy
- instrument metadata／multiplier source

derived artifact 的相容性由其直接輸入與相關版本決定。能由 complete source data
重建的不相容 cache 應拒絕並重建；無法安全重建或判定的資料應拒絕執行。

## 10. Concurrency 與效能邊界

架構允許：

- 多 page download
- parallel source validation／normalization
- bounded prefetch
- 多 stream I/O
- cache build parallelism

但 concurrency 不得改變：

- published source content
- warning 集合
- event order
- MarketState transition sequence
- strategy callback sequence
- order／fill allocation
- accounting result

所有會影響可觀察順序的資料必須在進入 sequential domain boundary 前轉為固定排序。
第一版可以先使用單執行緒 domain loop，等 correctness checksum 固定後再最佳化。

## 11. 里程碑演進

### M1：TWSE 回播核心

只需要：

- 保存的 2330 fixture
- 一個 TWSE normalizer
- `QuoteSnapshot`
- 單 stream ordering
- snapshot MarketState
- ExampleStrategy
- event／state checksum

不需要 online adapter、source repository lifecycle、正式 cache、fill 或 CLI。

### M2：真實資料與離線回測

加入：

- Teralion adapter
- source repository、integrity 與 replay cache
- plan／sync／verify／backtest／inspect orchestration
- market／limit fill
- accounting 與 run manifest

### M3：TAIFEX 與多商品

加入：

- TAIFEX normalizer
- `TradeBatch` 與 `BookSnapshot`
- trading-date boundary
- 多 stream deterministic merge
- futures multiplier 與 accounting

### M4：市場擴充

依實際 fixture 逐一加入 TPEx、warrants、options mappings，不改變核心 replay、
strategy 或 simulation 邊界。

## 12. 架構驗證

架構邊界至少由下列方式驗證：

- fixture tests 證明 wire／domain 分離及 mapping。
- dependency／compile-fail tests 防止 strategy 修改 MarketState。
- shuffled-input tests 證明 ordering determinism。
- reducer tests 證明 snapshot replacement 與 atomic transition。
- spy integration tests 證明 universe 外 streams 不被開啟。
- offline acceptance test 證明 backtest 不使用 network／credential。
- cache rebuild test 證明 source／derived 分離。
- reconciliation test 證明 fill 與 ledger traceability。

正式 mapping 見 [traceability matrix](../traceability.yaml)，驗證策略見
[verification plan](../verification/plan.md)。

## 13. 待詳細設計決定

| 議題 | 下游文件 |
| --- | --- |
| Workspace／crate boundaries | 各 design 文件完成後決定 |
| Source layout、manifest 與 atomic publish | [data sync 設計](../design/data-sync.md) |
| Domain types 與 canonical encoding | [market types 設計](../design/market-types.md) |
| Reducer API 與 state storage | [market state 設計](../design/market-state.md) |
| Stream merge、clock 與 callback loop | [replay engine 設計](../design/replay-engine.md) |
| Strategy trait 與 context | [strategy API 設計](../design/strategy-api.md) |
| Fill 與 ledger pipeline | [execution simulation 設計](../design/execution-sim.md) |
| Config、manifest 與 command UX | [CLI 操作](../operations/cli.md) |

詳細設計可以改變 implementation structure，但不得改變本文件的 source／derived、
wire／domain、online／offline、read-only strategy 或 deterministic execution 邊界。
