# CLI 操作契約

## 1. 文件目的

本文件定義 release `osmium` binary 的 config、data、cache、replay、backtest、display
與 inspect 操作契約。歷史 M1–M5 fixture runner 不屬於 release CLI，位於
`tools/acceptance/`。

```text
cli_contract_version    = 4
run_config_version      = 2
execution_plan_version  = 1
run_manifest_version    = 2
binary                  = osmium
current_scope           = private/internal v2 multi-instrument offline replay and backtest
```

本文固定 command spelling、config shape、stage side effects、exit status 與 artifacts。
它不固定 CLI parsing library 或 Rust module layout。

依據：

- [產品需求](../product-requirements.md)
- [操作與非功能需求](../requirements/operations.md)
- [Data Sync](../design/data-sync.md)
- [Execution Simulation](../design/execution-sim.md)
- [Local Data](local-data.md)
- [Release validation](../release/VALIDATION.md)

## 2. Command overview

```text
  osmium version
  osmium init          [--path <config.yaml>]
  osmium config check  --config <file>
  osmium plan          --config <file>
  osmium data sync     --config <file>
  osmium data verify   --config <file>
  osmium cache prepare --config <file>
  osmium replay        --config <file>
  osmium display       --config <file>
  osmium backtest      --config <file> --output <new-directory>
  osmium inspect       --run <run-directory>
  osmium run           --config <file> --output <new-directory>
```

Acceptance fixture runner 不掛在 release CLI。maintainer 使用
`tools/acceptance/` 下的 standalone tools；完整日資料與 formal evidence 由 private
bundle 管理，不提交至本 repository。

所有 command 支援 `--help`。unknown option、缺少必要 argument 或互斥 argument 同時
出現，回傳 usage error。

除 `display` 外的 non-interactive command 都支援：

```text
--format human|json
--quiet
--no-color
```

`--format json` 只在 stdout 輸出一個 `schema_version: 1` 的 JSON object，包含 command、
status 與 result fields；錯誤則在 stderr 輸出同一個 JSON envelope，不混入 human log。
`--quiet` 只抑制成功輸出，不改變 exit status 或副作用；目前 human summary 沒有 ANSI
color，`--no-color` 仍是穩定的 release option。`display` 是互動式 TUI，不接受這三個
output option。

`osmium version` 會回報 binary version、`cli_contract`、config schema、event schema、
cache format 與 accounting compatibility version；這些欄位是 archive/automation 的
第一層 compatibility check。

## 3. Release run configuration

Release config 使用 YAML，`config_version: 2` 必填。repository 中的 M5 files 是
maintainer acceptance inputs；實際 release 使用者應由 `osmium init` 建立並填入自己的
data root、universe 與 economics。logical shape：

```yaml
config_version: 2

data:
  source: teralion
  data_root: data
  source_policy: strict
  cache_policy: reuse_or_rebuild

universe:
  trading_dates: [2026-07-20]
  instruments:
    - market: twse
      symbol: "2330"
      session_kinds: [regular]

strategy:
  id: <compile-time-linked-strategy-id>
  version: "<strategy-version>"
  parameters: {}

replay:
  data_policy: strict

simulation:
  fill:
    evidence: top_of_book
    quantity: observed
  market_data_latency_ms: 0
  order_latency_ms: 0
  allocation: acceptance_sequence
  slippage:
    model: adverse_fixed_delta
    delta: "<exact-decimal>"
  fee:
    model: configured_rate
    rate: "<exact-decimal>"
    applicable_sides: [buy, sell]
    minimum: "<exact-money>"
    precision: 0
    rounding: <explicit-policy>
    provenance: <non-secret-reference>
  tax:
    model: configured_rate
    rate: "<exact-decimal>"
    applicable_sides: [sell]
    minimum: "<exact-money>"
    precision: 0
    rounding: <explicit-policy>
    provenance: <non-secret-reference>
  initial_cash:
    currency: TWD
    amount: "<exact-money>"
  position_accounting: average_cost_v1
  marking:
    model: last_observable_mark_v1
    allow_midpoint_fallback: false

instrument_economics:
  - market: twse
    symbol: "2330"
    quantity_unit: trading_unit
    units_per_trading_unit: "<positive-exact-integer>"
    currency: TWD
    multiplier: "1"
    provenance: <verified-metadata-or-explicit-reference>

output:
  publication: create_new
```

