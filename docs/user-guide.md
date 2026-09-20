# 使用指南

本頁是從 repository build 到檢查 backtest artifacts 的唯一完整 tutorial。欄位定義見
[RunConfig 參考](config-reference.md)，精確 command contract 見 [CLI 參考](operations/cli.md)。

## 先決條件

- Rust `1.97.1`（`rust-toolchain.toml` 會自動選取）。
- Linux x86_64 或相容的自行建置環境。
- 第一次下載 source 時需要網路與 `TERALION_API_KEY`。
- 後續 verify、cache、replay、backtest 與 inspect 都可離線。

## 完整工作流程

### 1. Build

```sh
cargo build --release --locked -p osmium-cli --bin osmium
target/release/osmium version
```

輸入是目前 source tree 與 lockfile，產物是 `target/release/osmium`；不需網路（依賴已在本機時）。
可重跑且會增量 build。失敗表示 toolchain、dependency 或 compile 問題，不會變更市場資料。

### 2. 建立與編輯 config

```sh
target/release/osmium init --path config.yaml
# 或：cp examples/config.yaml config.yaml
```

設定至少描述 data root、日期與商品 universe、compiled strategy、replay/simulation policy、每商品
economics 與 output policy。`init` 只建立不存在的檔案，不使用網路；檔案已存在時會拒絕覆寫。
所有 exact numeric value（price、rate、amount、multiplier）以 YAML string 表示。

不要把 API key、cookie、bearer token 或 signed URL 寫進 config。Credential 只由 process
environment 或 repository root `.env` 的 `TERALION_API_KEY` 提供。

### 3. Config check

```sh
target/release/osmium config check --config config.yaml
```

輸入是 YAML 與目前 binary 的 strategy registry；它驗證 schema、unknown/secret fields、strategy
identity/parameters、universe、sessions、economics 與 simulation 組合。只讀、離線、可重跑。失敗是
設定契約錯誤，尚未建立 plan 或資料 artifact。

### 4. Plan

```sh
target/release/osmium plan --config config.yaml
```

Planner 將 validated config、session profiles 與本地 source/cache catalog 物化成 frozen execution
plan，列出各 partition 要 reuse、download、resume 或 rebuild 的 action。命令只讀且不下載；輸出可
重跑，但本地 artifact 狀態改變時 plan identity/action 也可能改變。失敗代表 config 或本地
source/cache 狀態無法形成一致 plan。

### 5. Data sync

```sh
export TERALION_API_KEY='...'
target/release/osmium data sync --config config.yaml
```

輸入是 plan、credential 與 provider archive。只對缺少的 partition 使用網路，寫入 staging，驗證
cursor/page/checksum 後 atomic publish immutable source revision；complete source 會 reuse。中斷後僅
相容 staging 可 resume。失敗時不會把不完整資料發布成 complete；依診斷修正 credential、網路或
source integrity 後可重跑。

### 6. Data verify

```sh
unset TERALION_API_KEY
target/release/osmium data verify --config config.yaml
```

輸入是本地 source partition、manifest 與 pages；輸出是 verification summary，不寫入、不需網路。
它可反覆執行。失敗代表 source 缺少、不完整、identity 不符或 checksum 損壞；verify 不會自動修補。

### 7. Cache prepare

```sh
target/release/osmium cache prepare --config config.yaml
```

輸入是 verified source、normalizer mapping 與 current event/ordering versions；產物是 provider-neutral
replay cache 與 descriptor。Compatible cache 會 reuse；missing/stale cache 由 source 離線重建，因而
可安全重跑。Normalization、mapping 或 canonical validation 失敗時不發布 cache；source 本身不受影響。

### 8. Replay

```sh
target/release/osmium replay --config config.yaml
```

輸入是 frozen plan 與相容 cache streams；它只開 explicit universe，依 `match_time` 與版本化
tie-break 回播，原子更新 MarketState，輸出 event/final-state checksum summary。不執行 strategy、
orders 或 accounting，不寫 source/cache，也不需網路。相同輸入與版本應得到相同 checksum。

### 9. Backtest

```sh
target/release/osmium backtest \
  --config config.yaml \
  --output runs/example
```

輸入是相同 replay path、compiled strategy 與 simulation/economics config；產物是 immutable run
directory。它依 policy 執行 callbacks、orders、fills、feedback、fees/taxes、ledger、marking 與
reconciliation。`--output` 必須不存在；重跑請用新目錄。失敗表示 replay、strategy、simulation、
accounting 或 publication 問題，既有成功 run 不會被覆寫。

### 10. Inspect

```sh
target/release/osmium inspect --run runs/example
```

`inspect` 讀取 `run-manifest.yaml`，確認 manifest version 與列出的 artifact checksum，再摘要 status、
event/order/fill counts。它不讀 config/source、不載入 strategy、不重跑回測，也不使用網路。Checksum
或 manifest 錯誤代表 run evidence 被修改、損壞或版本不相容。

## `osmium run` convenience flow

