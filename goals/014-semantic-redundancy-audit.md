# 014：production semantic redundancy audit 與第二階段架構瘦身

Status: done
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

☑ Phase A 完成 semantic candidate table。  
☑ Tier-1 redundancy 全部處理，或有具體不同語義證據。  
☑ Tier-2 ownership duplication 高信心項目已收斂。  
☑ runner / execution / accounting / config / provider 的 ownership 比 Goal-013 更單一。  
☑ 沒有新增 speculative framework。  
☑ production Rust LOC 有實質下降，或有充分 irreducibility evidence。  
☑ tests LOC 不因追求 total LOC 被刪。  
☑ Goal-006 capability matrix 全部維持 PASS。  
☑ Goal-013 auction evidence semantics 維持 PASS。  
☑ provider neutrality 維持 PASS。  
☑ deterministic / accounting reconciliation PASS。  
☑ fmt / workspace test / clippy PASS。  
☑ affected benchmark 無重大未解釋 regression。  
☑ largest-module before/after 已記錄。  
☑ retained large modules 有 responsibility explanation。  
☑ 一個 focused commit；依本次 user instruction push `codex-refactor`。

## 執行紀錄

• Baseline revision / working tree：`631c5ebac372b80069f74e36387f7e4b4a38e913`；`codex-refactor` 與 `origin/codex-refactor` 同步，開始 audit 時 working tree clean。  
• Executor / reasoning：Sol high  
• Production Rust LOC before：77 files / 3,132 blank / 268 comment / 34,730 code。  
• Total Rust LOC before：115 files / 3,709 blank / 278 comment / 43,456 code；production / total = 79.92%。  
• Largest production modules before：`execution-sim/scheduled.rs` 2,038；`osmium-runner/scheduled.rs` 1,736；`osmium-config/lib.rs` 1,672；`run-planner/config.rs` 1,389；`teralion/twse.rs` 1,248；`osmium-runner/lib.rs` 1,233；`teralion/tpex.rs` 1,228；`osmium-cli/command.rs` 1,123；`osmium-runner/artifacts.rs` 1,087；`execution-sim/lib.rs` 1,041。  
• Phase A candidate table：

