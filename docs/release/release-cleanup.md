# Release cleanup：從 milestone repository 到 production tool

## 1. 文件定位

本文件定義 `osmium-lab` 從 M1–M5 milestone repository 整理成可發行工具的目標
結構、CLI、fixture 邊界、migration 規則與待辦事項。

截至 2026-08-03，M1–M5 已完成 formal acceptance，並另外完成 TPEx warrant 的
focused real-fixture acceptance。本輪 cleanup 已完成 production crate rename/deletion、
current-schema-only config、CLI namespace 收斂、acceptance tooling 分離與 operations
文件搬遷；binary archive、fixture bundle flow、JSON output、installer、SBOM/license
inventory 與 clean-machine/reproducibility gate 已納入本輪 release gate。實際 private
artifact store URL 與 SSO policy 仍由部署環境提供。

本文件不改寫歷史 milestone 文件、formal acceptance report、checksum 或 source
provenance。歷史文件可以保留 `m1-*`、`m2-*`、`m3-*` 名稱；release source、release
CLI 與 production package 則不得再以 milestone 作為主要 identity。

首個 release 的 distribution scope 已決定為 private/internal，交付形式以
`osmium` binary archive／installer 為主。Rust crates 只作 internal implementation
boundary，不承諾穩定的第三方 library API 或 crates.io 發布。

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
命名造成的 implementation boundary、收斂 CLI、分離 acceptance tooling，並使安裝與
使用流程可以獨立於 repository 歷史理解。

## 3. Target repository structure

### 3.1 Production workspace

```text
crates/
  osmium-cli/              # release binary: osmium (private/internal first release)
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
    tpex/                  # TPEx equity and warrant adapters
    taifex/                # TAIFEX futures and option adapters

tools/
  acquisition/             # maintainer-only source acquisition helpers
  acceptance/               # maintainer-only fixture builders and formal harnesses
  release/                  # internal archive packaging and release smoke checks

fixtures/
  smoke/                   # small, explicitly redistributable CI/developer fixture
  acceptance/               # optional acceptance bundle, distributed separately

docs/
  release/release-cleanup.md
  increments/               # historical M1–M5 scope and decisions
  verification/             # historical evidence and current release gates
```

`osmium-cli` 是 release binary；首個 release 以 binary archive／installer 交付。Rust
crates 是 implementation boundary，除非另有明確 API policy，不承諾它們是穩定的第三方
library API，也不要求 crates.io packaging。`normalizer/*` 可以保留 market-specific
名稱，因為它們描述 domain adapter，不是 delivery milestone。

### 3.2 Milestone crate migration

| Current identity | Release identity | Action |
| --- | --- | --- |
| `crates/m3-config`、`M3Config`、`M3PlanBundle` | `osmium-config`、`RunConfig`、`PlanBundle` | 已搬移並改名；舊 workspace member、directory 與 public types 已移除 |
| `crates/m2-config` | — | 未搬移 v1 parser；`osmium-config` 只接受 v2，release binary 對 v1 回傳穩定 upgrade error；package 已刪除 |
| `crates/m2-runner` | `osmium-runner` | replay/backtest、inspection、artifact publication 已搬移；舊 package/directory 已移除 |
| `crates/m1-runner` | `tools/acceptance/osmium_m1_runner` | acceptance runner 移出 production workspace；release binary 不再提供 fixture mode |
| `m3_fixture_data` binary | `tools/acceptance/osmium_fixture_data` | 已移出 production workspace；fixture builder 以 standalone manifest 建置 |
| `M2Command`、`M2CommandKind` | `Command`、`CommandKind` | 已改用產品 workflow 命名，並將 sync/verify 收斂至 `data` namespace |
| `M2AcceptanceStrategy` | neutral acceptance strategy module | 已移除 v1 strategy；acceptance binding 改為 neutral，僅由 current runner/CLI 使用 |

Migration 完成後，release workspace 必須刪除 `m1-*`、`m2-*` 與 `m3-*` production
package、crate directory、workspace member 與其 public milestone types；release
version 不保留這些 package 作為 alias。本輪已在 replacement 編譯、測試與 reference
search 後完成刪除；binary packaging 仍由 RLS-08/RLS-12 驗證。未來刪除不能用 `git rm`
取代功能搬移。

