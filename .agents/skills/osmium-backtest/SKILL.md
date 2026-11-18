---
name: osmium-backtest
description: >
  Use to run backtests with Osmium: writing a strategy against the strategy
  API, writing a run configuration, running `osmium check` and `osmium run`,
  reading and analysing the output files, and diagnosing a failed or
  surprising run. Not for changing Osmium itself (use osmium-engineering).
---

# Osmium Backtest

使用 osmium 跑回測的流程。規則不在這裡：每一步都連到擁有它的文件，數字與行為以文件和實際輸出
為準，不要憑記憶回答。

## 文件

```text
docs/guide/getting-started.md      編譯、第一個 run、輸出長什麼樣
docs/guide/writing-a-strategy.md   策略怎麼寫（完整例子：crates/osmium-cli/examples/breakout.rs）
docs/guide/running.md              指令、選 feed／商品／延遲／費率、公司行動、資源
docs/guide/results.md              每個輸出檔怎麼讀、成交依據、比較兩個 run
docs/guide/troubleshooting.md      錯誤訊息 → 原因與做法
docs/reference/config.md           設定檔每個 key 的定義與預設值
docs/reference/strategy.md         策略介面的精確規則
docs/reference/artifacts.md        輸出檔每一欄的定義
docs/reference/matching.md         委託怎麼成交；第 10 節是限制
docs/reference/accounting.md       費稅、交割、當沖、信用交易、公司行動；第 16 節是限制
```

## 流程

```text
釐清問題 → 策略（既有或新寫）→ 設定檔 → osmium check → 小範圍 run → 檢查輸出 → 放大範圍 → 回報
```

1. **釐清**：哪些商品、哪段期間、整股或零股、交易類別（現股／當沖／融資融券）、延遲與費率。使用者
   沒有給延遲或費率時要問，或明確說出你採用的假設；延遲沒有預設值。
2. **策略**：`<binary> strategies` 列出執行檔登記的策略。新策略依 `writing-a-strategy.md`，從
   `breakout.rs` 開始改；放在使用者自己的 crate 或 `crates/osmium-cli/examples/`。決策邏輯改了就
   更新 `StrategyEntry::version`。策略不得讀檔、讀系統時間或使用亂數。
3. **設定檔**：從 `docs/reference/config.md` 第 2 節的最小設定開始，只加需要的 key。帶小數的值寫成
   文字。持有或可能持有的商品都要在 `data.symbols`（或省略 `symbols` 回放整個 feed）。
4. **`check`**：每次改設定都先跑。`missing` 不是 `none` 時先處理。
5. **小範圍 run**：一兩檔商品、幾個交易日。逐筆讀 `order_events.csv`，確認每一筆委託的結果符合
   策略的意圖，再放大。全市場或長期間的 run 先依 `running.md` 第 6 節估計時間與記憶體。
6. **檢查輸出**（每個 run 都要做）：
   - 目錄裡有 `run.json`。
   - `diagnostics.csv`：每一種 `kind` 都看過，判斷是否影響結論。
   - `order_events.csv`：`Rejected` 的原因分布；成交依據 `basis` 的分布。
   - `entries.csv`／`realized.csv`：有沒有 `approx = true`。
   - `daily.csv`：`equity` 是績效；`cash` 只含已交割金額。
7. **回報**：設定的假設（延遲、費率、`cash_check`、`auction_marginal`、`missing_terms`）、期間與
   商品、期末權益與交易次數、成交依據的組成、所有診斷與近似、適用的已知限制（`results.md` 第 8
   節）。附上輸出目錄的路徑。不要只報損益。

## 規則

- **只讀 marketdata**（`data.md_root`，預設 `/srv/md/v1`）。輸出目錄放在 repository 與 marketdata
  之外；每個 run 一個新目錄，osmium 不覆寫既有結果。
- 用 `--release` 編譯。
- run 失敗時讀 stderr 的 `error:` 訊息，查 `troubleshooting.md`；不要用調整設定來掩蓋錯誤（例如
  未經使用者同意就把 `missing_terms` 改成 `"approximate"`、`cash_check` 改成 `"none"`）。這些選擇
  會改變結果的意義，要讓使用者決定並寫進回報。
- 結果是「不影響市場」假設下的保守估計。委託量相對於市場成交量不小、或 `PassiveQueue` 佔比很高
  時，在回報中明確指出。
- 比較兩個 run 之前先比對 `run.json` 的 `versions`、`strategy`、`config_blake3`、`reference`、
  `days[].files`。
- 懷疑 osmium 的行為有誤時，以 `trigger_feed`／`trigger_seq` 回 marketdata 查證那一則訊息，再對照
  規格；確認是缺陷才改用 `osmium-engineering`。

## 常用指令

```sh
cargo build --release
./target/release/osmium strategies
./target/release/osmium check run.toml
./target/release/osmium run run.toml --out <outside-the-repo>/run-1
cargo run --release -p osmium-cli --example breakout -- run run.toml --out <dir>
```