|Tier|Candidate|Current owners|Same semantic?|Can one owner disappear?|Expected production LOC impact|Risk / decision|
|---|---|---|---|---|---:|---|
|1|TWSE／TPEx equity/warrant quote normalization pipeline|`teralion/twse.rs` + `teralion/tpex.rs`|是；normalized diff 只剩 market identity/annotation/version 與兩處不一致 validation，JSON envelope、book/deal parsing、grouping、event construction、error/report model 相同|是；共用一個 private concrete quote normalizer，market-specific modules 只保留 mapping identity 與薄 API boundary|約 -900～-1,100|medium；只以 `enum` 表達 TWSE/TPEx 差異，不建 generic normalizer。共同 owner 採較完整的 all-intermediate phase validation。|
|1|無 production caller 的 single-instrument backtest coordinator/artifact path|`osmium-runner::run_backtest*` + `publish_backtest`，另有 multi path|是；multi path 已涵蓋單商品 universe，CLI 只使用 multi/scheduled；single path只剩同 module test|是；刪除 single coordinator、completed DTO、error variant與 artifact encoder，保留 multi 作唯一一般回測 owner|約 -300～-450|low/medium；breaking public API，但 goals 已授權，產品能力與 CLI 不變。|
|1|multi/scheduled artifact staging、lineage、checksums、manifest publish|`publish_multi_backtest` + `publish_scheduled_multi_backtest`|是；相同 output preflight、staging、common lineage files、hash sidecars、atomic publish；payload/manifest extra fields不同|是；artifact module 內一個 concrete common publish helper|約 -120～-200|medium；不合併不同 binary schema，只合併 framing/publish lifecycle。|
|2|normal與scheduled runner callback/finalization plumbing|`osmium-runner/lib.rs` + `scheduled.rs`|部分相同；strategy lifecycle、feedback、ledger/final mark相同，但 scheduled 必須在 event 間插入 control queue／visibility／timer|部分；先移除 obsolete single path；不引入 scheduler abstraction|約 -300（由 single path）|medium/high；保留 multi vs scheduled coordinator，因 control-time ordering 是產品 invariant。|
|2|planner economics 到 runtime accounting model conversion|`run-planner` + `osmium-cli` + `execution-sim`|否；raw/frozen canonical config、composition-root conversion、mutable ledger model生命週期不同|否|0|保留；planner 不應依賴 execution crate，CLI 是正確 composition owner。|
|3|普通與 scheduled simulator lifecycle|`Simulator` + `ScheduledDepthSimulator`|否；subsequent-event causal fill 與 control-time activation/expiry/visible-depth consumption 不同|否|0|保留；共享 order DTO 不足以消除兩套 intrinsic state transition。|
|3|accounting model branches|`execution-sim/accounting.rs`|否；equity premium/notional cash、futures realized cash、options premium cash及 day-trade repricing各有獨立 invariant|否|0|保留；ledger/reconciliation 已是唯一 owner，runner只提交 fills與要求 final performance。|
|3|RunConfig／EffectiveConfig／ExecutionPlan facts|`osmium-config` + `run-planner`|否；YAML DTO、canonical effective identity、partition action plan是連續但不同 lifecycle|否|0|保留；未發現 CLI 重建 frozen semantic facts。|
|3|`AuctionObservation::from_parts` direction invariant|`market-types` constructor + `market-state` partial merge|不一致；merge 可產生 `purpose=Unknown + direction=Known`，constructor/canonical decoder卻拒絕|不涉及 owner 移除|小幅增加|low；最小修正為只拒絕 known non-VI purpose + known direction，保留逐欄 partial knowledge並加 round-trip regression。|
|4|大型 config/accounting/provider檔案單純拆檔|各原 owner|否|否|0|不做；只搬檔不減責任或控制流跳轉。|