實際 checked-in config 的 exact fee/tax/slippage values 是 acceptance input，不是平台
hidden default。latency 欄位是非負整數毫秒，缺省值為 `0`；兩個 latency 會加入
effective config checksum，並只延後 order 的 eligible `match_time`，不改寫 source event
或 replay ordering。費率、金額、價格等 exact numeric value 以 YAML string 表達，經 schema parser
轉成 `Decimal`／Money／Quantity；不得先經 `f64`。

### 3.1 Required validation

在任何 download、cache write、strategy execution 或 output staging 前：

- 拒絕 unknown `config_version`、unknown field、duplicate YAML key。
- 拒絕 locale-specific number、NaN、infinity、negative quantity/rate/delta。
- 驗證 date 已結束、market/symbol/session 可解析。
- 驗證 strategy binding/params/universe。
- 驗證 strict/degraded policy。
- 驗證 fill/economics/rounding/accounting combinations。
- 驗證 `data_root` 及 output policy，但 absolute path 不進 domain identity。
- 拒絕 config 中的 API key、authorization、cookie、bearer 或 credential-bearing URL。

`strategy.id + version` 必須精確命中目前 binary 的 compiled registry。選中的 factory
會先驗證 flat scalar `parameters`、套用 schema 明定的 default，再建立 strategy；
registry 隨即核對 binary identity、parameter checksum、explicit universe 與 sessions。
unknown id、version mismatch、unknown parameter、未加引號的 decimal 或 factory contract
mismatch 都屬 config error，且不得延後至資料 I/O 後才失敗。

default 必須由 versioned schema明示，並在 effective config materialize。displayed
effective config、canonical effective config bytes 與 checksum 使用相同 values；
不能有「顯示 A、實際使用 B」的 hidden setting。

### 3.2 Effective config

所有 config-based commands 先建立：

```text
EffectiveRunConfig {
    config_version
    canonical user values
    explicit schema defaults
    resolved strategy declaration
    resolved instrument economics
    version bindings
}
```

canonical checksum 不包含：

- API key 或 credential source。
- `data_root`／output 的 absolute expanded path；保存於 operational projection 的原始
  logical path 也不參與 domain plan/result identity。
- wall-clock timestamp。
- process、host、thread count；除非是具語意且明示的 execution parameter。
- YAML whitespace、comments 或 map insertion order。

## 4. `plan`

```sh
cargo run -p osmium-cli -- plan --config <file>
```

`plan`：

- 解析、驗證並 materialize effective config。
- 解析 session/calendar 與固定五分鐘 margins。
- 宣告完整 strategy universe。
- 檢查 local source/cache catalog。
- 建立 frozen `ExecutionPlan` 與 deterministic `plan_identity`。
- 顯示 source、verification、cache actions 及 network requirement。

至少輸出：

```text
plan_identity
effective_config_checksum
market/symbol/trading_date/session/window
source_state and source_action
verification_action
cache_state and cache_action
strategy identity/universe
simulation/accounting model identities
source compression policy
network_requirement
completion policy
```

`plan` 不下載 ticks、不建立 cache、不執行 strategy、不建立 successful run、不修改
complete source。online coverage lookup 若被 policy 允許，必須在執行前顯示
`network_requirement=required`；prepared-data offline plan 只使用
local evidence。

human stdout 是可供人閱讀的 stable summary；automation 應使用 `--format json` 的
structured result，不應解析 human line。

## 5. `data sync`

```sh
cargo run -p osmium-cli -- data sync --config <file>
```

`data sync`：

- 只執行 frozen plan 中的 `DownloadMissingSource` 或
  `ResumeOrRestartBuilding`。
- 是唯一可讀 Teralion credential、建立 HTTP client 的 command。
- 本地開發可將 `TERALION_API_KEY` 放在工作目錄的 `.env`；已存在的 process environment
  值優先，`.env` 只在需要同步時載入。
- 完成 coverage、cursor pages、daily instrument、verify 與 atomic source publish。
- complete source action 是 reuse，HTTP request count 必須為零。
- 不建立 replay cache、不執行 strategy/simulation。

成功 summary 至少包含：

```text
plan_identity
partition identity/state/action
HTTP request count
page/record counts
uncompressed/compressed byte counts and compression ratio
terminal cursor evidence
source revision/uncompressed semantic checksum
compressed storage checksums
daily instrument compressed/uncompressed checksums
published/reused
warnings
```

sync 中斷或失敗不發布 complete revision。stdout/stderr 不顯示 API key、full cursor、
header 或 signed URL。

## 6. `data verify`

```sh
cargo run -p osmium-cli -- data verify --config <file>
```

