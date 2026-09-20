---
name: osmium-engineering
description: >
  Use when modifying, reviewing, debugging, testing, refactoring, or
  extending the osmium-lab codebase, including Rust code, architecture,
  providers, market normalization, DomainEvent, MarketState, replay,
  strategy APIs, execution, accounting, caches, version identities,
  performance, tests, and new features. Do not use for ordinary operation
  of an existing Osmium build unless code changes or architectural analysis
  are required.
---

# Osmium Engineering

安全、正確地修改 Osmium。既有 `docs/**` 是 product/domain knowledge 的 canonical source，
code 與 tests 是 executable truth；本 skill 只提供 routing、invariants 與工作流程。

## 先定位 semantic owner

```text
Source
  ↓
DomainEvent
  ↓
MarketState
  ↓
Strategy
  ↓
Execution
  ↓
Accounting
  ↓
Artifacts
```

修改前先回答「這個 semantic owner 是誰？」：

- Provider adapter：raw provider wire → neutral `DomainEvent`。
- `data-sync`：verified source 與 replay cache lifecycle。
- Planner：validated、frozen execution facts。
- Replay engine：deterministic streams、ordering 與 playback。
- MarketState reducer：cross-event market state 與 auction transitions。
- Strategy：decision logic only。
- `execution-sim`：order lifecycle 與 fill semantics；其中 accounting 擁有 fill → ledger／positions／P&L。
- Runner：orchestration only。
- CLI：composition root 與 user-facing command boundary。

Fee formula 不放 runner；Teralion wire bit 不放 replay；auction transition 不放 provider／runner；
fill eligibility 不放 strategy／runner。不要把 policy 移到方便呼叫的 caller。

## Canonical docs routing

任何工程任務先讀 `AGENTS.md`、`docs/product-requirements.md`、
`docs/architecture/overview.md`，再只讀與任務相關的 canonical docs：

- market、event、reducer、auction：`docs/architecture/replay-model.md`。
- execution、accounting、strategy lifecycle：`docs/architecture/execution-model.md`。
- provider、source、cache：`docs/architecture/data-flow.md` 與相關 `docs/interfaces/**`。
- config、planner：`docs/config-reference.md`。
- CLI、composition：`docs/operations/cli.md`。
- version 與 evidence mapping：`docs/traceability.yaml`。

不要為小修改掃完整 repository，也不要在 skill 內建立第二份 domain documentation。

## Correctness invariants

Evidence 必須保持三態差異：

```text
NoObservation != Unknown != false

NoObservation → retain prior evidence
Known(v)      → update
Unknown       → invalidate old knowledge
```

禁止把 unknown 降級成 `false`、`Continuous` 或猜測的 provider state。

Auction 必須分開 matching mechanism、auction purpose 與 instrument/day context；
`AuctionUncross` 的 current-event matching semantics 與 post-event `MarketState` 也必須分開：

- Periodic round N 的 result 保留本輪 `purpose=Periodic, delayed=true`；post-state round N+1 為
  `purpose=Periodic, delayed=false`，只有後續 explicit delay trigger 才重新變 `true`。
- Opening：current event 是 `CallAuction`，post-state 是 `Continuous`。
- Closing：current event 是 `CallAuction`，post-state 是 `Closed`。
- Known volatility-interruption uncross：post-state 是 `Continuous`。
- Unknown purpose：不得猜測 post-state。

同時維持 provider wire／domain separation、`match_time` 唯一 replay clock、完整 snapshot replacement、
no queue reconstruction、post-event read-only strategy state、explicit universe selective streams，及
verified source 可重用／cache 可離線重建等 product invariants。

## Version 與 identity

任何 semantic change 都要檢查 identity impact；同一 version identity 不得代表兩套不同 semantics：

- raw wire → `DomainEvent` 行為：provider mapping version？
- 合法 event value／decoding：`MARKET_TYPES_VERSION`、`EVENT_SCHEMA_VERSION`、
  `CANONICAL_EVENT_VERSION`？
- MarketState transition／canonical semantics：`MARKET_STATE_VERSION`、
  `STATE_REDUCER_VERSION`、canonical state versions？
- strategy-visible contract／policy：strategy API／rule version？
- execution／fill semantics：execution／fill model version？
- artifact schema／layout／meaning：manifest／artifact version？
- ordering：ordering version？

版本值以 current code、docs 或 CLI 為準，不在 skill 內硬編。正確 bump 使舊 cache stale 並由
verified source rebuild 是預期結果，不要偷偷增加 compatibility layer。

## 避免 overengineering

不要只為 future extensibility 新增 generic FSM、event bus、DI/plugin/scheduler framework、trait forest、
generic exchange normalizer、沒有第二個 implementation 的 registry、one-call-site wrapper、
compatibility layer，或隱藏 domain semantics 的 macro。

提出 abstraction 前回答：它刪除哪兩份以上既有 semantic duplication？哪個 owner 因而唯一？為何
function／enum／concrete struct 不足？added LOC 與 removed LOC 如何？control-flow jumps 是下降或上升？

## 修改流程

非 trivial change 依序執行：

1. Identify semantic owner。
2. 讀相關 canonical docs。
3. 檢查 current code 與 regression tests。
4. 說明 current invariant。
5. 做 smallest coherent change。
6. 新增或更新 regression。
7. 檢查 version／identity implications。
8. 跑 focused validation。
9. 跑 full relevant validation。
10. 只有 contract 改變時才更新 canonical docs。

Breaking change 可以接受，但必須是有意識的；除非使用者明確要求，不保留 legacy compatibility。

## Validation

先執行與 owner 對應的檢查：

```sh
./.agents/skills/osmium-engineering/scripts/validate.sh focused <area>
```

`<area>` 可為 `market`、`provider`、`replay`、`strategy`、`execution`、`runner` 或 `config`。
完成前執行：

```sh
./.agents/skills/osmium-engineering/scripts/validate.sh full
```

若變更範圍需要額外 smoke、docs、benchmark 或 release gate，依
`docs/operations/validation.md` 加跑；實際成功的 command 才能宣稱已驗證。

普通 config／sync／replay／backtest／inspect 使用問題應改用 `osmium-operator`。
