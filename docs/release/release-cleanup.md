# Release cleanup：從 milestone repository 到 production tool

## 1. 文件定位

本文件定義 `osmium-lab` 從 M1–M5 milestone repository 整理成可發行工具的目標
結構、CLI、fixture 邊界、migration 規則與待辦事項。

截至 2026-08-03，M1–M5 已完成 formal acceptance；本文件是下一個 release cleanup
工作的設計與 checklist，尚未代表 crate rename 或 CLI migration 已完成。

本文件不改寫歷史 milestone 文件、formal acceptance report、checksum 或 source
provenance。歷史文件可以保留 `m1-*`、`m2-*`、`m3-*` 名稱；release source、public
CLI 與 production package 則不得再以 milestone 作為主要 identity。

## 2. Release 目標

Release 使用者應只需要理解：

```text
osmium CLI
  -> versioned run config
  -> verified local source
  -> rebuildable replay cache
  -> replay / backtest
  -> inspectable run artifacts
```

Release 必須維持產品需求的核心邊界：

- 已驗證的 local source 可重用；replay cache 是可刪除、可重建的衍生物。
- Teralion wire format 不進入 strategy、replay、MarketState 或 simulation API。
- `match_time` 是唯一 replay clock；相同內容的 ordering 與 artifacts 必須 deterministic。
- MarketState 只由可觀察的 trade 與完整五檔 snapshot 更新，不重建 queue position。
- strategy 只能讀取 market state；replayer 只開啟 explicit universe 所需的 streams。
- 資料準備完成後，replay、backtest、inspect 預設不使用網路或 credential。

Release cleanup 的目標不是重寫核心 domain，也不是新增市場；目標是移除 milestone
命名造成的 public boundary、收斂 CLI、分離 acceptance tooling，並使安裝與使用流程
可以獨立於 repository 歷史理解。

## 3. Target repository structure

### 3.1 Production workspace

```text
crates/
  osmium-cli/              # public binary: osmium
  osmium-config/           # versioned config parsing, validation and plan resolution
  osmium-runner/           # replay/backtest orchestration and run artifacts
  data-sync/               # source acquisition, cursor, source repository and cache build
  run-planner/             # session, partition and execution-plan domain rules
  replay-engine/           # deterministic stream merge and replay lifecycle
  market-types/            # domain events, identities, canonical values and references
  market-state/            # read-only market state and reducer profiles
  strategy-api/            # strategy/context/intent interfaces
  execution-sim/           # fill, fee, tax, position and accounting models
  normalizer/
    twse/                  # TWSE equity and warrant adapters
    tpex/                  # TPEx equity adapter
    taifex/                # TAIFEX futures and option adapters

tools/
  acquisition/             # maintainer-only source acquisition helpers
  acceptance/               # maintainer-only fixture builders and formal harnesses

fixtures/
  smoke/                   # small, explicitly redistributable CI/developer fixture
  acceptance/               # optional acceptance bundle, distributed separately

docs/
  release/release-cleanup.md
  increments/               # historical M1–M5 scope and decisions
  verification/             # historical evidence and current release gates
```

`osmium-cli` 是 release binary；Rust crates 是 implementation boundary，除非另有
明確 API policy，不承諾它們是穩定的第三方 library API。`normalizer/*` 可以保留
market-specific 名稱，因為它們描述 domain adapter，不是 delivery milestone。

### 3.2 Milestone crate migration

| Current identity | Release identity | Action |
| --- | --- | --- |
| `crates/m3-config`、`M3Config`、`M3PlanBundle` | `osmium-config`、`RunConfig`、`PlanBundle` | 先搬移並改名；保留 schema compatibility，不保留 milestone public type |
| `crates/m2-config` | `osmium-config::legacy` 或 migration module | 將 `config_version: 1` 的讀取集中到 neutral config crate；不再作為 workspace package |
| `crates/m2-runner` | `osmium-runner` | 搬移 replay/backtest、inspection、artifact publication；移除 `M2` 命名 |
| `crates/m1-runner` | `replay-engine` 或 `osmium-runner` module | 依責任合併；不得為了歷史 milestone 保留獨立 production crate |
| `m3_fixture_data` binary | `tools/acceptance/osmium_fixture_data` | 移出 production workspace；只供 maintainer acceptance 使用 |
| `M2Command`、`M2CommandKind` | `Command`、`CommandKind` | CLI parser 改用產品 workflow 命名 |
| `M2AcceptanceStrategy` | acceptance-only strategy module | 不進 public strategy API；只保留 acceptance harness 所需 binding |

