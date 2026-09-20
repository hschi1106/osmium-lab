# 011：全 repo 最終 redundancy / dead-code cleanup

Status: done
Depends on: 010-remove-legacy-compatibility.md

## 目標

前面功能與主要架構穩定後，做最後一次**證據導向** cleanup。

禁止新增產品能力；也不是「自由重寫整個 repo」。

## 先列 candidate，再修改

掃描：

```text
workspace crates
public exports
module graph
Cargo dependencies
one-call-site wrappers
forwarding functions
duplicate helpers
duplicate validations
duplicate error conversions
dead feature branches
obsolete tools
stale fixtures/docs/examples/traceability
large modules with repeated patterns
```

可用：

```sh
cargo metadata --no-deps --format-version 1
cargo tree
rg
~/cloc/cloc
```

不要新增第三方 dependency analyzer。

每個 candidate 先記：

```text
why redundant
current callers
what disappears
what behavior remains
```

證明不了就不動。

## 優先順序

1. dead code / unused export
2. stale wrapper / alias
3. same-semantics duplicate helper
4. duplicate validator
5. unnecessary DTO conversion
6. obsolete test fixture/tool branch
7. unnecessary crate/module boundary
8. over-generic abstraction with one real use

### Crate consolidation

只有 crate 幾乎全 forwarding、無真正 independent boundary、合併能刪 boilerplate/dependency 時才合。

不以 crate 數最少為目標。

## Tests cleanup

可 table-drive 重複 case、刪已刪 API test、合併 identical setup helper。

不能刪 semantic edge cases、不能做 giant unreadable test、不能降低 Goal-006 coverage。

## Docs/tools cleanup

刪 dead scripts、舊 architecture 現行說明、無 caller migration helper、duplicate docs。

保留 changelog history、license/legal、user workflow docs、必要 validation evidence。

## LOC

開始/結束記：

- total Rust LOC
- workspace crate count
- top 10 Rust areas

本 goal 期望 complexity/LOC 有實質下降；若 repo 已足夠精簡，可不改，但需 candidate evidence。

## Full regression

- Goal-006 matrix
- provider tests
- fixture generator/checks
- offline smoke
- fmt
- workspace tests
- clippy

## 驗收

- [x] candidate list 逐項判讀。
- [x] dead/forwarding/duplicate code 清除。
- [x] 無新增抽象來 cleanup。
- [x] capability/accounting/provider correctness 未下降。
- [x] docs/fixtures/tools 與 final architecture 一致。
- [x] crate boundary 只在有實益時調整。
- [x] fmt/test/clippy/smoke 通過。
- [x] total + top-area cloc 已記錄。
- [x] 無 push；依 long-run 要求保留一個 focused commit。

## 執行紀錄

- Baseline revision / working tree：`2d07d39`；Goal 011 開始前 worktree clean。
- Rust LOC before：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,665 blank / 271 comment / 42,566 code。
- Workspace crates before：`cargo metadata --no-deps --format-version 1` = 12 crates；前一個 provider decoupling goal 已移除非必要 workspace crates，這次不以減少 crate 數為目標。
- Top areas before：以 `~/cloc/cloc --include-lang=Rust --by-file crates` 的 code 行數排序，前十為 `execution-sim/accounting.rs` 2,104、`execution-sim/scheduled.rs` 2,038、`osmium-runner/scheduled.rs` 1,735、`osmium-config/lib.rs` 1,672、`run-planner/config.rs` 1,389、`osmium-runner/lib.rs` 1,233、`teralion/twse.rs` 1,226、`teralion/tpex.rs` 1,206、`osmium-cli/command.rs` 1,123、`osmium-runner/artifacts.rs` 1,103。
- Candidates：
  - `strategy-api/src/runner.rs` 的 `output_error_is_stable`／`context_error_is_stable` 只有定義，且以 `allow(dead_code)` 隱藏；沒有 caller 或測試價值。
  - `run-planner/tests/support/mod.rs` 的 `degraded` 看似只有定義，但進一步追到 `partition_plan.rs` 發現它是另一個 integration test crate 的 caller；shared support 在其他 test crate 中未使用，所以 `allow(dead_code)` 必須保留。
  - `data-sync` 的 `hex` 與 `decode_hex_32`／`decode_nibble` 在 cache、partition、storage、verify 四個 module 重複；功能相同且都屬 crate-private serialization utility。
  - `osmium-runner/src/artifacts.rs` 的 `encode_orders` 與 `encode_multi_orders` 完全相同，差別只在 single／multi container 取得 order iterator；可共用 iterator-based private encoder，保留 `OSORDERS1` 格式。
  - 已判讀但不動：single-call public wrappers 是 CLI／provider／library boundary；TWSE／TPEx parser 雖有相似形狀但欄位語意不同；public exports 有 workspace callers 或 external strategy/API boundary；12 個 crate 各自有獨立 domain／provider／storage boundary。
- Removed / retained：移除 `strategy-api` runner 的兩個零 caller error-string helpers；保留 `run-planner` shared test support 的 `degraded` helper 與 `allow(dead_code)`，因它跨 integration test crate 使用。`data-sync` 的四份 hex/decode helper 合併為 crate-private utility；single/multi `OSORDERS1` encoder 合併為 iterator-based private helper。其餘 public API、provider parser、crate boundary 與 current artifact/version semantics 保留。
- Validation：`rtk cargo fmt --all -- --check` 通過；`rtk cargo clippy --workspace --all-targets --all-features -- -D warnings` 通過；`rtk cargo test --workspace` = 328 passed（55 suites）；focused provider/data/runner tests = 157 passed（24 suites）；fixture generator + fixture diff、compact fixture、fixture bundle、license verifier 與 Python acceptance = 8 passed；release `osmium` build 通過。最終 working tree 的 fresh offline smoke 通過 config check、plan、data verify、cache prepare（reused）、replay、backtest、run（output 與 replay-only）、inspect，以及內建 example strategy backtest／inspect（1 order / 1 fill）。`rtk git diff --check` 通過。
- Rust LOC after / delta：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,657 blank / 271 comment / 42,497 code；相對 baseline code `-69`（blank `-8`、comment `0`）。
- Workspace crates after：`cargo metadata --no-deps --format-version 1` = 12 crates，沒有因 cleanup 搬動 crate boundary。
- Top areas after：前十仍為 `execution-sim/accounting.rs` 2,104、`execution-sim/scheduled.rs` 2,038、`osmium-runner/scheduled.rs` 1,735、`osmium-config/lib.rs` 1,672、`run-planner/config.rs` 1,389、`osmium-runner/lib.rs` 1,233、`teralion/twse.rs` 1,226、`teralion/tpex.rs` 1,206、`osmium-cli/command.rs` 1,123、`osmium-runner/artifacts.rs` 1,087；只縮短與本 goal 直接相關的 artifact/data-sync/runner 區域，沒有觸碰大模組做格式性重寫。
- 剩餘風險：`data-sync` 的共用 helper 仍是 crate-private，未擴張 public API；跨 crate 的 hex/parser 相似碼刻意保留以維持 provider/storage boundary。artifact order encoder 只合併 `OSORDERS1` 相同語意，scheduled `OSORDERS2` 與 fills/ledger/positions/performance 仍分開；shared test support 的 `allow(dead_code)` 仍可能在單一 integration test crate 中出現，但已有另一個 test caller。未做 crate consolidation 或新 abstraction。
- 下一步：012-final-acceptance.md
