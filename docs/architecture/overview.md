# 架構與 ownership 總覽

本頁是 production responsibility 的 canonical map。產品範圍由
[`product-requirements.md`](../product-requirements.md) 定義；event semantics 與 execution
細節分別由[回播模型](replay-model.md)和[執行與帳務模型](execution-model.md)定義。

## 核心資料路徑

```text
Source
  ↓ provider adapter / normalizer
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

Online acquisition 與 offline execution 分離：

```text
online:  provider → staging → verified immutable source
offline: verified source → rebuildable cache → replay → backtest → artifacts
```

不變條件：

- verified source 是可重用 fact；cache 是 derived、deletable、rebuildable artifact；
- provider wire type 只存在 adapter boundary，core 只使用 versioned domain types；
- `match_time` 是唯一 replay clock，同時間事件使用版本化 deterministic ordering；
- MarketState 只根據成交與完整 snapshot 更新，不重建 order-by-order queue；
- strategy 只讀 post-event state，沒有 next-event/future access；
- planner 先凍結 explicit universe，replayer 只開所需 streams；
- execution/accounting 不改寫 event、state 或 source evidence。

## 唯一 owner

### Provider adapter：wire → neutral evidence

`crates/providers/teralion` 擁有 credential、network/query、opaque cursor、wire envelope、market-specific
validation、normalization 與 source sync orchestration。它把 raw record 轉成 provider-neutral
`DomainEvent`，或明確 skip/reject；不擁有 replay ordering、cross-event MarketState 或 fill policy。

### data-sync：verified source 與 cache lifecycle

`crates/data-sync` 擁有 provider-neutral staging/repository、manifest/checksum verification、immutable
source publication，以及 replay cache codec、identity、reuse/rebuild/publication。它不解讀 Teralion
fields，也不決定 session、strategy 或 fill。

### Planner：validated/frozen execution facts

`crates/osmium-config` 解析 `config_version: 3` 並建立 typed input；`crates/run-planner` 擁有 effective
config、sessions、partitions、instrument contract/economics identity、local artifact classification 與
`ExecutionPlan`。Planner 不下載、normalize、replay 或執行 strategy。

### Replay engine：deterministic stream playback

`crates/replay-engine` 驗證 plan/binding，選擇性開啟 streams，以 bounded merge 依 ordering key 推進
`ReplayClock`，並協調 event transition。它不連接 provider、不解碼 wire、不決定成交。

### MarketState reducer：cross-event market state

`crates/market-state` 是 book/trade/volume/auction/phase cross-event state 與 transition 的唯一 owner。
Reducer 先驗證後 atomic commit；read-only `MarketStateView` 提供給 strategy。Provider 與 runner 不得
另建平行 auction FSM。

### Strategy：decision logic only

`crates/strategy-api` 擁有 lifecycle、context、output transaction、compiled factory/registry、order
intent、scheduled request、timer 與 feedback API。Concrete strategy 只決定何時輸出 intent；不能修改
market state、決定 fill、結算 cash，或進行 I/O/future lookup。

### ExecutionSim：order lifecycle 與 fill semantics

`crates/execution-sim` 擁有 acceptance、restriction、activation、partial fill、cancellation、allocation、
fill evidence 與 order status。`Simulator` 處理 market-event causal path；`ScheduledDepthSimulator`
處理 control-time visibility/activation/expiry/depth consumption。兩者的 trigger/time model 本質不同，
但共用的 order/fill/accounting policy仍由此 crate 擁有。

### Accounting：fill → ledger/positions/P&L

同一 `execution-sim` crate 的 accounting owner 驗證 instrument economics，原子套用 fills、fees、taxes、
cash charges、cash、positions、average cost、realized/unrealized P&L 與 reconciliation。Runner 只提供
已驗證 config 與 final mark，不重算 formula。

### Runner：orchestration only

`crates/osmium-runner` 串起 replay callbacks、simulation feedback、ledger settlement、finalization 與
artifact publication。它不解析 YAML、provider wire 或重複實作 execution/accounting policy。

### CLI：composition root 與 user contract

`crates/osmium-cli` 擁有 arguments、output/error contract，組合 provider、registry、planner、runner 與
filesystem workflow。它可選擇 concrete implementation，但不成為 domain policy owner。

## 依賴方向

```text
osmium-cli → osmium-config / providers/teralion / data-sync / osmium-runner
osmium-runner → replay-engine / strategy-api / execution-sim
replay-engine → market-state → market-types
providers/teralion → data-sync / run-planner / market-types
```

Domain crates 不反向依賴 CLI、provider transport 或 artifact serializer。

## Command capability boundary

| Command | Network | Mutates source/cache/run | Executes strategy |
| --- | --- | --- | --- |
| `config check`, `plan` | 否 | 否 | 僅 resolve/validate instance |
| `data sync` | 缺 source 時是 | source | 否 |
| `data verify` | 否 | 否 | 否 |
| `cache prepare` | 否 | cache | 否 |
| `replay` | 否 | 否 | 否 |
| `backtest` | 否 | run output | 是 |
| `run` | 視 plan | 可能 source/cache/run | 有 output 時是 |
| `inspect` | 否 | 否 | 否 |

Credential 只能進入 sync transport context，不得進入 config、identity、manifest、cache、log、strategy
context 或 run output。

## Atomicity 與 determinism

- source/cache 完整驗證後才 atomic publish；
- event transition 失敗不留下 advanced clock 或 partial state；
- callback error/panic 不提交該 callback 的 partial output；
- fill、cash、position、fees/taxes 與 P&L 必須 reconciliation；
- concurrency 不得改變 warnings、event order、callbacks、allocation 或 accounting result；
- run output 使用 staging + create-new publication，不覆寫既有 evidence。

Artifact lifecycle 與 identities 見[資料流程](data-flow.md)。
