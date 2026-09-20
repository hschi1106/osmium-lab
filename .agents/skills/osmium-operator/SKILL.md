---
name: osmium-operator
description: >
  Use when operating or explaining an existing Osmium build: creating
  configuration, checking or planning runs, syncing and verifying data,
  preparing caches, replaying markets, running backtests, inspecting
  artifacts, selecting compiled strategies, explaining checksum/source/cache
  lifecycles, or troubleshooting user-facing workflows. Do not use for Rust
  implementation, new strategy code, architecture changes, or refactoring.
---

# Osmium Operator

正確地使用與說明既有 Osmium build。Canonical product knowledge 仍在 `docs/**`；本 skill 不鼓勵
為普通操作問題掃描或修改 Rust code。

## Boundary

本 skill 可以修改 YAML config、執行 CLI、讀取 run artifacts、解釋 errors、建議 command sequence，
以及選擇既有 compiled strategy。

遇到修改 Rust、實作新 strategy、改 `Strategy` trait／factory／registration、加入 provider、改
architecture、MarketState、cache format、execution／fill／accounting semantics 時，轉交
`osmium-engineering`。Troubleshooting 本身不等於授權 refactor。

Strategy boundary：operator 只選擇既有 compiled strategy、設定 `id`／`version`／parameters、執行並
檢視 output；新增或改寫 strategy implementation 屬於 engineering。

## Canonical docs routing

一般操作先讀：

- `README.md`
- `docs/user-guide.md`
- `docs/config-reference.md`
- `docs/operations/cli.md`
- `docs/operations/local-data.md`

只有需要 deeper explanation 才讀 `docs/architecture/data-flow.md` 或
`docs/architecture/execution-model.md`；市場／provider 問題才讀相關 `docs/interfaces/teralion.md`、
`twse.md`、`tpex.md` 或 `taifex.md`。若 docs 與 current command output 不一致，可檢查 current CLI／code，
但不要因此自行改 code。版本值一律從 current docs、code 或 `osmium version` 取得，不硬編在 skill。

## Workflow model

```text
config check
↓
plan
↓
data sync
↓
data verify
↓
cache prepare
↓
replay
↓
backtest
↓
inspect
```

- `config check`：離線驗證 user contract，不建立 plan。
- `plan`：凍結 execution facts 並檢查 local source/cache state，不下載。
- `data sync`：取得或 resume immutable provider source；可能需要 network 與 credential。
- `data verify`：離線、只讀地驗證 local source integrity。
- `cache prepare`：由 verified source 離線建立 neutral、versioned `DomainEvent` cache；cache 可重建。
- `replay`：deterministic events → MarketState → checksums；不執行 strategy。
- `backtest`：replay + strategy + execution + accounting → immutable run artifacts。
- `inspect`：驗證並摘要既有 run artifacts，不重跑 strategy。

`run` 是 convenience orchestration；是否需 network 由 plan 決定。需要定位副作用或錯誤階段時，使用
上方分步流程。

## Artifact mental model

```text
verified source = immutable reusable historical fact
replay cache    = derived, deletable, rebuildable artifact
run output      = immutable execution evidence
```

不要手改 cache／manifest／`current.yaml`，不要覆寫成功 run directory，不要把 API key 放進 config，
也不要靠刪除 verified source 解決 cache version mismatch。只對診斷已精確定位的 derived cache 採取
安全重建措施。

## Evidence 與 troubleshooting

Unknown 保持 unknown；不要替回測結果編造 exchange facts。解釋 fill／no-fill 只能使用：

- order 與選定 execution model；
- eligible event、visible book／trade evidence；
- latency、auction state、quantity policy 與 slippage。

不得歸因於 Osmium 未重建的 queue position、hidden liquidity 或「實際市場沒排到」。清楚區分
「Osmium modeled result」與「real exchange unknown」。

排查時先取得實際 command、exit category／JSON error、config、plan action 與相關 artifact identity；
credential 與受限制 raw data 必須遮蔽。優先依文件建議重跑 read-only `config check`、`plan`、
`data verify` 或 `inspect`，再決定是否需要有寫入副作用的 sync/cache/backtest 步驟。
