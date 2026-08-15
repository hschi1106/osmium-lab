# CLI 參考

目前 binary 為 `osmium 0.1.0`，CLI contract version `4`，要求 `config_version: 2`。

## 1. 命令總覽

```text
osmium version
osmium init [--path <config.yaml>]
osmium config check --config <file>
osmium plan --config <file>
osmium data sync|verify --config <file>
osmium cache prepare --config <file>
osmium replay --config <file>
osmium backtest --config <file> --output <new-directory>
osmium run --config <file> [--output <new-directory>]
osmium display --config <file>
osmium inspect --run <run-directory>
```

非互動命令支援全域選項：

- `--format human|json`
- `--quiet`
- `--no-color`

`display` 是互動式 TUI，不接受這三個選項。

## 2. 命令行為

| 命令 | 網路 | 寫入 | 結果 |
| --- | --- | --- | --- |
| `version` | 否 | 否 | product 與 schema／model versions |
| `init` | 否 | 新建 config skeleton | 既有檔案時拒絕 |
| `config check` | 否 | 否 | schema、strategy、universe 與 economics 驗證 |
| `plan` | 否 | 否 | plan identity、network requirement 與 partition actions |
| `data sync` | 是，缺資料時 | source staging／revision | reuse 或 atomic publish summary |
| `data verify` | 否 | 否 | 每個 current source revision 的 checksum／record summary |
| `cache prepare` | 否 | replay cache | reuse 或由 complete source rebuild |
| `replay` | 否 | 否 | event count、event 與 final-state checksum |
| `backtest` | 否 | 新建 run directory | strategy、simulation、accounting 與 artifacts |
| `run` | 視 plan 而定 | source/cache；有 `--output` 時另建 run | sync → cache → replay/backtest |
| `display` | 否 | 否 | 只讀行情 TUI |
| `inspect` | 否 | 否 | run status、event/order/fill count |

`backtest` 不會自動下載或建立 cache；source/cache 未準備好時會失敗。`run` 是頂層 orchestration：plan 需要網路時會執行 sync，因此可能讀取 `TERALION_API_KEY`；省略 `--output` 時最後執行 replay，提供 `--output` 時執行 backtest。

## 3. 建議流程

可控的分步流程：

```sh
osmium config check --config config.yaml
osmium plan --config config.yaml
osmium data sync --config config.yaml
osmium data verify --config config.yaml
osmium cache prepare --config config.yaml
osmium backtest --config config.yaml --output runs/example
osmium inspect --run runs/example
```

已了解 plan 副作用時，可使用：

```sh
osmium run --config config.yaml --output runs/example
```

## 4. 設定與 plan

config 解析會先拒絕 unknown/secret fields，再解析 compiled strategy factory 與 parameters，驗證 strategy declaration、universe、session profile、instrument reference、economics 與 simulation policy。失敗發生在 network、cache stream 或 output staging 之前。

`plan` 會列出每個 partition 的 `source_action` 與 `cache_action`，並回報 `network_requirement`。plan identity 由 effective config、strategy metadata、session plan、source/cache policy、simulation 與版本集合計算。

## 5. Data sync 與 verify

`data sync` 先讀取本地 plan。所有 source 都可 reuse 時不讀取 API key，也不發出 HTTP request。需要下載時從 process environment 或 `.env` 讀取 `TERALION_API_KEY`，依序取得 coverage、range、daily instrument 與完整 tick cursor chain，完成 staging 後 atomic publish。

`data verify` 只讀 `current` revision，檢查 manifest、payload、checksums 與 record count。它不下載、不修補、不重建 cache。

## 6. Cache 與 replay

`cache prepare` 對每個 partition reuse compatible cache，或由 complete source 正規化並建立新 cache。缺少 complete source 時拒絕。

`replay` 只接受 plan 已綁定的 cache streams。它不執行 strategy simulation，也不建立 run directory；輸出 event count、canonical event checksum 與 final-state checksum。

## 7. Backtest 與 artifacts

`backtest --output` 需要不存在的目錄。執行內容為：

```text
load config / plan
-> open selected cache streams
-> replay and reduce state
-> invoke compiled strategy
-> simulate orders and fills
-> account and reconcile
-> publish immutable artifacts
```

run artifacts 保存 manifest、effective config checksum、plan identity、strategy metadata、source/cache lineage、warnings、strategy outputs、orders、fills、positions、performance 與 checksums。若 callback、simulation、accounting、checksum 或 publication 失敗，結果不得標示為 success。

`inspect` 只驗證與摘要既有 run directory，不存取 config、source、Teralion 或 strategy runtime。

## 8. TUI

`display` 需要 compatible source/cache，使用和 replay 相同的 ordered events 與 MarketState reducer。按鍵與畫面內容見 [使用指南](../user-guide.md#4-行情-tui)。TUI 不發布 run artifacts，也不提供 `--format json`。

## 9. 輸出格式

human success output 使用 `key=value` 與 record lines。JSON success envelope：

```json
{
  "schema_version": 1,
  "status": "success",
  "command": "plan",
  "result": { "fields": {}, "records": [] }
}
```

JSON error envelope：

```json
{
  "schema_version": 1,
  "status": "error",
  "command": "replay",
  "error": { "category": "replay", "code": 30, "message": "..." }
}
```

`--quiet` 只隱藏成功輸出；錯誤仍輸出至 stderr。checksum 與大型 exact integer 在 JSON 中保持 string，避免精度遺失。

## 10. Exit status

| Code | Category | 意義 |
| ---: | --- | --- |
| 0 | success | 命令完整成功或顯示 help |
| 1 | internal | I/O 或未分類的內部錯誤 |
| 2 | usage | command syntax 或 option 錯誤 |
| 10 | config | schema、strategy、universe 或 economics 錯誤 |
| 20 | source | credential、transport、sync 或 source 錯誤 |
| 21 | cache | cache 缺少、建立或讀取錯誤 |
| 30 | replay | event、ordering、state 或 TUI replay 錯誤 |
| 40 | simulation | order、fill、accounting 或 publication 錯誤 |
| 50 | integrity | artifact checksum 或 reconciliation 錯誤 |

自動化應使用 exit code 與 JSON `error.category`，不要解析人類可讀訊息判斷類別。
