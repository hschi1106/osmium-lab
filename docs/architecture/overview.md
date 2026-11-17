# 架構與 ownership 總覽

本頁是 production responsibility 的 canonical map。產品範圍由
[`product-requirements.md`](../product-requirements.md) 定義；event semantics 與 execution
細節分別由[回播模型](replay-model.md)和[執行與帳務模型](execution-model.md)定義；資料來源見
[marketdata 介面](../interfaces/marketdata.md) 與 [ADR-0007](decisions/0007-marketdata-native.md)。

## 核心資料路徑

```text
marketdata (/srv/md/v1，唯讀)
  ↓ osmium-md：manifest 驗證、parquet 讀取、mapping
DomainEvent
  ↓ replay engine
MarketState
  ↓ read-only context
Strategy
  ↓ intent
Execution
  ↓ fill
Accounting
  ↓
Artifacts
```

osmium-lab 只讀 marketdata，不下載、不 staging、不維護自己的 source 或 cache。marketdata 的
manifest row（path、rows、`schema_version`、`git_commit`、`code_hash`、`inputs_hash`）就是一次
讀取的資料 identity。

不變條件：

- marketdata 欄位只在 `osmium-md` 邊界解讀，core 只使用 versioned domain types；
- replay clock 是 `observed_time`（feed 內 `exch_ts` 的 running max），同時間事件使用版本化
  deterministic ordering；
- MarketState 只根據成交與完整 snapshot 更新，不重建 order-by-order queue；
- strategy 只讀 post-event state，沒有 next-event/future access；
- planner 先凍結 explicit universe，replayer 只開所需 streams；
- execution/accounting 不改寫 event、state 或 source evidence。

## 唯一 owner

### osmium-md：marketdata → DomainEvent

`crates/osmium-md` 擁有 `md_root` layout、manifest 讀取與檔案一致性檢查、parquet projection／
row-group pruning、row 驗證、feed 層級 observed time，以及 books／trades → `DomainEvent` 的
mapping。讀取前後比對 manifest，資料在讀取中被更新即失敗。它不擁有 replay ordering、
cross-event MarketState 或 fill policy。

### Planner：validated/frozen execution facts

`crates/run-planner` 擁有 effective config、sessions、instrument contract/economics identity 與
canonical encoding。Planner 不讀 parquet、不 replay、不執行 strategy。

### Replay engine：deterministic stream playback

`crates/replay-engine` 驗證 plan/binding，選擇性開啟 streams，以 bounded merge 依 ordering key 推進
`ReplayClock`，並協調 event transition。它不讀檔、不解讀來源欄位、不決定成交。

### MarketState reducer：cross-event market state

`crates/market-state` 是 book/trade/volume/auction/phase cross-event state 與 transition 的唯一 owner。
Reducer 先驗證後 atomic commit；read-only `MarketStateView` 提供給 strategy。`MarketStateProfile`
宣告每個 source format 接受的 event kinds；marketdata feed 的 profile 由 `Feed::state_profile` 提供。
資料邊界與 runner 不得另建平行 auction FSM。

### Strategy：decision logic only

`crates/strategy-api` 擁有 lifecycle、context、output transaction、compiled factory/registry、order
intent、scheduled request、timer 與 feedback API。Concrete strategy 只決定何時輸出 intent；不能修改
market state、決定 fill、結算 cash，或進行 I/O/future lookup。

### ExecutionSim：order lifecycle 與 fill semantics

`crates/execution-sim` 擁有 acceptance、restriction、activation、partial fill、cancellation、allocation、
fill evidence 與 order status。`Simulator` 處理 market-event causal path；`ScheduledDepthSimulator`
處理 control-time visibility/activation/expiry/depth consumption。兩者的 trigger/time model 本質不同，
但共用的 order/fill/accounting policy 仍由此 crate 擁有。

### Accounting：fill → ledger/positions/P&L

同一 `execution-sim` crate 的 accounting owner 驗證 instrument economics，原子套用 fills、fees、taxes、
cash charges、cash、positions、average cost、realized/unrealized P&L 與 reconciliation。Runner 只提供
已驗證 config 與 final mark，不重算 formula。

### Runner：orchestration only

`crates/osmium-runner` 串起 replay callbacks、simulation feedback、ledger settlement、finalization 與
artifact publication。它不解析 config 檔或 parquet，也不重複實作 execution/accounting policy。

目前沒有 CLI composition root；以 `osmium-md` 串接 planner 與 runner 的入口尚在重建中。

## 依賴方向

```text
osmium-runner → replay-engine / strategy-api / execution-sim / market-state
osmium-md     → replay-engine / market-state / market-types
execution-sim → replay-engine / strategy-api / market-types
strategy-api  → replay-engine / market-state / market-types
run-planner   → strategy-api / market-types
replay-engine → market-state → market-types
```

Domain crates 不反向依賴 `osmium-md`、parquet／arrow 或 artifact serializer。

## Atomicity 與 determinism

- 讀取的資料必須與 manifest 一致，且讀取期間 manifest 不得改變；
- event transition 失敗不留下 advanced clock 或 partial state；
- callback error/panic 不提交該 callback 的 partial output；
- fill、cash、position、fees/taxes 與 P&L 必須 reconciliation；
- concurrency 不得改變 warnings、event order、callbacks、allocation 或 accounting result；
- run output 使用 staging + create-new publication，不覆寫既有 evidence。
