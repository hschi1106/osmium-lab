---
name: osmium-engineering
description: >
  Use for changing, reviewing, debugging, or testing Osmium code or
  architecture, including domain, provider, replay, strategy APIs, execution,
  accounting, versions, and tests. Not for ordinary operation of an existing
  build.
---

# Osmium Engineering

Canonical docs 擁有 domain truth；code 與 tests 是 executable truth。本 skill 只提供 routing、
invariants 與 workflow。

## Ownership 與文件

```text
provider      → wire → DomainEvent
data-sync     → source/cache lifecycle
planner       → validated, frozen execution facts
replay        → ordering/playback
market-state  → cross-event state
strategy      → decisions
execution-sim → order/fill/accounting
runner        → orchestration
CLI           → composition/user contract
```

先定位 semantic owner；不要把 policy 移到 caller。只讀 owner 相關文件，不確定時從
`docs/README.md` 導航：

- market／event／reducer／auction → `docs/architecture/replay-model.md`
- execution／accounting／strategy → `docs/architecture/execution-model.md`
- provider／source／cache → `docs/architecture/data-flow.md` + `docs/interfaces/**`
- config／planner → `docs/config-reference.md`
- CLI → `docs/operations/cli.md`
- versions／evidence → `docs/traceability.yaml`

## High-risk invariants

```text
NoObservation != Unknown != Known(false)
NoObservation → preserve same-round evidence
Known(v)      → update
Unknown       → invalidate
```

不得猜測 unknown evidence。Provider wire 與 domain semantics 必須分離。Auction event matching 不等於
post-event state；修改 auction／reducer 前必讀 `replay-model.md`。Known Periodic uncross result 保留
round evidence；下一 round 以 `delayed=false` 開始，直到 explicit delay trigger。維持
`match_time` 唯一 replay clock、complete snapshot replacement、no queue reconstruction、post-event
read-only strategy state 與 explicit-universe selective streams。

## Version／identity

同一 version identity 不得代表兩套 semantics。每次 semantic change 檢查：

```text
wire → event          → mapping version?
event legality/codec  → market/event/canonical versions?
state transition      → state/reducer/canonical-state versions?
strategy-visible rule → strategy version?
execution/fill        → model version?
artifact meaning      → manifest/artifact version?
ordering              → ordering version?
```

版本取自 current code／docs／CLI。正確 bump 可使 derived cache stale；除非使用者要求，不加 legacy
compatibility。

## Scope 與 workflow

優先使用 concrete function／enum／struct。不要為 hypothetical future 新增 framework、trait forest、
generic normalizer、compatibility layer 或 one-call-site wrapper。Abstraction 必須移除真實 duplicated
semantics 或釐清 ownership。

```text
locate owner → read contract → inspect code/tests → smallest coherent change
→ regression → identity check → focused/full validation
```

只有 contract 改變才更新 canonical docs。普通 config／sync／replay／backtest／inspect 問題改用
`osmium-operator`。

## Validation

```sh
./.agents/skills/osmium-engineering/scripts/validate.sh focused <area>
./.agents/skills/osmium-engineering/scripts/validate.sh full
```

其他 gate 見 `docs/operations/validation.md`；只宣稱實際成功的 command。