`data verify`：

- 完全離線，不讀 API key、不建立 HTTP client。
- 重算 source partition state、manifest/checksum/counts/version。
- 驗證 `ZstdPerPageV1`、frame checksum、compressed/uncompressed sizes及 SHA-256，
  並串流解壓 JSON envelope；不得在 data root產生 `.json`。
- 驗證 daily instrument/economics。
- 若 cache 存在，驗證 descriptor、lineage、bounds、payload checksum。
- 不執行 strategy。
- 不以 cache rebuild 掩蓋 corrupt source。

每個 partition 回報 `Missing`／`Building`／`Complete`／`Incomplete`／`Corrupt`、
reason 及建議 action。`Strict` 下任何非 complete required source 使 command 以 data
error 結束。

`verify` 預設 read-only；不下載、不 repair source、不建立 cache。report 可以寫入
明確 `--report <new-file>`，不得修改 verified revision。

## 7. `cache prepare`

```sh
cargo run -p osmium-cli -- cache prepare --config <file>
```

此 command：

- 完全離線。
- 只接受 complete compatible source。
- reuse valid cache，或由 local source deterministic rebuild。
- 不修改 source、不執行 strategy/simulation。
- 以 atomic directory publish cache。

成功 summary：

```text
partition/source revision
cache action/identity
event count and first/last ordering key
payload byte count/checksum
source zstd object decode count
source JSON parse count
HTTP request count = 0
```

invalid cache 不在原目錄修補；建立全新 cache identity或先隔離 invalid artifact。

### 7.1 Acceptance fixture preparation

Committed fixture 只提供 maintainer-only offline source preparation tool；目前
`fixtures/teralion/` 是 compact representative slices，不代表完整交易日：

```sh
target/release/osmium_fixture_data \
  --config examples/smoke.yaml \
  --fixtures fixtures/teralion \
  --data-root target/acceptance-data
```

先建立 acceptance-only binary：

```sh
cargo build --release \
  --manifest-path tools/acceptance/osmium_fixture_data/Cargo.toml
```

tool 會以與 online sync 相同的 `TeralionSync` cursor state machine，將每個 selected
JSONL shard 包成 fixture response、驗證 source format、發布 partition source revision，
接著建立 source-bound replay cache。它拒絕覆寫既有 data root；需要重建時使用新的空
data root。可先執行 `tools/acceptance/verify_compact_fixtures.sh` 檢查 slices 的
manifest、metadata、checksum、JSON 與大小上限。

需要完整日資料的 formal acceptance 必須先取得 private fixture bundle，不能把 compact
slice 當成完整日資料使用。

## 8. `replay`

### 8.1 Config mode

```sh
cargo run --release -p osmium-cli -- replay \
  --config <file> \
  --output <new-directory>
```

config replay：

- 完全離線。
- 要求 source complete 且 cache valid；不隱式 sync。
- 只開啟 frozen universe/date streams。
- 執行 event、MarketState、TradingContext 與 strategy observation。
- `simulation_binding = NotUsed`。
- order/fill/ledger artifacts 標示 `NotApplicable`，不以零筆冒充已執行 simulation。

若 cache missing 且 plan action 是 offline rebuild，使用者先執行 `cache prepare`；
`replay` 不在 runtime 邊讀 source 邊 fallback。

Config replay/backtest 會依 frozen `ReplayPlan` 只開啟每個 selected instrument/date
partition；`LocalCacheFactory` 可在 `OSMIUM_STREAM_OPEN_AUDIT` 指定
檔案記錄實際 opened bindings。多商品 merge 只保留各 stream 的 current head，不將
所有 source records 載入記憶體。

### 8.2 Market replay TUI

```sh
cargo run --release -p osmium-cli -- display \
  --config examples/config.yaml
```

此命令是簡化看盤介面，不產生回測 artifacts。release 只接受
`config_version: 2`，且只處理一個 `trading_date`；執行前必須完成 source verify 與
`cache prepare`。命令完全離線、不讀取 `.env` 或 `TERALION_API_KEY`，並依 frozen `ReplayPlan`
只開啟 explicit universe 的 cache streams。

所有 selected symbols 共用同一個 `match_time` 時鐘。`←`／`→` 只切換顯示標的，不改變
播放狀態或時間；`Space` 暫停／繼續；`+`／`-` 使用固定倍率
`0.1x, 0.25x, 0.5x, 1.0x, 2.0x, 5.0x, 10.0x, 25.0x, 50.0x`；`R` 重設並恢復
`1.0x`；`Q` 離開。

