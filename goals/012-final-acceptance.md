# 012：最終 correctness、provider neutrality、performance 與 LOC 驗收

Status: complete
Depends on: 011-final-redundancy-cleanup.md

## 目標

不再主動重構。

對 Goal-000 baseline 與最終 repo 做完整驗收：

1. provider 是否真的解耦？
2. product-scope backtesting capabilities 是否完整？
3. deterministic/accounting 是否正確？
4. 架構是否更簡單？
5. Rust LOC/crate/dependency 實際如何變？
6. performance 是否有不可接受退化？

若發現 regression，只做最小修正並重跑 gate。

## Revision/tree

```sh
git rev-parse HEAD
git status --short
```

列 long-run modifications；本 goal 只建立一個 focused commit，不 push。

## Final LOC

用 README dirty-working-tree cloc。

另可執行：

```sh
~/cloc/cloc --vcs=git --include-lang=Rust .
```

但若有未 track 新檔，以 working-tree file-list count 為 final authority。

輸出：

```text
Goal-000 baseline Rust LOC
Final Rust LOC
delta
percentage delta

baseline workspace crate count
final workspace crate count
delta
```

再列主要 area before/after。

## Provider neutrality gate

確認 core：

```text
market-types
market-state
replay-engine
strategy-api
execution-sim
run-planner
generic data-sync storage/cache
```

不依賴 Teralion。

用 Cargo dependency graph、source references、arbitrary non-Teralion SourceId core tests 驗。

Teralion-specific 只應在 provider crate、provider fixtures/tests、composition-root selection、provider docs。

## Capability gate

逐項重跑 Goal-006 matrix：

```text
capability | production owner | test/evidence | result
```

所有 in-scope 必須 PASS；out-of-scope 列出即可。

## Full validation

至少：

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

再跑 final repo 對應的：

- fixture generation/checksum
- provider synthetic conformance
- offline data verify
- cache prepare
- replay
- backtest
- inspect
- release/smoke relevant gates

Goal-001 已刪 TUI 後，final gate 不得要求 display。

## Determinism

固定 repo-owned input 至少重跑兩次：

- neutral replay
- representative backtest
- scheduled backtest

確認 event sequence/checksum、final state、strategy output、orders/fills、accounting result 相同。

## Accounting reconciliation

對 representative equity、futures、options、multi-instrument 確認：

```text
cash
position
fees
tax
cash charges
realized P&L
unrealized/marking
performance
reconciliation
```

## Performance

用 Goal-000 可比較的相同 benchmark command/build profile/input 與 final 比較。

另可用 Goal-002 synthetic workload。

至少報：

- cache/normalization
- replay
- end-to-end backtest

做合理重複，不用單次 wall time 下結論。

若明顯退化，區分 correctness cost / accidental regression / noise；只修 accidental regression。

## Final architecture report

簡短說明：

```text
provider
-> verified source
-> neutral DomainEvent
-> cache
-> replay/MarketState
-> strategy
-> execution
-> accounting
-> artifacts
```

列 removed features/crates/legacy paths、consolidated execution/accounting、retained boundaries、breaking changes、provider extension point。

不要寫長篇設計史。

## 最終成功條件

- [x] provider neutrality PASS。
- [x] Goal-006 所有 in-scope capability PASS。
- [x] fmt/tests/clippy PASS。
- [x] fixture/provider/offline smoke PASS。
- [x] deterministic rerun PASS。
- [x] accounting reconciliation PASS。
- [x] performance 無未解釋重大 regression。
- [x] final cloc/crate count 與 baseline 比較完成。
- [x] final docs/PRD/traceability 與 code 一致。
- [x] 無 legacy dual path/TUI/hidden Teralion core dependency。
- [x] 無因 cleanup 刪必要 correctness evidence。
- [x] working tree 只包含本 goal 的唯一 commit，且未 push。

## 執行紀錄

