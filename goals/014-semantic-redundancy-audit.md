# 014：production semantic redundancy audit 與第二階段架構瘦身

Status: pending
Depends on: 013-auction-evidence-correctness.md
Executor: Sol / high

## 目標

針對 Goal-000～013 完成後仍然偏大的 production code，做一次 semantic redundancy audit + evidence-driven simplification。

這次 KPI 不再看 total Rust LOC。

主要 KPI：

> **production Rust LOC、state ownership、control-flow duplication、閱讀主流程所需跳轉數量。**

本 goal 必須回答：

> 為什麼 final Osmium 的每個大型 production module 還需要這麼多 code？  
> 哪些是不可避免的 domain complexity，哪些其實只是 ownership / lifecycle / DTO / validation duplication？

────────

## 1. 執行模型

本 goal 由 Sol High 執行。

原因：

• 需要跨 execution / runner / accounting / config / planner 分析 semantic ownership。  
• 不能只靠文字相似度找 duplicate。  
• 需要判斷「真的不同語義」與「同一語義兩份實作」。  
• 重點是架構推理，不是大量外部工具輪詢。

### 建議工作方式

Sol High 分兩階段，但仍是一個 goal / 一個 commit：

```text
Phase A — Audit / decision
Phase B — Implementation / validation
```

Phase A 完成前不要修改 production architecture。

先產出 candidate table；只修改高信心 candidates。

完成後一個 focused commit。

建議 commit message：

```text
refactor: remove semantic production redundancy
```

不 push。

────────

## 2. Baseline：production LOC，而不是 total LOC

### 2.1 Production files

production Rust 定義為 workspace crate 下：

```text
crates/**/src/**/*.rs
```

排除：

```text
crates/**/tests/**
crates/**/benches/**
tools/**
target/**
```

使用本機 `~/cloc/cloc`。

產出 production Rust file list，再：

```sh
~/cloc/cloc \
  --list-file=/tmp/osmium-production-rust.txt \
  --include-lang=Rust
```

記：

```text
Production Rust LOC before
Total Rust LOC before
Production/Total ratio
```

Goal-013 完成後的 current branch 是本 goal baseline；不要拿 main 當 before。

### 2.2 Large-module inventory

至少重新檢查：

```text
execution-sim/accounting.rs
execution-sim/scheduled.rs
osmium-runner/scheduled.rs
osmium-config/lib.rs
run-planner/config.rs
osmium-runner/lib.rs
providers/teralion/twse.rs
providers/teralion/tpex.rs
osmium-cli/command.rs
osmium-runner/artifacts.rs
```

實作前重新用 cloc 排序，不假設上述順序仍相同。

────────

## 3. Audit 方法：不是找相似文字，而是找重複責任

每個候選記：

|Candidate|Current owners|Same semantic?|Can one owner disappear?|Expected production LOC impact|Risk|
|---------|--------------|--------------|------------------------|-----------------------------:|----|

### A. Execution lifecycle

檢查：

```text
execution-sim/src/lib.rs
execution-sim/src/scheduled.rs
osmium-runner/src/lib.rs
osmium-runner/src/scheduled.rs
```

問題：

• 普通與 scheduled order lifecycle 是否仍有兩份：  
  • acceptance  
  • restriction  
  • activation  
  • partial fill  
  • cancellation  
  • fill feedback  
  • order status  
• runner 是否仍在實作 execution policy，而非只 orchestration？  
• simulator 是否仍依賴 runner 才能完成本應屬 execution 的 transition？  
• market-event fill 與 control-time fill 哪些真正不同、哪些只是 trigger source 不同？  
• Simulator / ScheduledDepthSimulator 是否真的需要兩份 state model？

不要預設必須合併兩個 simulator。

若差異是 intrinsic：

```text
market-event causal fill
vs
control-time scheduled fill
```

就保留。

但 shared lifecycle / fill settlement / restriction 不應複製。

### B. Runner coordination

檢查：

```text
run_multi_backtest
run_scheduled_multi_backtest
strategy callback plumbing
feedback delivery
ledger settlement
final mark
session transition
universe validation
```

問題：