價格與一分鐘桶成交量使用同一個 replay start/end 範圍；左下顯示完整五檔，右下顯示最新成交在
最上方的明細。domain event 沒有可驗證的 aggressor side，因此 `SIDE` 顯示 `—`，不推測
`BUY`／`SELL`；畫面不顯示 imbalance、trade delta、queue position 或其他未由來源支持的
指標。完整 UI 邊界與清理行為見[Market replay TUI 設計](../design/market-replay-ui.md)。

## 9. `backtest`

```sh
cargo run --release -p osmium-cli -- backtest \
  --config <file> \
  --output <new-directory>
```

`backtest`：

- 完全離線且不讀 API key。
- preflight 驗證 frozen plan、complete source、valid cache、economics 及 versions。
- replay event/state/context。
- 執行 strategy、intent validation、pending fill、accounting、marking、
  reconciliation。
- 正常 EOF 時取消未完成 Day orders。
- atomic publish successful/degraded/failed artifacts。

Reference acceptance 必須是 `Strict` successful。`ExplicitDegraded` 使用不同
completion quality/result identity，且不允許 corrupt source。

Backtest 對每個 instrument 建立隔離 simulator/ledger，TAIFEX 使用
`FuturesV1` multiplier accounting，TWSE 使用 `EquityV1`；跨商品事件只共享
deterministic replay clock，不共享 fill eligibility、position 或 queue state。

output 必須是不存在的新 directory；不提供 `--force`。existing path 回傳 usage/
config error，避免混合兩次 run。

## 10. `inspect`

```sh
cargo run -p osmium-cli -- inspect --run <run-directory>
```

`inspect`：

- 不讀 config 外部 default、不重跑 strategy、不建立 HTTP client。
- 先驗證 run manifest version/status及 artifact checksums。
- 顯示 effective config/plan identity、lineage、versions、counts、warnings。
- backtest 顯示 orders、fills、cash、positions、fee、tax、P&L、
  reconciliation。
- failed/degraded run 顯示 failure stage、processed prefix或 degraded scopes。
- `NotApplicable`／`Unavailable(reason)` 不顯示成零。

目前支援的 output options：

```text
--format human|json
--quiet
--no-color
--orders
--fills
--positions
--warnings
```

最後四個 detail selector 仍未開放；它們不是 RLS-06 的 JSON envelope，也不能被當成
目前 release contract。

detail order 使用 canonical record order。missing attachment或 checksum mismatch
使 inspect 回傳 data/integrity error，但仍可輸出已驗證 manifest diagnostics。

## 11. `run`

```sh
cargo run --release -p osmium-cli -- run \
  --config <file> \
  --output <new-directory>
```

convenience orchestration：

```text
plan
-> sync required source actions
-> verify source
-> cache prepare/reuse
-> revalidate frozen plan and bind published source/cache into ReplayPlan
-> backtest
-> inspectable artifact publish
```

每個 stage 保留獨立 status、counts、identity及 failure。`run` 不改變 subcommand
semantics：

- complete source 第二次 run 不 sync。
- cache rebuild 仍只讀 local source。
- backtest stage 仍不持有 HTTP client/credential。
- 任一 required stage failed，後續 stage不執行。

`run` 是唯一同時包含 online preparation及 offline execution的 convenience command；
正式 network-disabled acceptance 使用分開程序驗證，不能用同一 process 的 mock
network 取代。

## 12. Execution plan

所有 config commands 共用：

```text
ExecutionPlan {
    plan_identity
    config_checksum
    requested_partitions
    source_actions
    verification_actions
    cache_actions
    replay_plan
    strategy_binding
    simulation_binding
    accounting_binding
    result_binding
    network_requirement
    completion_policy
    degraded_scopes
    version_set
}
```

plan 在有副作用 stage 前 freeze。任何 runtime discovery 若要求新增 partition、
stream、model default或 degraded scope，command 必須停止並要求重新 plan。

`plan_identity` 不含 absolute path、API key、wall-clock、random run ID或 thread
scheduling。effective plan 保存至 run artifacts 時，其 canonical semantic checksum
必須與 preflight plan相同。

## 13. Output publication

publisher 在 output parent 建 sibling staging directory，完成後 atomic rename。
successful backtest 至少發布：