Migration 完成後，release workspace 不應包含 `m1-*`、`m2-*` 或 `m3-*` production
package。直接刪除舊目錄只可發生在 replacement 已編譯、測試、打包並完成 reference
search 之後；不能用 `git rm` 取代功能搬移。

歷史 evidence 中出現的舊 package name 不需重寫。新的 release evidence 必須使用
neutral crate name，並在 migration report 中記錄 old-to-new mapping。

## 4. Release config boundary

### 4.1 User-facing config

Release 的 user-facing identity 是 `RunConfig`，不是 `M3Config`。YAML 的
`config_version` 表示 config schema version，不表示 milestone；crate rename 不應
單獨造成 schema version bump。

第一個 release cleanup 版本應：

- 以 neutral parser 接受目前有效的 `config_version: 2`。
- 將舊 `config_version: 1` 支援放在明確的 legacy migration path；migration 後使用
  同一個 canonical `RunConfig`。
- 拒絕 unknown fields、credential-bearing fields、invalid market/reference/economics
  combinations。
- 將 instrument kind、underlying、expiry、strike、option side、currency、multiplier、
  quantity unit 與 provenance 綁入 effective config identity。
- 不把 absolute `data_root`、output path、wall clock 或 credential 放入 plan identity。

### 4.2 Public config workflow

```text
config file
  -> parse and validate
  -> migrate legacy schema if explicitly allowed
  -> resolve sessions, partitions, instruments and economics
  -> materialize effective config
  -> calculate plan identity
```

`effective-config.yaml`、`execution-plan.yaml` 與 run manifest 必須使用 neutral
terminology。舊名稱只能出現在 migration diagnostics 或歷史 evidence。

## 5. Release CLI contract

### 5.1 Target command surface

```sh
osmium version
osmium init [--path <config.yaml>]
osmium config check --config <file>
osmium plan --config <file>
osmium data sync --config <file>
osmium data verify --config <file>
osmium cache prepare --config <file>
osmium replay --config <file> [--output <directory>]
osmium backtest --config <file> --output <new-directory>
osmium inspect --run <run-directory>
```

Optional convenience command：

```sh
osmium run --config <file> --output <new-directory>
```

`run` 可以依序執行 plan、必要的 data/cache preparation 與 backtest，但不得隱藏
network side effect；若需要網路，必須在 summary 明示並要求使用者確認 policy。

### 5.2 Command side-effect policy

| Command | Network | Writes | Purpose |
| --- | --- | --- | --- |
| `version` | no | no | 顯示 binary、schema、event/cache compatibility versions |
| `init` | no | new config only | 建立無 secret 的 config skeleton |
| `config check` | no | no | 解析、migration、validation 與 effective config diagnostics |
| `plan` | no by default | no | 顯示 source/cache reuse 或 preparation action |
| `data sync` | yes, explicit | source revision | 下載、驗證並 atomic publish complete source |
| `data verify` | no | no | 重驗 source completeness、identity、checksum 與 compatibility |
| `cache prepare` | no | new cache identity | 由 verified local source deterministic rebuild cache |
| `replay` | no | new replay artifacts | 只執行 market state、strategy observation，不做 accounting |
| `backtest` | no | new run directory | 執行 strategy、fill、fee/tax、accounting 與 artifacts |
| `inspect` | no | no | 驗證並呈現 run/source/cache lineage |

所有 command 應支援：

- `--help`
- `--format human|json`（machine-readable output 不與 human log 混用）
- `--quiet`、`--no-color` 與明確 log level
- stable non-zero exit code categories：usage、config、source、cache、replay、
  simulation、integrity、internal