刪除的目標是 release source tree，不是歷史 evidence：舊 package name 可以留在
milestone 文件、既有 acceptance report 與 migration report 中，讓歷史 checksum
仍可追溯。

歷史 evidence 中出現的舊 package name 不需重寫。新的 release evidence 必須使用
neutral crate name，並在 migration report 中記錄 old-to-new mapping。

首個 release 不維護 M2 config compatibility。`config/m2-twse-2330.yaml` 與其他
`config_version: 1` 檔案只作 historical acceptance material；它們不是 release
example，release binary 也不應以 legacy parser 讀取它們。

## 4. Release config boundary

### 4.1 User-facing config

Release 的 user-facing identity 是 `RunConfig`，不是 `M3Config`。YAML 的
`config_version` 表示 config schema version，不表示 milestone；crate rename 不應
單獨造成 schema version bump。

第一個 release cleanup 版本應：

- 以 neutral parser 接受目前有效的 `config_version: 2`。
- 不支援 `config_version: 1` migration；遇到 v1 時以穩定錯誤明確要求使用者升級至
  目前 schema，不保留 legacy parser 或 migration module。
- 拒絕 unknown fields、credential-bearing fields、invalid market/reference/economics
  combinations。
- 將 instrument kind、underlying、expiry、strike、option side、currency、multiplier、
  quantity unit 與 provenance 綁入 effective config identity。
- 不把 absolute `data_root`、output path、wall clock 或 credential 放入 plan identity。

### 4.2 Public config workflow

```text
config file
  -> parse and validate
  -> accept current schema only
  -> resolve sessions, partitions, instruments and economics
  -> materialize effective config
  -> calculate plan identity
```

`effective-config.yaml`、`execution-plan.yaml` 與 run manifest 必須使用 neutral
terminology。舊名稱只能出現在歷史 evidence；release diagnostics 不應暴露 milestone
config identity。

目前的 `config/m5-tpex-warrant.yaml` 是 maintainer acceptance configuration，不是
release example；它固定 TPEx `72328U`／`2026-07-20` 的 private fixture 與 contract
provenance。release example 應改成不依賴 repository fixture path 的 neutral config，
並由 user-owned `data_root` 提供 source/cache。

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
osmium replay --config <file>
osmium backtest --config <file> --output <new-directory>
osmium display --config <file>
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
| `config check` | no | no | 解析目前 schema、validation 與 effective config diagnostics |
| `plan` | no by default | no | 顯示 source/cache reuse 或 preparation action |
| `data sync` | yes, explicit | source revision | 下載、驗證並 atomic publish complete source |
| `data verify` | no | no | 重驗 source completeness、identity、checksum 與 compatibility |
| `cache prepare` | no | new cache identity | 由 verified local source deterministic rebuild cache |
| `replay` | no | new replay artifacts | 只執行 market state、strategy observation，不做 accounting |
| `backtest` | no | new run directory | 執行 strategy、fill、fee/tax、accounting 與 artifacts |
| `display` | no | no | 以只讀 TUI 依 `match_time` 播放 explicit universe 的歷史行情 |
| `inspect` | no | no | 驗證並呈現 run/source/cache lineage |

RLS-06 的 target contract 是所有 non-interactive command 支援：

- `--help`
- `--format human|json`（machine-readable output 不與 human log 混用）
- `--quiet`、`--no-color` 與明確 log level
- stable non-zero exit code categories：usage、config、source、cache、replay、
  simulation、integrity、internal

目前 implementation 已提供 `--help`、human-readable summary、namespace 與穩定主要
exit categories；`--format`、`--quiet`、`--no-color` 與完整 machine-readable JSON
仍列為 RLS-06 follow-up，不把未實作選項放入使用者 quickstart。

Release CLI 不提供 implicit online fallback。`replay`／`backtest` 缺資料時應明確
要求先執行 `data sync` 或 `cache prepare`，不能在回播中途建立 HTTP client。

### 5.3 Display contract

`display` 是只讀的 historical-market TUI，不是 strategy、simulation 或另一套 replay
engine。它必須：

