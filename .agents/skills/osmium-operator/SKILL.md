---
name: osmium-operator
description: >
  Use for operating or troubleshooting an existing Osmium build: config,
  data, cache, replay, backtest, artifacts, and compiled strategies. Not for
  Rust or architecture changes.
---

# Osmium Operator

操作或說明既有 Osmium build：config／plan／sync／verify／cache／replay／backtest／inspect、artifacts
與 compiled strategies。修改 Rust、architecture 或 domain semantics 時轉交 `osmium-engineering`；
選擇／設定 existing strategy 是 operator，實作／修改 strategy code 是 engineering。

## 文件 routing

從 `docs/user-guide.md` 開始。操作細節查 `docs/config-reference.md`、`docs/operations/cli.md` 與
`docs/operations/local-data.md`；只有問題需要時才讀 `docs/architecture/data-flow.md`、
`docs/architecture/execution-model.md` 或相關 `docs/interfaces/**`。版本值以 current docs／code／CLI
為準，不硬編在 skill。

## Workflow

```text
check    → validate config
plan     → freeze/check required actions
sync     → obtain source; may need network
verify   → verify source read-only
cache    → build/reuse derived event cache
replay   → deterministic state/checksums; no strategy
backtest → strategy + execution + accounting
inspect  → verify/summarize an existing run
```

`run` 是 orchestration；是否需要 network 由 plan 決定。定位副作用或失敗階段時使用分步流程。

## Artifacts

```text
verified source = immutable reusable fact
cache           = derived/rebuildable artifact
run output      = immutable execution evidence
```

不要手改 artifacts、覆寫 successful run、把 credentials 放入 config，或用刪除 verified source
處理 cache mismatch。

## Evidence 與 troubleshooting

不要編造 exchange facts。Fill／no-fill 只能根據 modeled orders、eligible evidence、latency、auction
state、quantity policy 與 slippage 解釋；queue position 與 hidden liquidity 保持 unknown。清楚區分
Osmium modeled result 與 real exchange unknown。

優先 read-only diagnosis：`config check`／`plan`／`data verify`／`inspect`；定位後才建議有副作用的
sync／cache／backtest。收集 actual command、error category、config、plan action 與 artifact identity，
並遮蔽 credentials 與受限制 raw data。