Release CLI 不提供 implicit online fallback。`replay`／`backtest` 缺資料時應明確
要求先執行 `data sync` 或 `cache prepare`，不能在回播中途建立 HTTP client。

### 5.3 CLI migration from current commands

目前 top-level `sync`、`verify`、`cache prepare`、`replay`、`backtest`、`inspect` 的
行為是既有基礎；release cleanup 主要調整 namespace 與 help text：

```text
current `sync`   -> release `data sync`
current `verify`  -> release `data verify`
current `plan`    -> release `plan`
current others   -> same spelling, neutral help and artifact terminology
```

在 pre-release 階段可以保留一次性的 compatibility aliases，但 aliases 不應出現在
正式文件的 primary examples；release 完成後移除 milestone-oriented parser branches。

## 6. Fixture、source 與 cache distribution

### 6.1 三種資料層級

| Layer | Location | Release policy | Lifecycle |
| --- | --- | --- | --- |
| Smoke fixture | `fixtures/smoke/` | 可放入 source distribution；必須有 redistribution approval、metadata、checksum | 小型 CI／quickstart，immutable |
| Acceptance fixture | `fixtures/acceptance/` 或獨立 bundle | 依 market authorization 決定；M5 目前為 private-internal-review-only，不預設隨 public release 發布 | 大型 formal verification，checksum pinning |
| User source/cache | 使用者設定的 `data_root` | 不進 Git、不進 binary archive、不由 repository fixture 自動建立 | source 可重用；cache 可刪除重建 |

目前 repository 的歷史 fixture path 可以保留以維持 evidence link；release bundle 不
應要求使用者依賴 repository layout。應提供 fixture manifest，包含：

- fixture id、market、instrument kind、symbol、trading date、session
- source market、format registry、record counts、query window
- mapping／event／cache schema versions
- checksum、provenance、redistribution scope
- acquisition／verification tool version

### 6.2 不得進 release archive 的內容

- `raw/`：原始 acquisition dump、local credential context 或未整理 response。
- `target/`：build、source/cache staging、run output 與 temporary diagnostics。
- `.env`、API key、cookie、authorization、bearer token 或 credential-bearing URL。
- 未獲 redistribution approval 的大型 acceptance fixture。
- 某一台機器的 absolute path、wall-clock log 或 nondeterministic temporary file。

### 6.3 Fixture commands

Public CLI 不需要知道 repository fixture path。Maintainer-only tooling 可以提供：

```sh
tools/acquisition/acquire_fixture.sh <selection>
tools/acceptance/verify_fixtures.sh <manifest>
tools/acceptance/build_source_cache.sh --config <file> --fixtures <bundle>
tools/acceptance/run_formal.sh --output <evidence-directory>
```

這些工具的 source／fixture access policy 必須獨立於 `osmium` runtime；formal acceptance
可以使用 sandbox 或其他 network-denial runner，但 release CLI 不應依賴它。

## 7. Release artifact layout

Target distribution archive：

```text
osmium-<version>-<target>/
  bin/osmium
  examples/config.yaml
  docs/quickstart.md
  docs/config-reference.md
  docs/data-layout.md
  RELEASE-NOTES.md
  SHA256SUMS
  fixture-manifest.yaml       # metadata only unless separately approved
```

Run output remains user-owned and is not bundled into the binary archive：

```text
<run-directory>/
  run-manifest.yaml
  effective-config.yaml
  execution-plan.yaml
  data-lineage.yaml
  cache-lineage.yaml
  event-stream.blake3
  final-state.blake3
  orders.blake3
  fills.blake3
  ledger.blake3
  performance.yaml
  positions.yaml
  warnings.yaml
```

`run-manifest.yaml` 必須使用 release schema versions 並保留 source revision、cache
identity、mapping version、accounting model 與 strategy identity；不可因 crate rename
改變 domain checksum。

## 8. Release validation gates

Release candidate 必須通過：