```sh
target/release/osmium run --config config.yaml --output runs/example
```

`run` 依 current plan 串接必要的 sync、verify/cache preparation、replay 與 backtest。若 source
已 complete，整條流程可離線；若 plan 要求 download，命令會需要網路與 credential。需要精確控制
副作用或定位失敗階段時，使用上方分步命令。

## Backtest artifacts

所有模式都發布以下類別：

| 類別 | Files | 用途 |
| --- | --- | --- |
| Identity | `run-manifest.yaml`, `effective-config.yaml`, `execution-plan.yaml`, `strategy.json` | run/schema/model/strategy identity 與 artifact checksum index |
| Lineage | `data-lineage.yaml`, `cache-lineage.yaml` | source revision 與 cache identity |
| Replay evidence | `event-stream.blake3`, `final-state.blake3`, `replay-summary.json` | event stream 與 reducer 結果 |
| Strategy/execution evidence | `strategy-output.bin`, `orders.bin`, `fills.bin` 及各自 `.blake3` | callback outputs、orders 與 fills |
| Accounting result | `ledger.bin`, `positions.yaml`, `performance.yaml`, `run-summary.yaml` | ledger、positions、P&L 與人類可讀摘要 |
| Diagnostics | `warnings.yaml` | run warnings |

`scheduled_visible_depth_v1` 另發布 `execution-trace.bin`、`fill-costs.json`、`cash-charges.json` 與各自
checksum；subsequent-event mode 不會偽造這些 control-time artifacts。Identity 回答「用哪個版本與
設定執行」，lineage 回答「資料從哪裡來」，execution evidence 回答「發生哪些決策與成交」，
accounting result 回答「現金、部位與損益如何結算」。Binary 檔是 canonical evidence，不是手改介面。

## Strategy 開發

Osmium 不載入 runtime strategy plugin。Strategy crate 必須編譯進 binary，factory 必須註冊到
`StrategyRegistry`；canonical example 是
[`crates/example-strategy`](../crates/example-strategy/src/lib.rs)。

### Identity、schema 與註冊

1. 實作 `StrategyFactory`，提供固定 `StrategyDefinition` 與 `StrategyParameterSchema`。
2. `build` 只使用 validated parameters、explicit universe 與 sessions；不得 I/O、讀 environment
   或擴張 universe。
3. 實作 `Strategy` 的 identity、canonical parameter checksum、declaration 與 callbacks。
4. 將 factory 加入 CLI 的 compiled registry，重新 build，再以 config 的精確 `id`／`version` 選取。

Duplicate registration、unknown id、version/schema mismatch、unknown parameter 或 declaration 與
effective universe/session 不符都會在執行前拒絕。External binary 可透過
`osmium_cli::run_with_registry_provider` 注入額外 compiled factories，但 CLI contract 與 orchestration
仍由 Osmium 擁有。

### Lifecycle 與可見資訊

```text
resolve factory + parameters
  -> initialize
  -> event / timer callbacks
  -> atomically commit callback output
  -> order/fill feedback callbacks
  -> finalize
```

`on_event` 讀取目前 `DomainEvent`、event 套用後的 read-only `MarketStateView`、deterministically
ordered universe states、`TradingContext`、session 與 decision time。`on_timer`、`on_feedback` 只在
對應 capability/transition 發生時呼叫。`finalize` 取得 replay clock 與 final states。API 不提供
mutable market state、next event、future state、network、filesystem、wall clock 或未記錄 randomness。

Callback 可輸出 indicators；simulation runner 另開 order intents，scheduled runner 才開 scheduled
requests、timers 與 cash charges。Callback 成功後才整批提交，error/panic 不留下 partial output。

### Orders 與 scheduled requests

`OrderIntent` 指定 universe instrument、buy/sell、positive typed quantity，以及 market 或帶 exact
limit price 的 limit order；目前 time-in-force 固定為 `Day`。Order 不能由產生它的 origin event
成交，最早使用後續 eligible evidence。

Scheduled request 另指定 stable client id、可選 batch id、`activate_at`、可選 `expire_at` 與 policy：
visible depth at activation、visible depth until expiry、first auction cross，或由 immutable reference
支持的 settlement-at-activation。Timers 與 cash charges 也只屬 scheduled capability。Execution 與
accounting 的完整語義見[執行與帳務模型](architecture/execution-model.md)。

## 常見問題

### Strategy 找不到

確認 factory 已註冊、config id/version 完全相符，且使用重新編譯的 binary；YAML 路徑不會動態
載入 strategy。

### Replay 沒有 orders 或 fills

`replay` 刻意不執行 simulation strategy；使用 `backtest` 或帶 `--output` 的 `run`。

### Sync 後仍無法 replay

依序執行 `data verify`、`cache prepare` 與 `plan`。診斷會區分 source、mapping/cache identity 或
event validation 問題。

### Cache 損壞或 stale

只移除診斷指出的 derived cache，再執行 `cache prepare`。Verified source 完整時不需重新下載；
不要手改 descriptor 或 `current.yaml`。
