# 012：最終 correctness、provider neutrality、performance 與 LOC 驗收

Status: pending
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

列 long-run modifications；不 commit/push。

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

- [ ] provider neutrality PASS。
- [ ] Goal-006 所有 in-scope capability PASS。
- [ ] fmt/tests/clippy PASS。
- [ ] fixture/provider/offline smoke PASS。
- [ ] deterministic rerun PASS。
- [ ] accounting reconciliation PASS。
- [ ] performance 無未解釋重大 regression。
- [ ] final cloc/crate count 與 baseline 比較完成。
- [ ] final docs/PRD/traceability 與 code 一致。
- [ ] 無 legacy dual path/TUI/hidden Teralion core dependency。
- [ ] 無因 cleanup 刪必要 correctness evidence。
- [ ] working tree 未被 commit/push。

## 執行紀錄

- Final revision / working tree：
- Baseline Rust LOC：
- Final Rust LOC / delta：
- Baseline/final crate count：
- Provider neutrality：
- Capability matrix：
- Full validation：
- Determinism：
- Accounting reconciliation：
- Performance baseline vs final：
- Final architecture summary：
- Remaining known out-of-scope：
- Final status：
