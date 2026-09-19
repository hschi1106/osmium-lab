# 011：全 repo 最終 redundancy / dead-code cleanup

Status: pending
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

- [ ] candidate list 逐項判讀。
- [ ] dead/forwarding/duplicate code 清除。
- [ ] 無新增抽象來 cleanup。
- [ ] capability/accounting/provider correctness 未下降。
- [ ] docs/fixtures/tools 與 final architecture 一致。
- [ ] crate boundary 只在有實益時調整。
- [ ] fmt/test/clippy/smoke 通過。
- [ ] total + top-area cloc 已記錄。
- [ ] 無 commit/push。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- Workspace crates before：
- Top areas before：
- Candidates：
- Removed / retained：
- Validation：
- Rust LOC after / delta：
- Workspace crates after：
- Top areas after：
- 剩餘風險：
- 下一步：012-final-acceptance.md
