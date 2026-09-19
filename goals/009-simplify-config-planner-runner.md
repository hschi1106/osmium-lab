# 009：簡化 config → planner → runner 的資料流

Status: complete
Depends on: 008-simplify-accounting-economics.md

## 目標

讓主要 application flow 可以直接理解：

```text
YAML
-> parse
-> semantic validation / materialization
-> ExecutionPlan
-> run
```

移除中間只 copying/forwarding/duplicate validating 的 DTO、wrapper 與 orchestration。

不預設合併 crate；先用 call/data flow 證明問題。

## Audit

```text
osmium-config
run-planner
osmium-cli command orchestration
osmium-runner public entrypoints
strategy registry/bootstrap
session/instrument materialization
source/cache state planning
```

特別核對：

```text
RunConfig
EffectiveRunConfig
PlanBundle
ExecutionPlan
InstrumentSelection
InstrumentContractConfig
SessionPlan
SimulationConfig
```

對每個回答：

- serialization boundary？
- validated immutable domain config？
- plan result？
- runtime object container？
- 是否重複持有另一 struct 已有資訊？

## 目標 ownership

偏好：

```text
osmium-config = YAML/application parsing + strategy bootstrap glue
run-planner   = provider-neutral validated planning values
osmium-runner = execute frozen plan
osmium-cli    = composition / IO / presentation
```

不要讓 planner 下載 provider、config 解 wire、runner 重 parse YAML、CLI 重做 planner semantic validation。

Goal-003 provider resolver 邊界必須保持。

## 優先刪除

- pass-through wrappers/getters 沒 contract value
- duplicate universe/session validation
- duplicate config copies
- duplicate plan/replay binding construction
- CLI/config 各一份 source/cache check
- 舊 API adapter

如果 wrapper 是 licensing/public strategy API 或 versioned artifact boundary，保留並說明。

## 不做

- 不合 giant crate/file。
- 不為乾淨全面 rename。
- 不新增 DI/builder framework。
- 不把 provider dispatch 拉回 config/planner。
- 不改 execution/accounting semantics。

## Regression

- config check
- plan
- data sync plan behavior
- cache prepare
- replay
- backtest
- run
- inspect
- compiled strategy registry injection
- multi-instrument/session plan
- deterministic config/plan identity

## LOC

記 total、osmium-config、run-planner、osmium-cli、osmium-runner。

## 驗收

- [x] YAML -> plan -> run 可順著追。
- [x] duplicate validation/DTO copy 減少。
- [x] provider neutrality 未倒退。
- [x] runner 只執行 frozen plan。
- [x] CLI 只做 composition/IO/presentation。
- [x] 無新 orchestration framework。
- [x] Goal-006 matrix 回歸通過。
- [x] fmt/test/clippy 通過。
- [x] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：`968fcab`；Goal 009 開始前 worktree clean。
- Rust LOC before：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,658 blank / 273 comment / 42,461 code。範圍分項：`osmium-config` 1 / 130 / 1 / 1,702，`run-planner` 9 / 366 / 6 / 3,594，`osmium-cli` 3 / 102 / 10 / 1,923，`osmium-runner` 6 / 238 / 0 / 4,767。
- Data-flow duplication：`osmium-config::plan` 原本同時建立 `ExecutionPlan` 與未被消費的平行 `PlanBundle.session_plans`；CLI replay core、schedule、source sync、cache prepare 與 backtest 又以 `RunConfig` 重新推導 session plan、instrument class、contract shape。`execute run` 也重複 load config、建立 registry 與 plan，讓 source/cache mutation 後的 re-plan 與 final execution 不易沿同一條 flow 閱讀。
- Removed / retained boundaries：`PlannedPartition` 現在攜帶並驗證與 `SourcePartitionKey` identity 對應的 `SessionPlan`，`PlanBundle` 僅保留 `ExecutionPlan` 與 optional `ReplayPlan`；`EffectiveRunConfig::contract_for` 成為 validated contract lookup。CLI 的 replay/schedule/sync/cache/backtest 改消費 frozen plan；`execute run` 只在 source/cache mutation 後重新 plan，不再重載 YAML／strategy registry。`RunConfig::partition_keys()` 保留給不需要 provider mapping 或 cache state 的 read-only `data verify`，`InstrumentSelection` 保留作 YAML/application parsing 與 strategy bootstrap input；Goal-003 的 provider mapping resolver 仍在 CLI composition root。
- Validation：`rtk cargo fmt --all -- --check` 通過；`rtk cargo clippy --workspace --all-targets --all-features -- -D warnings` 通過；`rtk cargo test --workspace` = 328 passed（55 suites）；fixture generator、fixture diff、compact/bundle verifier、license verifier 與 Python acceptance tests（8 passed）通過；release `osmium` 與 fixture-data helper build 通過。fresh temporary-root smoke 通過 `config check`、`plan`、`data verify`、`cache prepare`（reused）、`replay`、`backtest`、`run`（output 與 replay-only）、`inspect` 及 compiled strategy smoke（1 order / 1 fill）。`git diff --check` 通過。
- Rust LOC after / delta：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,666 blank / 273 comment / 42,580 code；相對 baseline code `+119`。分項：`osmium-config` 1 / 128 / 1 / 1,672（code `-30`），`run-planner` 9 / 371 / 6 / 3,714（`+120`，含 frozen session regression tests），`osmium-cli` 3 / 107 / 10 / 1,952（`+29`），`osmium-runner` 維持 6 / 238 / 0 / 4,767。
- Breaking changes：移除 `RunConfig::selection_for`、`instrument_class_for`、`session_plan_for` 與 `PlanBundle.session_plans`；workspace 內 CLI 與 acceptance fixture helper 已遷移至 frozen plan API。`PlannedPartition::classify`／`coverage_unavailable` 現在要求 matching `SessionPlan` 並回傳 validation result。
- 剩餘風險：`osmium-runner` 維持 provider-neutral generic entrypoints，仍由 CLI 將 `ReplayPlan`、session schedule、simulator 與 ledger 組合後呼叫；`ExecutionPlan` identity 對應 session identity，但不把完整 session windows 另編碼進 canonical bytes。source verify 仍需由 config layer 產生 requested keys，因該 command 不應為驗證而觸發 provider mapping/cache inspection。
- 下一步：010-remove-legacy-compatibility.md