• replay / strategy / accounting orchestration 是否仍兩條大流程？  
• 能否共用一個 event/callback driver，而 scheduled mode 只插入 control queue？  
• 是否存在「普通 runner 一套，scheduled runner 再複製一套」？

若可共用，偏好：

```text
shared backtest coordinator
+ optional scheduled control source
```

但禁止建抽象 scheduler framework。

### C. Accounting

檢查：

```text
execution-sim/accounting.rs
runner ledger/final mark code
run-planner economics config
osmium-config economics conversion
```

問題：

• accounting model 是否仍有相同 formula 分支？  
• validation 是否多層重複？  
• fee / tax / currency / multiplier / quantity 是否重複 representation？  
• ledger finalization / reconciliation 是否有多個 owner？  
• artifact serialization DTO 是否反向影響 accounting model？

### D. Config / planner

檢查：

```text
osmium-config
run-planner
osmium-cli
```

問題：

• RunConfig / EffectiveRunConfig / ExecutionPlan 是否仍重複保存同一 facts？  
• 是否有相同 validation 在 YAML layer 與 planner layer 各一次？  
• CLI 是否仍重建 frozen plan facts？  
• getter / wrapper 是否只 forwarding？

### E. Provider TWSE / TPEx duplication

檢查：

```text
providers/teralion/src/twse.rs
providers/teralion/src/tpex.rs
```

兩者很大且高度相似。

必須分析：

• JSON envelope parsing 是否相同？  
• book/deal parsing 是否相同？  
• group validation 是否相同？  
• market annotation semantics 是否不同？  
• auction/session logic 是否可共用 private helper？  
• TWSE / TPEx 真正差異在哪？

只抽完全相同 wire semantics。

禁止建立：

```text
GenericExchangeNormalizer<T, U, V, W>
```

如果共用 3～5 個 private functions 就夠，直接用 function。

### F. Artifact encoding

檢查：

```text
osmium-runner/src/artifacts.rs
```

已經合併過 orders encoder，但仍檢查：

• fills  
• ledger  
• position  
• performance  
• checksum framing  
• single/multi/scheduled artifact writer

是否有 same-format duplication。

不同 magic/version/schema 不強行合併。

────────
────────

## 4. Audit outcome 分級

### Tier 1 — 明確 redundancy

特徵：

• 同一 input contract  
• 同一 validation  
• 同一 transition  
• 同一 output  
• 只有 caller/container 不同

=> 本 goal 必須處理。

### Tier 2 — ownership duplication

特徵：

• 同一 policy 在 runner / simulator / planner 重複  
• 一個 owner 可以明確接管

=> 優先處理。

### Tier 3 — suspicious but domain semantics 不同

=> 保留，寫出不同 invariant。

### Tier 4 — readability-only

例如：

• 大檔案  
• 長 match  
• 命名不漂亮

如果沒有實際 duplication，不要只為漂亮搬檔。

────────

## 5. Implementation 原則

優先刪除

1. duplicate policy
2. duplicate lifecycle transition
3. duplicate validation
4. duplicate DTO
5. pass-through wrapper
6. duplicate loop / plumbing
7. one-call-site abstraction
8. stale public export

### 新 abstraction 的門檻

每新增：

• trait  
• generic  
• struct  
• module  
• helper layer

都必須在執行紀錄回答：

```text
它刪掉了哪兩份以上的 existing code？
它讓哪個 owner 唯一化？
新增 LOC vs 刪除 LOC？
```

若答案只是：

> future extensibility

不准新增。

不追求最少檔案

可以拆大檔讓 ownership 清楚；但如果只是：

```text
2000 行一檔
→ 4 個 500 行檔
```

production LOC 不變、control flow 更跳，不算成功。

────────

## 6. 明確不做

• 不新增產品能力。  
• 不新增 order type。  
• 不新增 provider。  
• 不修改 Goal-013 market semantics，除非 refactor 暴露既有 bug。  
• 不建 plugin / DI / ECS / event bus。  
• 不用 macro 隱藏大量 domain logic。  
• 不靠刪 tests 降 LOC。  
• 不改成 codegen。  
• 不把不同 exchange / accounting semantics 硬泛化。  
• 不把 scheduled control time 與 replay time 合併。