### Build and package

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo build --release -p osmium-cli`
- `cargo package` 或等價 release archive smoke test
- clean environment 的 `cargo install`／binary invocation test

### CLI and data

- `osmium --help`、`version`、`init`、`config check` 可用。
- smoke fixture 可完成 plan、verify、cache prepare、replay、backtest、inspect。
- prepared source/cache 下，network-disabled replay/backtest 成功。
- second run reuse source/cache，不重複下載或覆寫 complete source。
- cache 删除後可由 source deterministic rebuild。
- universe 外 instrument 不開 stream。

### Integrity and reproducibility

- 10 次相同 run byte-identical。
- discovery permutation、cache rebuild、debug/release 結果 byte-identical。
- corruption、unknown format、wrong market、missing economics 與 invalid config 都有
  stable failure evidence。
- release archive、logs、manifest、run artifacts 與 Git history 不含 secret。
- fixture license／redistribution scope 逐份 review；未授權資料不能標成 public release。

## 9. Release cleanup TODO

以下項目目前都是 `todo`；完成後應各自使用小型、可 review 的 commit。

| ID | 優先級 | Todo | 完成條件 |
| --- | --- | --- | --- |
| RLS-01 | P0 | 決定 release distribution scope：public、private 或 internal | fixture／source／license policy 有 signed-off decision |
| RLS-02 | P0 | 建立 `osmium-config` 並搬移 `m3-config`／`m2-config` 行為 | config v1/v2 migration、plan identity、tests 通過 |
| RLS-03 | P0 | 建立 `osmium-runner` 並搬移 `m2-runner` | replay/backtest/inspect artifact checksum 不變 |
| RLS-04 | P0 | 移除 `m1-*`、`m2-*`、`m3-*` production workspace packages | `cargo metadata` 與 release tree 不再出現 milestone production crates |
| RLS-05 | P0 | 將 `m3_fixture_data` 與 formal scripts 移到 `tools/acceptance` | production build 不編譯 acceptance-only binary |
| RLS-06 | P1 | 收斂 CLI namespace、help、exit codes 與 JSON output | release CLI contract test 完整 |
| RLS-07 | P1 | 定義 smoke fixture 與大型 acceptance bundle distribution | manifest、checksum、authorization、download/verify flow 完整 |
| RLS-08 | P1 | 建立 release CI 與 clean-machine install test | no-secret、offline、package、reproducibility gates 通過 |
| RLS-09 | P1 | 更新 operations／quickstart／config reference | 使用者不需閱讀 M1–M5 文件即可完成一次 backtest |
| RLS-10 | P2 | 建立 versioning、CHANGELOG、release notes 與 support policy | binary、schema、event/cache compatibility policy 發布 |
| RLS-11 | P2 | 清理 historical code comments 與 public error names | production public surface 不再暴露 milestone terminology |
| RLS-12 | P2 | 產生 release archive、SBOM／license inventory 與 checksums | archive 可驗證、可安裝、可重現 |

## 10. Definition of done

Release cleanup 完成時必須同時滿足：

- production workspace 沒有 `m1-*`、`m2-*`、`m3-*` crate 或 public type。
- neutral config／runner crate 通過完整 workspace、release、offline acceptance。
- `osmium` 可以在 clean machine 由 documented command 安裝並顯示 help/version。
- 使用者以一份 neutral config 可完成 data check、cache preparation、replay/backtest
  與 inspect。
- smoke fixture 可取得；大型或私有 acceptance fixture 有獨立 manifest 與權限邊界。
- release archive 不含 raw dump、target、secret、未授權資料或 repository absolute path。
- source/cache/run artifact schema、checksum 與 accounting identity 可向前追溯。
- M1–M5 historical evidence 與 traceability links 維持可讀，並新增 release acceptance
  report。

## 11. Decisions required before implementation

在開始 RLS-02 之前需要固定三個 product decision：

1. 首個 release 是 public distribution 還是 private/internal distribution？這會決定
   M5 acceptance fixtures 是否能隨 bundle 提供。
2. 是否需要在首個 release 支援 `config_version: 1` migration，或直接要求使用者升級
   至目前 schema？
3. 首個 release 是 binary archive／installer 優先，還是同時承諾 crates.io library
   API？若沒有 library API 承諾，Rust crates 應保持 internal implementation boundary。

這三項決定完成前，不應刪除 acceptance evidence，也不應將 large/private fixture
直接移入 public distribution。