- 使用已驗證的 local source／cache；啟動與播放不使用網路或 credential，也不寫入 run
  directory。
- 依 `match_time` 播放 explicit universe；標的切換不得改變 replay clock 或市場狀態。
- 預設以 `1.0x` 播放，支援 pause、resume、固定倍率調整與標的切換。
- 顯示目前標的、`match_time`、播放狀態、速度、簡單價格折線、同時間範圍成交量、完整
  五檔與最新成交明細。
- 不新增 strategy、撮合、queue position、imbalance 或 trade delta 語意；TUI display
  logic 必須與 replay／market-state domain 分離。

### 5.4 CLI migration from current commands

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
| Smoke fixture | `fixtures/smoke/` | repository-owned synthetic payload，可放入 CI/internal archive；仍必須有 metadata、checksum | 小型 CI／quickstart，immutable |
| Acceptance fixture | `fixtures/acceptance/` 或獨立 bundle | private/internal access-controlled bundle；M5 目前為 private-internal-review-only，不進 unrestricted archive | 大型 formal verification，checksum pinning |
| User source/cache | 使用者設定的 `data_root` | 不進 Git、不進 binary archive、不由 repository fixture 自動建立 | source 可重用；cache 可刪除重建 |

目前 repository 的歷史 fixture path 可以保留以維持 evidence link；release bundle 不
應要求使用者依賴 repository layout。應提供 fixture manifest，包含：

- fixture id、market、instrument kind、symbol、trading date、session
- source market、format registry、record counts、query window
- mapping／event／cache schema versions
- checksum、provenance、redistribution scope
- acquisition／verification tool version

目前已驗證的 M5 private acceptance inventory 至少包含：

- TWSE warrant `03003T`／`2026-07-20`
- TAIFEX option `TXO24000U6`／`2026-07-28`
- TPEx warrant `72328U`／`2026-07-20`，11 筆 `WARRANT_REALTIME`／`WARRANT_SNAPSHOT`

TPEx warrant 的 fixture metadata、daily instrument、source revision 與 focused
network-disabled acceptance report 已提交，但 redistribution scope 仍是
`private-internal-review-only`。因此它可以作為 private/internal acceptance bundle 的
manifest entry，但不能直接放進 unrestricted binary archive。

RLS-07 的 repository flow 已固定為：

```text
package_fixture_bundle.sh
  -> manifest.yaml + explicitly listed payload paths
  -> fixture checksum / secret scan
  -> checksums.sha256 + private archive

fetch_fixture_bundle.sh
  -> local archive/directory or HTTPS bearer-token source
  -> archive path safety + checksums.sha256
  -> manifest/payload checksum verification
  -> new local bundle directory
```

HTTPS source 必須由部署環境提供 `OSMIUM_FIXTURE_BUNDLE_TOKEN` 或明確指定的 token
environment；repository 不假設、保存或散布實際 SSO／artifact store URL。`fixtures/smoke/`
是 synthetic、可放入 CI 的小型 fixture；`fixtures/acceptance/` 仍維持 private scope。

### 6.2 不得進 release archive 的內容

- `raw/`：原始 acquisition dump、local credential context 或未整理 response。
- `target/`：build、source/cache staging、run output 與 temporary diagnostics。
- `.env`、API key、cookie、authorization、bearer token 或 credential-bearing URL。
- 未獲 redistribution approval 的大型 acceptance fixture。
- 某一台機器的 absolute path、wall-clock log 或 nondeterministic temporary file。

### 6.3 Fixture commands

Release CLI 不需要知道 repository fixture path。Maintainer-only tooling 可以提供：

```sh
tools/acquisition/acquire_m5_fixtures.sh
tools/acceptance/verify_m5_fixtures.sh
cargo build --release \
  --manifest-path tools/acceptance/osmium_fixture_data/Cargo.toml
tools/acceptance/run_m5_acceptance.sh --output <evidence-directory>
```

目前 TPEx warrant 的 maintainer harness 是：

```sh
tools/acceptance/run_tpex_warrant_acceptance.sh --output target/acceptance-tpex-warrant-<date>
```