────────

## 7. Production LOC 目標

本 goal 不設定硬性百分比，但必須有實質 production reduction，除非 Phase A audit 能以具體 evidence 證明目前 architecture 已接近 irreducible。

「實質」不是：

```text
-20 lines
```

在目前數萬 production LOC 的 codebase 上，如果仍存在明確 Tier-1 / Tier-2 candidates，只減幾十行不能標 done。

若 Phase A 找到多個高信心 candidates，目標應至少朝：

```text
數百 production LOC
```

的 reduction 推進。

這不是硬 KPI；若最後沒有達到，執行紀錄必須具體證明其餘大型模組為何屬 intrinsic complexity。

────────

## 8. Tests / correctness firewall

Goal-006 + Goal-013 是 functional firewall。

不得以 LOC 下降交換：

• provider neutrality  
• auction partial knowledge  
• deterministic replay  
• no-look-ahead  
• order causality  
• scheduled latency / control-time semantics  
• accounting exactness  
• reconciliation  
• run artifact lineage

### Focused regression

依修改範圍至少：

```text
execution-sim
osmium-runner
run-planner
osmium-config
teralion-provider
```

### Full gate

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

再跑：

• provider synthetic fixtures  
• fixture generator/checks  
• offline smoke  
• deterministic replay/backtest  
• representative accounting  
• affected benchmarks

────────

## 9. Performance

這次以 simplicity 為主，但不能產生重大 perf regression。

至少比較 Goal-012 / 013 後 baseline：

```text
cache/normalization
replay
end-to-end backtest
```

如果只修改 execution/runner，至少跑 end-to-end backtest + affected focused benchmark。

同 build profile / fixed synthetic input / reasonable repetitions。

────────

## 10. 最終報告必須回答

### A. Production LOC

```text
Production Rust LOC before:
Production Rust LOC after:
delta:
percentage:
```

### B. Total LOC

secondary：

```text
Total Rust LOC before:
Total Rust LOC after:
delta:
```

### C. Largest modules

列前 10：

```text
before
after
```

### D. Removed semantic owners

例如：

```text
order entry policy:
before = runner + simulator
after  = simulator only
```

### E. Retained large modules

對仍 >1000 LOC 的 production module，每個一句：

```text
why still large / what independent semantics remain
```

不是要求拆掉，只要求能解釋。

────────

## 11. 驗收

☐ Phase A 完成 semantic candidate table。  
☐ Tier-1 redundancy 全部處理，或有具體不同語義證據。  
☐ Tier-2 ownership duplication 高信心項目已收斂。  
☐ runner / execution / accounting / config / provider 的 ownership 比 Goal-013 更單一。  
☐ 沒有新增 speculative framework。  
☐ production Rust LOC 有實質下降，或有充分 irreducibility evidence。  
☐ tests LOC 不因追求 total LOC 被刪。  
☐ Goal-006 capability matrix 全部維持 PASS。  
☐ Goal-013 auction evidence semantics 維持 PASS。  
☐ provider neutrality 維持 PASS。  
☐ deterministic / accounting reconciliation PASS。  
☐ fmt / workspace test / clippy PASS。  
☐ affected benchmark 無重大未解釋 regression。  
☐ largest-module before/after 已記錄。  
☐ retained large modules 有 responsibility explanation。  
☐ 一個 focused commit，未 push。

## 執行紀錄

• Baseline revision / working tree：  
• Executor / reasoning：Sol high  
• Production Rust LOC before：  
• Total Rust LOC before：  
• Largest production modules before：  
• Phase A candidate table：  
• Tier-1 changes：  
• Tier-2 changes：  
• Candidates intentionally retained：  
• New abstractions and deletion justification：  
• Focused validation：  
• Full validation：  
• Determinism / accounting：  
• Performance：  
• Production Rust LOC after / delta：  
• Total Rust LOC after / delta：  
• Largest production modules after：  
• Retained large-module explanations：  
• Breaking changes：  
• Remaining risks：  
• Commit：  
• Final status：
