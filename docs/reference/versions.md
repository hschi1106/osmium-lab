# 版本

結果依賴哪些版本、各版本何時必須更新。`osmium version` 印出目前的值；每個 run 的 `run.json` 記錄
它使用的值（[輸出規格](artifacts.md)第 2 節）。

| 名稱 | `run.json` 的欄位 | 定義於 | 何時更新 |
| --- | --- | --- | --- |
| osmium 版本 | `versions.osmium` | workspace `Cargo.toml` | 每次 release |
| marketdata schema | `versions.md_schema` | `osmium_md::SCHEMA_VERSION` | 支援的 `md.schema_version` 改變 |
| decoder | `versions.decoder` | `osmium_md::DECODER_VERSION` | 相同的 marketdata 檔案解碼出不同的訊息或參考資料（[marketdata](marketdata.md)）|
| ordering | `versions.ordering` | `osmium_replay::ORDERING_VERSION` | 訊息的全序或時間線的順序改變（[回放規格](replay.md)）|
| matching rules | `versions.matching_rules` | `osmium_matching::RULES_VERSION` | 相同的訊息與委託產生不同的回報（[撮合規格](matching.md)）|
| ledger rules | `versions.ledger_rules` | `osmium_ledger::RULES_VERSION` | 相同的回報與參考資料產生不同的帳（[帳務規格](accounting.md)）|
| output | `output_version` | `osmium_runner::OUTPUT_VERSION` | 輸出檔案的名稱、欄位或意義改變（[輸出規格](artifacts.md)）|
| 策略版本 | `strategy.version` | 策略的 `StrategyEntry` | 策略的決策邏輯改變（[策略介面規格](strategy.md)第 6 節）|

## 規則

- osmium 只讀 `md.schema_version = "1"` 的 marketdata；其他版本在讀取時失敗，不嘗試相容
  （[marketdata](marketdata.md)）。
- 會改變結果的修改必須在同一個 commit 更新對應的版本常數，並記錄在 `CHANGELOG.md`。只改變速度、
  記憶體或錯誤訊息的修改不更新版本。
- 兩個 run 的結果可以直接比較的條件：`versions`、`output_version`、`strategy`、`config_blake3`、
  `reference` 與 `days[].files` 全部相同。此時所有輸出檔案的 `blake3` 也必須相同；不同就是決定性
  的缺陷。
- marketdata 的資料內容以 manifest identity（`git_commit`、`code_hash`、`inputs_hash`）識別，不是
  以 schema 版本：schema 相同而內容重新產生過的資料會有不同的 identity。