它會在 network-disabled、credentials-absent 環境驗證 fixture integrity、source/cache
lineage、replay、backtest、inspect、重跑 determinism、cache rebuild、debug/release
一致性與 corruption rejection；這些能力維持在 `tools/acceptance/`，不讓 `osmium`
runtime 依賴 repository script。

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
  SUPPORT.md
  SBOM.cdx.json
  THIRD-PARTY-LICENSES.txt
  BUILD-METADATA
  DEPENDENCIES.txt
  SHA256SUMS
  fixture-manifest.yaml       # metadata only; acceptance payload is separate and access-controlled
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
- `tools/release/package.sh --output target/osmium-internal.tar.gz`
- `tools/release/smoke_clean_machine.sh --archive <archive> --checksum <file>`
- `tools/release/verify_reproducibility.sh --output <new-directory>`
- clean environment 由 binary archive／installer 安裝並執行 `osmium --help`／`version`

首個 release 不以 `cargo package`、`cargo install` 或 crates.io library API 作為交付
gate；Rust crates 仍須通過 workspace build/test，但不構成 user-facing package。

### CLI and data

- `osmium --help`、`version`、`init`、`config check` 可用。
- smoke fixture 可完成 plan、verify、cache prepare、replay、backtest、inspect。
- `osmium display --config <file>` 可在已準備的 source/cache 上離線啟動只讀 TUI；預設
  `1.0x`，支援 pause/resume、固定倍率調整與 explicit universe 標的切換，且切換不改變
  replay clock 或市場狀態；畫面需包含標的、時間、狀態、速度、價格折線、成交量、完整
  五檔與最新成交。
- prepared source/cache 下，network-disabled replay/backtest 成功。
- second run reuse source/cache，不重複下載或覆寫 complete source。
- cache 刪除後可由 source deterministic rebuild。
- universe 外 instrument 不開 stream。

### Integrity and reproducibility

- 10 次相同 run byte-identical。
- discovery permutation、cache rebuild、debug/release 結果 byte-identical。
- corruption、unknown format、wrong market、missing economics 與 invalid config 都有
  stable failure evidence。
- release archive、logs、manifest、run artifacts 與 Git history 不含 secret。
- fixture license／redistribution scope 逐份 review；未授權資料不能進入 release
  distribution。

現階段已完成的是 production source cleanup、binary archive、installer、fixture flow、
SBOM/license inventory 與 release candidate smoke gate；仍需在實際 internal deployment
執行一次由 organization artifact store/SSO 提供的 HTTPS authorization review：
`docs/verification/evidence/m5/tpex-warrant-2026-08-03/acceptance-report.yaml` 記錄
TPEx warrant exact-symbol/date 的 real-fixture 結果；它證明 domain、source/cache 與
offline determinism 邊界，不能取代 provider-side authorization audit；repository 內的
local archive、manifest、checksum、install 與 offline/reproducibility gate 已有可重現
evidence。

## 9. Release cleanup TODO

以下項目是 release cleanup execution checklist；首個 release 的 distribution、schema
compatibility 與 delivery policy 已在第 11 節固定。TPEx warrant acceptance 已補足
release planning 所需的 fixture evidence，但不會自動完成 crate migration 或 packaging
工作。完成後仍應各自使用小型、可 review 的 commit。