• Tier-1 changes：TWSE／TPEx 的相同 JSON envelope、book/deal parsing、realtime grouping、auction/event construction、warning/error/report model 移到唯一 `equity_quote.rs` owner；`twse.rs`／`tpex.rs` 只保留 public re-export 與 mapping identity。TPEx 原本只比較第一筆 intermediate 的 trial phase，現在與共同 contract 一致，驗證所有 intermediates，並有 mixed-phase regression。刪除沒有 production caller 的 `CompletedBacktest`、`run_backtest*`、`BacktestError`、`publish_backtest` 與單商品 artifact encoders；一商品正式走 one-element multi universe。multi／scheduled artifact 共用 base lineage、strategy bytes/hash、sidecar hashing、manifest checksum 與 atomic staging publish，不合併不同 orders/fills/trace schema。  
• Tier-2 changes：一般 subsequent-event backtest coordinator 只剩 `run_multi_backtest`；runner 不再維護第二份 single-instrument callback／feedback／settlement／finalization 流程。Execution lifecycle 仍由 simulator 擁有，runner只 orchestration；ledger/reconciliation 仍由 `execution-sim` 唯一擁有。  
• Candidates intentionally retained：`Simulator` 與 `ScheduledDepthSimulator` 分別擁有 market-event causality 與 control-time activation/expiry/visible-depth consumption，不能合併；normal與scheduled runner保留兩條 coordinator，因後者必須在 replay events 間 deterministic interleave visibility、timer與control queue；planner canonical economics 與 runtime mutable ledger model生命週期不同，CLI conversion是正確 composition boundary；YAML `RunConfig`、canonical `EffectiveRunConfig`、partition `ExecutionPlan` 不是重複 DTO；equity/futures/options/day-trade accounting branches保留各自 cash與tax invariant。  
• New abstractions and deletion justification：新增一個 concrete `equity_quote` module及兩個 private two-variant enums，取代兩份完整 normalizer pipeline，讓 equity quote wire semantics只有一個 owner；原 `twse.rs + tpex.rs` 2,476 code LOC 變成 shared implementation + identity re-export約 1,435 LOC，淨減約 1,041。Artifact 新增 `base_files`、`insert_hashed`、`publish_artifacts` 三個 private functions，取代 multi／scheduled 重複 framing並吸收已刪 single publisher，`artifacts.rs` 1,087 -> 886（-201）；沒有新增 trait、generic、struct、framework或 speculative extension point。  
• Focused validation：PASS。`market-types` 43；Teralion provider 68；`execution-sim + osmium-runner + run-planner + osmium-config + teralion-provider + market-state + strategy-api` 合併 focused gate 234；runner 16、CLI 18。具名 TPEx all-intermediate phase regression、Periodic boundary、NoObservation/Unknown、VI continuity均通過。  
• Full validation：PASS。`cargo fmt --all -- --check`、`cargo test --workspace`（334 passed / 55 suites）、`cargo clippy --workspace --all-targets --all-features -- -D warnings`；fixture generator zero diff、compact verifier、bundle verifier、Python acceptance 8 tests、license verifier與 release builds通過。Fresh isolated offline `config check -> plan -> data verify -> cache prepare -> replay -> backtest -> run -> inspect` 通過；一般 smoke 2 orders / 0 fills，compiled custom strategy 1 order / 1 fill。  
• Determinism / accounting：兩次 replay均為 event checksum `b642c09945985960d20c2d1857874946a03e44868d74e04e2a8403fb6b3cf5e3`、final-state checksum `b52e79639f8762289172e42897f30773413e951d799f971c7488b7bdf44b1fe9`；兩份 backtest artifact directory diff為空。Shuffled replay、scheduled control suite（5 tests）、options premium/multiplier、equity+futures multi-ledger、TWSE equity + TAIFEX option reconciliation具名 tests通過。Core dependency/source scan確認 `market-types`、`market-state`、`replay-engine`、`strategy-api`、`execution-sim`、`run-planner`、`data-sync` 無 Teralion dependency或 symbol。  
• Performance：同一 `bench` profile、fixed synthetic input、baseline detached於 `631c5eb`。Cache prepare 0.901s / 55,465 records/s -> 0.810s / 61,718；scan median 0.031s -> 0.030s；cache replay median 0.092s -> 0.090s。Replay 1／8／32 streams median 76.22／80.05／86.24ms -> 75.65／81.50／86.76ms（約 -0.7%／+1.8%／+0.6%，noise range）。End-to-end backtest 117.33ms / 418,916 events/s -> 116.98ms / 420,158，兩側皆 16 fills / 147,458 output records且 checksum equivalent；無重大未解釋 regression。  
• Production Rust LOC after / delta：78 files / 3,036 blank / 261 comment / 33,295 code；code -1,435（-4.13%），production / total = 79.10%。  
• Total Rust LOC after / delta：116 files / 3,616 blank / 271 comment / 42,091 code；code -1,365（-3.14%）。Repository test files diff為 +82 / -9，沒有刪 tests換 LOC。  
• Largest production modules after：`execution-sim/scheduled.rs` 2,038；`osmium-runner/scheduled.rs` 1,736；`osmium-config/lib.rs` 1,672；`teralion/equity_quote.rs` 1,413；`run-planner/config.rs` 1,389；`osmium-cli/command.rs` 1,120；`execution-sim/lib.rs` 1,041；`osmium-runner/lib.rs` 1,034；`data-sync/cache.rs` 970；`osmium-runner/artifacts.rs` 886。  
• Retained large-module explanations：`execution-sim/scheduled.rs` 同時擁有 scheduled request、activation/expiry、visible book consumption、auction cross與execution trace transitions；`osmium-runner/scheduled.rs` 是 replay/control/visibility/timer/callback的deterministic merge loop；`osmium-config/lib.rs` 擁有完整 YAML schema解析與domain resolution；`equity_quote.rs` 集中兩市場共享wire parser、strict group validation、neutral event construction與diagnostics；`run-planner/config.rs` 擁有 frozen canonical config identity及跨universe economics/session validation；`osmium-cli/command.rs` 是 source/cache/replay/backtest composition root；`execution-sim/lib.rs` 擁有 subsequent-event causal simulator與multi-instrument dispatch；`osmium-runner/lib.rs` 擁有一般multi replay/strategy/execution/accounting coordinator。這些 >1,000 LOC modules各自仍含多個相依但不重複的domain transitions，純拆檔只會增加主流程跳轉。  
• Breaking changes：移除 public single-instrument runner/artifact API（`CompletedBacktest`、`run_backtest`、`run_backtest_stream`、`BacktestError`、`publish_backtest`）；TWSE／TPEx config constructor改為 shared `NormalizerConfig::{twse, twse_warrant, tpex, tpex_warrant}`，normalizer public names為 shared concrete type re-export；`ConfigError::WrongMarket` 攜帶 expected/actual。`AuctionObservation::from_parts` 現允許 partial `purpose=Unknown/NoObservation + direction=Known`，但仍拒絕 known non-VI purpose + known direction，使 canonical decoder能讀取 reducer可產生的partial state。Canonical bytes、event/state/artifact layout未改，故不 bump schema/version。  
• Remaining risks：TWSE／TPEx shared wire contract以目前fixture與normalized diff為證據；未來若交易所wire semantics分歧，必須在 `QuoteMarket` 明確分支而非假設相同。Repository fixtures仍非完整交易日，外部provider authorization/full-day release gate不在本goal。Feature branch push不會自動觸發GitHub Actions；本地full validation已PASS，PR或`workflow_dispatch`後才有CI結果。  
• Commit：`refactor: remove semantic production redundancy`（本 goal 單一 focused commit；依本次 user instruction push）。  
• Final status：done；Goal-014 acceptance全部成立，不開始Goal-015。

