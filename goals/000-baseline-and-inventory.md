# 000：建立 long-run baseline、能力清單與 LOC 基準

Status: done
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

- [x] revision / working tree 已記錄。
- [x] root Rust cloc baseline 已保存。
- [x] major-area LOC 已保存。
- [x] workspace fmt/test/clippy baseline 已保存。
- [x] synthetic smoke/fixture baseline 已保存。
- [x] benchmark baseline 可用性已保存。
- [x] capability baseline 已分類。
- [x] 無 production architecture change。

## 執行紀錄

- Baseline revision / working tree：`a410fe6a53a1162d9d525a44c0c91b6be02666c6`；`git status --short` 為乾淨。
- Workspace crates：14；`market-state`、`market-types`、`osmium-runner`、`execution-sim`、`replay-engine`、`twse-normalizer`、`strategy-api`、`osmium-config`、`data-sync`、`run-planner`、`taifex-normalizer`、`tpex-normalizer`、`example-strategy`、`osmium-cli`。產品範圍為 TWSE／TPEx 股票與權證、TAIFEX 期貨與選擇權的 verified source、offline cache、deterministic replay、Rust strategy、一般／scheduled execution、exact accounting 與 run artifacts；即時交易、零股／盤後／鉅額、完整撮合與 queue position 維持 out-of-scope。
- Rust LOC before / after：`~/cloc/cloc --vcs=git --include-lang=Rust .` 與 dirty-working-tree file-list cloc 均為 110 files / 3,669 blank / 272 comment / 42,761 code；delta `0`。
- Major-area LOC：
  
  | 區域 | files | blank | comment | code |
  | --- | ---: | ---: | ---: | ---: |
  | `crates/execution-sim` | 4 | 410 | 21 | 5,746 |
  | `crates/data-sync` | 11 | 432 | 26 | 4,952 |
  | `crates/normalizer/*` | 11 | 384 | 14 | 5,038 |
  | `crates/osmium-runner` | 6 | 233 | 0 | 4,529 |
  | `crates/strategy-api` | 13 | 488 | 38 | 4,796 |
  | `crates/market-types` | 32 | 545 | 115 | 3,928 |
  | `crates/run-planner` | 9 | 357 | 6 | 3,500 |
  | `crates/osmium-cli` | 5 | 201 | 18 | 3,058 |
  | `crates/market-state` | 6 | 220 | 8 | 2,468 |
  | `crates/replay-engine` | 10 | 232 | 25 | 2,370 |
  | `crates/osmium-config` | 1 | 129 | 1 | 1,675 |
- Validation：
  
  - `cargo metadata --no-deps --format-version 1`：exit 0；workspace 為上述 14 crates。
  - `cargo fmt --all --check`：exit 0。
  - `cargo test --workspace`：exit 0；320 passed、56 suites、0 failed。
  - `cargo clippy --workspace --all-targets -- -D warnings`：exit 0；無 warnings。
  - `python3 tools/acceptance/generate_synthetic_fixtures.py`、`git diff --exit-code -- fixtures`、`tools/acceptance/verify_compact_fixtures.sh`、`tools/acceptance/verify_fixture_bundle.sh --bundle . --manifest fixtures/smoke/manifest.yaml`、`tools/release/verify_license.sh`：皆 exit 0；6 compact synthetic entries、7 JSONL、27 records，smoke bundle verified，license verified。
  - `python3 -m unittest discover -s tools/acceptance -p 'test_*.py'`：exit 0；8 tests passed。
  - Offline smoke（無 `TERALION_API_KEY`）：`data verify`、`cache prepare`（reused）、`replay`、`backtest`、`inspect` 皆 exit 0。Replay 固定輸入為 2 events，`event_checksum=4668d8745908ed6475347c8e6d435fbcf865939a000498adee809f1a4b212860`、`final_state_checksum=a53c1e969d3b74398384527bf9c73e3ac8d0b30aa42a3c057aa720314c49a02a`。一般 smoke 為 2 orders / 0 fills；compiled strategy smoke 為 1 order / 1 fill，且 registry identity assertion 通過。
- Benchmark baseline：使用既有 offline targets、bench profile 與 synthetic input，三項 command 均 exit 0：
  
  - `cargo bench --locked --package data-sync --bench verified_cache_pipeline`：50,000 records；cache prepare 1.049s / 47,685 records/s；cache scan median 0.034s；cache-backed replay median 0.099s。
  - `cargo bench --locked --package replay-engine --bench canonical_hot_path`：50,000 events；single-encode replay median 87,981,829ns / 568,299 events/s；固定 checksum equivalent，pipeline speedup 1.27x。
  - `cargo bench --locked --package osmium-runner --bench full_multi_backtest`：8 TWSE instruments、49,152 events、21 rounds；median 127,186,747ns / 386,455 backtest events/s、16 fills、147,458 strategy output records；event/final-state checksums equivalent。
  
  這三個既有 benchmark 足以作 cache/normalization、replay hot path、end-to-end backtest 的後續比較；未新增 benchmark framework。
- Capability baseline：
  
  | 能力 | 文件／production owner／tests 證據 | 分類 |
  | --- | --- | --- |
  | provider/source | PRD DATA-01～04；`data-sync` source/revision/cache tests、normalizer fixture tests、acceptance partition/fixture tools | documented + tested（repo synthetic；real-day source evidence 維持 repository 外） |
  | market/instrument | PRD DATA-05；`market-types` identity/quantity/price tests、三個 normalizer fixtures、execution economics tests | documented + tested（支援市場的 synthetic coverage） |
  | replay/state | PRD REPLAY-01～06；`replay-engine` ordering/merge tests、`market-state` reducer/checksum tests、smoke replay | documented + tested |
  | strategy | PRD STRAT-01；`strategy-api` trait/registry/read-only tests、compiled strategy smoke | documented + tested |
  | order/execution | PRD SIM-01；`execution-sim` subsequent-event、market/limit、partial-fill 與 feedback tests | documented + tested |
  | scheduled execution | PRD SIM-01 scheduled model；`execution-sim` scheduled/depth tests、runner scheduled tests | documented + tested |
  | latency | PRD `market_data_latency_ms`／`order_latency_ms` semantics；scheduled latency/visibility tests與 benchmark inputs | documented + tested |
  | accounting/economics | PRD SIM-02；fee/tax/day-trade adjustment、cash charge transaction、equity/futures/options、P&L/reconciliation tests | documented + tested |
  | multi-instrument / multi-day | PRD REPLAY-05／DATA-05；planner session/universe tests、multi-stream runner benchmark、multi-instrument accounting tests | documented + tested（固定 synthetic input） |
  | artifacts / inspect | PRD OPS-02；runner artifact publication/manifest tests、CLI inspect smoke | documented + tested |
  | 即時交易、零股／盤後／鉅額、完整 exchange matching、queue position、hidden liquidity、交易所內部狀態重建 | PRD 明確列為不支援；不建立 production path | explicitly out-of-scope |
- 阻塞 / 既有失敗：無。首次直接執行 fixture helper 時未設定 CI 使用的 `CARGO_TARGET_DIR=target`，因此 binary path 不存在；依 `.github/workflows/ci.yml` 以相同 target 設定重建後通過。既有 `target/smoke-data` 由 fixture tool 正確拒絕覆寫，沿用其 verified revision 並完成離線 smoke；未刪除或覆寫任何資料。
- Production architecture change：無；本 goal 僅新增 baseline 執行紀錄。
- 下一步：001-remove-tui.md