```text
effective-config.yaml
execution-plan.yaml
run-manifest.yaml
data-lineage.yaml
cache-lineage.yaml
strategy.json
event-stream.blake3
final-state.blake3
strategy-output.bin
strategy-output.blake3
orders.bin
orders.blake3
fills.bin
fills.blake3
ledger.bin
ledger.blake3
positions.yaml
performance.yaml
warnings.yaml
run-summary.yaml
```

`strategy.json` 使用 `osmium-strategy-v1`，保存 strategy id/version、實際
`strategy-source-blake3` identity、parameter schema version、已套用 default 的參數與
canonical parameter checksum；此檔案本身也列入 `run-manifest.yaml` 的 artifact checksum。

`run-manifest.yaml` 至少記錄：

- status/completion quality/failure stage。
- config/plan/result identities。
- source/cache lineage。
- event、ordering、state、strategy、eligibility、fill、fee、tax、accounting、marking
  及 artifact versions。
- strategy binary/params identity。
- instrument economics provenance。
- event/warning/order/fill/position counts。
- artifact checksums及 reconciliation status。

wall-clock timestamps、duration、throughput及 host diagnostics可以記錄，但不進
domain result checksum。

failed run若要保存 diagnostics，使用明確 failed manifest及 partial artifact flags。
publisher不得讓 partial staging directory取得 successful final path/manifest。

## 14. Stdout、stderr 與 logging

stdout：

- stable stage summary。
- user-requested `inspect` detail。
- machine-readable output（`--format json`）不得混入 progress log。

stderr：

- error、warning及 progress；machine-readable mode下仍需可分離。
- error 至少包含 stage、market、symbol、trading date、format/occurrence/order/fill
  identity（適用時）及建議 action。

禁止輸出：

- API key、authorization/bearer/cookie。
- full cursor。
- signed/request URL中 credential query。
- source payload全量 dump。

人類訊息可以演進；stable category、identity、exit status及 machine-readable field
才是 automation contract。

## 15. Exit status

| Code | Category | 語意 |
| --- | --- | --- |
| `0` | `Success` | command 完整成功，或 help |
| `2` | `Usage` | command syntax、missing argument、unsupported option |
| `10` | `Config` | config/schema/preflight/declaration error |
| `20` | `Source` | Teralion、credential、network、source acquisition failure |
| `21` | `Cache` | cache missing/build/rebuild/read failure |
| `30` | `Replay` | event ordering、stream、replay lifecycle failure |
| `40` | `Simulation` | strategy、fill、arithmetic、accounting、reconciliation failure |
| `50` | `Integrity` | source/run/artifact checksum、manifest、corruption failure |
| `1` | `Internal` | 未分類 internal/storage failure；不得用於可分類 domain error |

Acceptance-only runner 的 historical fixture failure 由其 standalone test process
處理；release config mode 必須使用上表的分類。

legitimate rejected order 不使 process 回傳 `50`；它是 successful backtest 的
domain record。reconciliation failure一定回傳 `50`。

## 16. Offline 與 second-run acceptance

正式驗收分兩個 process：

```text
online:
  plan -> sync -> verify -> cache prepare

network-disabled, no-key:
  plan -> backtest -> inspect
```

第二段必須：

- 移除 Teralion auth environment variables。
- 由 CI/container 阻止 network。
- HTTP request count = 0。
- 只開啟 `TWSE/2330/2026-07-20` stream。
- 結果與相同 prepared inputs的 network-enabled local run byte-identical。

再次完整執行相同 config：

- source reuse且 sync request count = 0。
- cache hit時 source zstd decode count及 source JSON parse count都為 0。
- 刪除 cache後只由 local source重建，source revision不變。

## 17. Verification

至少驗證：

- config unknown/duplicate/secret/numeric/schema validation。
- plan read-only、actions、network requirement及 identity determinism。
- sync only-stage network boundary及 second-sync zero requests。
- per-page/daily-instrument `.json.zst`、dual checksum、streaming decode及
  no-uncompressed-file contract。
- verify no-key/offline及五種 source state。
- cache prepare hit/rebuild/corrupt/atomic publish。
- replay/backtest offline、selective stream及output existing-path rejection。
- market/limit orders、partial fill、fee/tax/P&L/reconciliation artifacts。
- inspect successful/failed/degraded/NotApplicable/Unavailable。
- exit code各 category及 secret redaction。
- run stage short-circuit及 atomic failed publication。
- acceptance-only fixture runner 與 release CLI boundary。

compact fixture 的 repository gate 見
[Release validation](../release/VALIDATION.md)；完整日 acceptance evidence 由 private
bundle 的外部報告保存。