### Post-audit version correction

Goal-014 後的 correctness follow-up 發現版本 identity 未同步涵蓋兩項既有語意變更，已補正：TPEx quote mapping `8 -> 9`、TPEx warrant mapping `7 -> 8`；`market-types 10 -> 11`、event schema `8 -> 9`、canonical event `8 -> 9`。前者反映 TPEx intermediate auction/trial phase 現在要求所有 records 與 final record 一致，後三者反映 `purpose=Unknown/NoObservation` 搭配 `direction=Known` 已成為合法 canonical AuctionObservation value set。TWSE mapping semantics 未變，維持 quote `11`／warrant `7`。

同一 verified source／partition 的舊 TPEx mapping version 與新 version 現在產生不同 cache identity；既有 data-sync stale descriptor tests 也確認舊 market-types／event schema／canonical event descriptor 不會被視為 `Current`。因此舊 derived cache 及受版本影響的 event／run checksum 會 stale／改變並由 source rebuild，沒有加入 compatibility reader 或 migration。新增 `tpex_mapping_version_changes_partition_cache_identity` regression；canonical `purpose=Unknown` + `direction=Known(Down)` encode/decode round-trip 保持相同 DomainEvent，Opening／Closing／Periodic 搭配 Up/Down 仍拒絕。

Follow-up validation：`cargo fmt --all --check`、指定 crate tests、workspace tests（335 passed）、workspace clippy（`-D warnings`）、synthetic fixture／bundle／acceptance／license gates、current release CLI 與 fixture helper build、fresh offline `config check -> plan -> data verify -> cache prepare -> replay -> backtest -> run -> inspect`、compiled strategy smoke 與 clean-machine release smoke 全部 PASS。相同 final versions 與 inputs 重跑得到 event checksum `11b1a67c3722a905c7eda5ce4e2a91c30de556c2c36d0a398bd4636e6426f35a`、final-state checksum `523ee48c72e5887251ec7813f9e52952b9c960a8f4c8a422b8b62f2962394ad7`，backtest artifacts byte-identical；checksum 差異相對 Goal-014 舊值是版本／canonical identity 刻意改變的預期結果。

Goal-014 Status: done。
