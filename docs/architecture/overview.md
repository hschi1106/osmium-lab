# 系統架構總覽

## 1. 核心邊界

`osmium-lab` 將資料準備與離線執行分開：

```text
online
Teralion -> sync -> staging -> verified source

offline
verified source -> normalizer -> replay cache -> replay engine
                                                -> MarketState
                                                -> Strategy
                                                -> Simulation / Accounting
                                                -> Run artifacts
```

架構遵守下列不變條件：

- verified source 是可重用事實；replay cache 是可刪除、可重建的衍生 artifact。
- Teralion wire type 只存在於 adapter／normalizer 邊界，domain 與 strategy 不依賴 wire schema。
- `match_time` 是唯一 replay clock；相同時間使用版本化 deterministic tie-break。
- `MarketState` 只表達成交與完整 snapshot 能支持的狀態，不重建逐筆委託或 queue。
- strategy 只能取得 read-only state，不能修改 event、clock 或 source。
- execution plan 先凍結 explicit universe，replayer 只開啟所需 streams。
- source 準備完成後，執行流程預設離線。

## 2. Workspace 責任

| Crate | 責任 |
| --- | --- |
| `market-types` | exact domain primitive、event schema 與 canonical encoding |
| `market-state` | snapshot reducer、read-only view 與 state checksum |
| `normalizer/{twse,tpex,taifex}` | market／format wire mapping 與驗證 |
| `data-sync` | Teralion query、cursor、source repository、verify 與 replay cache |
| `run-planner` | effective config、partition、session plan 與 execution plan |
| `replay-engine` | stream validation、deterministic merge、clock 與 event occurrence |
| `strategy-api` | strategy lifecycle、context、output、orders、timers 與 registry |
| `execution-sim` | fill evidence、scheduled execution 與 accounting |
| `osmium-config` | YAML schema 與跨區塊驗證 |
| `osmium-runner` | replay、strategy、simulation、accounting 與 artifact 協調 |
| `osmium-cli` | command contract、輸出格式、TUI 與 exit status |

依賴方向由 orchestration 指向 domain core：

```text
osmium-cli -> osmium-config / osmium-runner / data-sync
osmium-runner -> replay-engine / strategy-api / execution-sim
replay-engine -> market-state -> market-types
normalizers -> market-types
```

domain crates 不反向依賴 CLI、filesystem layout、Teralion transport 或 run artifact serializer。

## 3. 元件責任

### Configuration 與 Planner

- 解析 `config_version: 2` 並拒絕 unknown／secret fields。
- 解析 compiled strategy、parameters、universe、instrument economics 與 simulation policy。
- 以 calendar、instrument profile 與 session kinds 建立 `SessionPlan`。
- 比較 source/cache state，產生固定的 `ExecutionPlan` 與 identity。

Planner 不下載 payload、不正規化事件，也不執行 strategy。

### Source Adapter 與 Repository

- 以 Teralion coverage、range、ticks、instrument 與 opaque cursor 取得資料。
- 使用 frozen query、staging、checksums 與 atomic publish 建立 source revision。
- 提供本地 verify 與 immutable read。

Repository 不解讀 domain event，也不維護 market state。

### Normalizer 與 Cache

- 依 market、instrument kind 與 source format 選擇唯一 mapping。
- 驗證 identity、`match_time`、exact price／quantity、snapshot shape 與 market-specific flags。
- 產生 versioned `DomainEvent` 或明確的 known-skip diagnostic。
- cache descriptor 綁定 source checksum、mapping、event schema、ordering 與 cache format。

未知格式沒有 generic fallback。cache build 失敗不修改 published source。

### Replay 與 MarketState

- 驗證 streams 與 plan 相容，使用 bounded merge 依 ordering key 取出事件。
- 推進 `ReplayClock`，原子套用 state transition，再產生 strategy context。
- 每個 instrument 維護獨立 state；完整 book snapshot 直接替換舊 book。

Replay Engine 不呼叫 Teralion、不解讀 wire payload，也不決定 fill。

### Strategy Runtime

- 解析 registry 中的 compiled strategy，驗證 identity、parameters、universe 與 sessions。
- 固定執行 `initialize`、session/timer/event callbacks、feedback 與 `finalize`。
- callback output 以 transaction 收集；error 或 panic 不提交 partial output。

Strategy 不取得 mutable `MarketState`、next event 或 source repository handle。

### Simulation 與 Accounting

- 驗證 order intent 與 scheduled request。
- 依 execution policy、origin boundary、TradingContext 與可觀察 evidence 判定 fill。
- 套用 slippage、fee、tax、multiplier、cash、position 與 marking。
- 產生 deterministic feedback 並在結束時 reconciliation。

Simulation 不修改 replay event 或 MarketState，也不宣稱重建真實撮合。

### Result Writer 與 Inspector

- 以 staging 建立 output，在成功或明確失敗狀態下發布 run artifacts。
- 保存 effective config checksum、plan identity、版本、checksums、strategy metadata、warnings、orders、fills 與績效。
- `inspect` 只讀既有 artifacts，不重跑策略或存取網路。

## 4. 線上與離線能力

| 能力 | 網路／API key | Source mutation | Strategy |
| --- | --- | --- | --- |
| `config check`、本地 `plan` | 否 | 否 | 僅建立與驗證 instance |
| `data sync` | 是 | 發布新 source revision | 否 |
| `data verify` | 否 | 否 | 否 |
| `cache prepare` | 否 | 否，只建立 derived cache | 否 |
| `replay` | 否 | 否 | 否 |
| `backtest` | 否 | 否 | 是 |
| `run` | 視 plan 而定 | 可能發布 source/cache/run | 有 `--output` 時是 |
| `display` | 否 | 否 | 否 |
| `inspect` | 否 | 否 | 否 |

credential 只能進入 sync transport context，不得進入設定、manifest、cache、log、strategy context 或 run output。

## 5. 原子性與可重現

- source 完成所有 cursor pages、metadata 與 checksum 驗證後才可 publish。
- cache 完整建立並驗證 lineage 後才可 publish。
- event transition 失敗時，不得留下已推進 clock 或 partial state。
- callback 失敗時，不得提交該 callback 的 partial output。
- fill、cash、position、fee、tax 與 P&L transition 必須一致；reconciliation 失敗使 run failed。
- concurrency 可以用於下載、驗證、正規化與 prefetch，但不得改變 warning 集合、event order、callbacks、fill allocation 或 accounting result。

詳細流程見 [資料流程與儲存](data-flow.md)、[回播模型](replay-model.md)與[模擬與帳務](execution-model.md)。
