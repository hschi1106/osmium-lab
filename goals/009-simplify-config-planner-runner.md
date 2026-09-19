# 009：簡化 config → planner → runner 的資料流

Status: pending
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

- [ ] YAML -> plan -> run 可順著追。
- [ ] duplicate validation/DTO copy 減少。
- [ ] provider neutrality 未倒退。
- [ ] runner 只執行 frozen plan。
- [ ] CLI 只做 composition/IO/presentation。
- [ ] 無新 orchestration framework。
- [ ] Goal-006 matrix 回歸通過。
- [ ] fmt/test/clippy 通過。
- [ ] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- Data-flow duplication：
- Removed / retained boundaries：
- Validation：
- Rust LOC after / delta：
- Breaking changes：
- 剩餘風險：
- 下一步：010-remove-legacy-compatibility.md