| ID | 優先級 | Todo | 完成條件 |
| --- | --- | --- | --- |
| RLS-01 | P0 | [完成] 首個 release 採 private/internal distribution | decision 已記錄於第 11 節；fixture authorization 依 RLS-07 的 bundle policy 管理 |
| RLS-02 | P0 | [完成] 建立 `osmium-config` 並搬移 current config 行為 | v2 parser、v1 rejection、plan identity、focused tests 通過 |
| RLS-03 | P0 | [完成] 建立 `osmium-runner` 並搬移 runner 行為 | runner package 已 neutral；workspace/release tests 與 archive checksum gate 已通過 |
| RLS-04 | P0 | [完成] 移除 `m1-*`、`m2-*`、`m3-*` production workspace packages | `cargo metadata` 與 production tree 不再包含 milestone crates |
| RLS-05 | P0 | [完成] 將 fixture builder 與 formal scripts 移到 `tools/acceptance` | production workspace 不編譯 acceptance-only binary |
| RLS-06 | P1 | [完成] 收斂 CLI namespace、help、exit codes 與 JSON output | non-interactive commands 支援 `--format human|json`、`--quiet`、`--no-color`；category codes 與 JSON envelope 有 focused tests |
| RLS-07 | P1 | [完成] 定義 smoke fixture 與 private/internal acceptance bundle distribution（包含 TPEx warrant private scope） | smoke fixture、private manifest、package/fetch/verify flow、path safety、checksum 與 secret scan 已完成；provider URL/SSO 由部署環境注入 |
| RLS-08 | P1 | [完成] 建立 release CI 與 binary archive／installer clean-machine install test | deterministic archive、offline installer、clean-machine smoke、reproducibility script 已完成；CI workflow 已接 archive smoke |
| RLS-09 | P1 | [完成] 更新 operations／quickstart／config reference | README、quickstart、config reference、data layout 與 operations docs 已更新 |
| RLS-10 | P2 | [完成] 建立 versioning、CHANGELOG、release notes 與 support policy | `CHANGELOG.md`、release notes 與 [SUPPORT.md](SUPPORT.md) 已發布 |
| RLS-11 | P2 | [完成] 清理 historical code comments 與 public error names | production naming 已 neutral；允許保留的 historical/digest references 已在 [namespace-review.md](namespace-review.md) 分類 |
| RLS-12 | P2 | [完成] 產生 release archive、SBOM／license inventory 與 checksums | deterministic archive、CycloneDX SBOM、transitive license inventory、internal/external checksums 已接入 package script |

## 10. Definition of done

Release cleanup 完成時必須同時滿足：

- production workspace 沒有 `m1-*`、`m2-*`、`m3-*` crate 或 public type。
- neutral config／runner crate 通過完整 workspace、release、offline acceptance。
- 首個 release 以 binary archive／installer 交付；`osmium` 可以在 clean machine 由
  documented command 安裝並顯示 help/version。
- archive 內容由 deterministic packager 產生，兩次固定 `SOURCE_DATE_EPOCH` 的 build
  byte-identical，並包含 SBOM 與完整 transitive dependency/license inventory。
- `osmium` 只接受目前 schema；`config_version: 1` 以明確錯誤拒絕，不提供 M2 config
  compatibility。
- 使用者以一份 neutral config 可完成 data check、cache preparation、replay/backtest
  與 inspect。
- smoke fixture 可由 private/internal distribution 取得；大型或私有 acceptance
  fixture 有獨立 manifest 與權限邊界。
- release archive 不含 raw dump、target、secret、未授權資料或 repository absolute path。
- `fixtures/smoke/` 可以在無 credential 的 CI 中完成 bundle verify；acceptance bundle
  必須先通過 manifest checksum 與 private authorization flow。
- source/cache/run artifact schema、checksum 與 accounting identity 可向前追溯。
- M1–M5 historical evidence 與 traceability links 維持可讀，並新增 release acceptance
  report。

## 11. Release decisions fixed before implementation

以下三項 product decision 已固定，並作為 RLS-02 之後 implementation 的邊界：

1. 首個 release 採 private/internal distribution，使用 access-controlled binary
   archive／installer；不做 unrestricted public release。M5 acceptance fixtures 依各自
   authorization 提供，TPEx warrant 維持 `private-internal-review-only`，不直接放入
   unrestricted archive。
2. 首個 release 不維護 M2 `config_version: 1` compatibility。只接受目前的
   `config_version: 2` schema；遇到 v1 時以穩定、可操作的錯誤要求使用者升級，不保留
   legacy parser、migration module 或 v1 writer。M2 v1 configs 與 evidence 仍可保留
   作為歷史追溯資料。
3. 首個 release 以 binary archive／installer 為優先交付形式。`osmium` binary 是
   user-facing product；Rust crates 維持 internal implementation boundary，不承諾
   crates.io 發布或穩定的第三方 library API。

這些決定不授權刪除 acceptance evidence，也不改變既有 fixture 的 redistribution
scope；它們只固定 release package、config compatibility 與 public API 的邊界。
