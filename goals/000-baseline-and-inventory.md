# 000：建立 long-run baseline、能力清單與 LOC 基準

Status: pending
Depends on: none

## 目標

在任何重構前建立可重現 baseline，讓後續每個 goal 都能回答功能有沒有掉、deterministic/accounting 有沒有變、workspace/dependency/LOC 實際變多少。

本 goal 不重構 production code。

## 必須記錄

### Repository

```sh
git rev-parse HEAD
git status --short
cargo metadata --no-deps --format-version 1
```

記錄 revision、dirty files、workspace crate count/names 與目前產品範圍。

### Rust LOC

```sh
~/cloc/cloc --vcs=git --include-lang=Rust .
```

保存 Rust files / blank / comment / code。

再對大型區域個別 cloc，至少：

```text
execution-sim
data-sync
normalizer/*
osmium-runner
strategy-api
market-types
run-planner
osmium-cli
market-state
replay-engine
osmium-config
```

### 測試 baseline

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

再依 repo 現有 validation 文件執行 repository-owned synthetic smoke / fixture checks。

保存 command、exit code、test summary、現有 event/final-state checksum 與既有失敗。

### Performance baseline

只使用 repo 既有固定、可離線 benchmark。

挑選能代表：

1. cache/normalization
2. replay hot path
3. end-to-end backtest

的現有 target，記錄 command/build profile/input/result。若本來不能穩定比較，只記不可作 baseline，不新建 framework。

## 能力清單

依文件 + production code + tests 分類：

```text
provider/source
market/instrument
replay/state
strategy
order/execution
scheduled execution
latency
accounting/economics
multi-instrument / multi-day
artifacts / inspect
```

標示：

- documented + tested
- documented but evidence unclear
- implementation exists but product claim unclear
- explicitly out-of-scope

Goal-006 會正式收斂。

## 不做

- 不刪 code。
- 不改 architecture。
- 不新增 capability。
- 不把 baseline failure 靜默修綠。
- 不 commit/push。

## 驗收

- [ ] revision / working tree 已記錄。
- [ ] root Rust cloc baseline 已保存。
- [ ] major-area LOC 已保存。
- [ ] workspace fmt/test/clippy baseline 已保存。
- [ ] synthetic smoke/fixture baseline 已保存。
- [ ] benchmark baseline 可用性已保存。
- [ ] capability baseline 已分類。
- [ ] 無 production architecture change。

## 執行紀錄

- Baseline revision / working tree：
- Workspace crates：
- Rust LOC total：
- Major-area LOC：
- Validation：
- Benchmark baseline：
- Capability baseline：
- 阻塞 / 既有失敗：
- 下一步：001-remove-tui.md