- 起始 revision / working tree：`b44e778`；Goal 011 完成後 working tree clean。最終 revision 是本 goal 唯一的 focused commit；未 push，commit 後 working tree clean。
- 本 goal 只做 final acceptance 的最小修正：`crates/osmium-runner/benches/full_multi_backtest.rs` 為 neutral `DomainEvent` 補上明確 `MarketSignal::Continuous`，恢復 benchmark 的 16 fills；`run-planner`、`market-types` 的 core tests 將測試資料改為 arbitrary `synthetic-source`／offset 命名，移除 core source scan 的 Teralion 字面依賴。沒有修改 production execution、accounting 或 provider composition path。
- Baseline Rust LOC：Goal 000 的 `~/cloc/cloc --vcs=git --include-lang=Rust .` 為 `110 files / 3,669 blank / 272 comment / 42,761 code`。
- Final Rust LOC：`~/cloc/cloc --vcs=git --include-lang=Rust .` 為 `115 files / 3,674 blank / 271 comment / 42,834 code`；delta 為 `+5 files / +5 blank / -1 comment / +73 code`，code percentage delta 約 `+0.17%`。workspace crates 的 dirty-working-tree cloc 為 `114 files / 3,657 blank / 271 comment / 42,498 code`；raw `cloc .` 會包含 ignored `target/` build output，因此不作 root comparison authority。
- Baseline/final crate count：`14 -> 12`（`-2`）。主要 area code（baseline -> final）：`execution-sim 5,746 -> 5,738`、`data-sync 4,952 -> 2,517`、`normalizer/* 5,038 -> removed and owned by provider pipeline`、`providers/teralion final 7,716`、`osmium-runner 4,529 -> 4,754`、`strategy-api 4,796 -> 4,574`、`market-types 3,928 -> 4,193`、`run-planner 3,500 -> 3,714`、`osmium-cli 3,058 -> 1,951`、`market-state 2,468 -> 2,888`、`replay-engine 2,370 -> 2,405`、`osmium-config 1,675 -> 1,672`。`normalizer/*` 與 `providers/teralion` 不作一對一 code delta，因 Goal 003 已將 source pipeline 移入 provider crate。
- Provider neutrality：`cargo tree -e normal` 對 `market-types`、`market-state`、`replay-engine`、`strategy-api`、`execution-sim`、`run-planner`、`data-sync` 均無 Teralion reference；core source scan 對 `teralion`、三個 normalizer 名稱為零結果。`provider_neutral_source_ids_are_storage_safe_and_part_of_identity` 通過。Teralion 只保留在 provider crate、provider fixtures/tests、`osmium-cli` composition root 與 provider docs。
- Capability matrix：Goal 006 的所有 in-scope 均 PASS。provider-neutral source、cache reuse/rebuild、deterministic multi-stream、multi-instrument/multi-day、firm/indicative isolation、visibility/no-look-ahead；TWSE/TPEx equity/warrant 與 TAIFEX future/option 六類 model scope；continuous、opening/closing、delayed、periodic/disposal、volatility interruption、Unknown/degraded；strategy lifecycle/timer/scheduled/deterministic/callback transaction；market/limit ROD、partial/displayed depth、latency、slippage、scheduled activation/expiry、auction/cancel/stability；cash/position/P&L/marking、fee/tax/day-trade、cash charge、quantity/multiplier/currency、EquityV1/FuturesV1/OptionsV1、reconciliation/per-fill cost；immutable artifacts、lineage、checksums 與 inspect，均有 PRD/traceability owner 與 test/fixture evidence。IOC/FOK、即時交易、零股／盤後／鉅額、完整 exchange matching、queue position、hidden liquidity、exercise/assignment 仍明確 out-of-scope。
- Full validation：`rtk cargo fmt --all -- --check`、`rtk cargo clippy --workspace --all-targets --all-features -- -D warnings`、`rtk cargo test --workspace` 均通過；workspace 為 `328 passed / 55 suites`，`rtk cargo test -p teralion-provider` 為 `67 passed / 11 suites`。fixture generator + `git diff --exit-code -- fixtures`、compact verifier、smoke bundle verifier、license verifier、Python acceptance `8 tests` 均通過。release `osmium` build 通過；fresh offline CLI smoke 的 config check、plan、data verify、cache prepare reuse、replay、backtest、run（有 output 與 replay-only）、inspect，以及 custom strategy backtest/inspect 均通過；一般 smoke 為 `2 orders / 0 fills`，custom strategy 為 `1 order / 1 fill`。
- Determinism：兩次 replay 固定得到 `event_checksum=a796e6ab3494f338254ef7312e0a30e25964288f2cc3a62d906456f9022a7239`、`final_state_checksum=b03ad9a67e8f0590ce572038913afb4806ea1a12e0441f1e1736488d5028733b`。兩次 representative backtest 的 directory diff 為空，event/final-state/orders/fills/ledger checksum 分別一致；shuffled replay、strategy output、multi-market runner 與 scheduled backtest focused tests 通過，scheduled suite 連續兩次各 `5 passed`。
- Accounting reconciliation：`options_v1_moves_premium_cash_with_contract_multiplier`、`multi_ledger_reconciles_equity_and_futures_cash_separately`、futures latency/isolation、以及 TWSE equity + TAIFEX option multi-market E2E 均通過。代表性 multi-market case 為 `6 events / 4 orders / 4 fills`、`EquityV1 + OptionsV1`、realized P&L `-102`、shared cash `999898`；fees、tax、cash charges、marking 與 per-fill lineage 由 workspace accounting regression suite 覆蓋。
- Performance baseline vs final（相同 locked bench profile 與 fixed synthetic input）：cache benchmark 的舊 package command `data-sync/verified_cache_pipeline` 在 Goal 003 provider decoupling 後已不存在，故以現 owner `teralion-provider` 執行；cache prepare `1.049s / 47,685 records/s -> 0.873s / 57,263 records/s`，cache scan median `0.034s -> 0.033s`，cache-backed replay median `0.099s -> 0.094s`。replay single-encode median `87,981,829ns / 568,299 events/s -> 78,724,454ns / 635,127 events/s`。end-to-end multi-backtest `127,186,747ns / 386,455 events/s -> 119,430,818ns / 411,552 events/s`，同為 `16 fills / 147,458 output records` 且 checksum equivalent；無未解釋重大 regression。
- Final architecture summary：`Teralion provider -> verified source -> neutral DomainEvent -> rebuildable cache -> replay/MarketState -> read-only strategy -> execution -> accounting -> immutable artifacts`。已移除 TUI/interactive display、三個獨立 normalizer workspace crates、legacy strategy output dual path／舊 evaluator aliases，以及重複 execution/accounting/helper paths；保留 provider composition boundary、Teralion wire format 與 domain event 分離、`match_time` ordering、MarketState ownership、frozen plan/session/contract、strategy read-only、per-instrument ledger 與 artifact lineage。Breaking changes 包含 StrategyOutput canonical v3 only、legacy evaluator aliases 與舊 RunConfig selection helpers 移除、benchmark owner 移動，以及 neutral event fixture 必須明確提供 signal。新的 provider 只需在 composition root 將 verified source 映射至 generic partition/cache/replay contracts。
- Remaining known out-of-scope：repository synthetic fixtures 仍非完整交易日；外部 provider authorization/release data、即時交易、完整交易所撮合、逐筆委託、queue position、hidden liquidity、IOC/FOK、零股／盤後／鉅額與 options exercise/assignment 不屬本產品 scope。
- Final status：所有 Goal 012 acceptance checkbox 均完成；本 goal 以一個 focused commit 收尾，未 push。
