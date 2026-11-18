# Osmium Lab

<p align="center">
  <img src="assets/logo-readme.png" alt="Osmium Lab" width="320">
</p>

<p align="center">台灣市場歷史行情回播與策略回測平台</p>

Osmium Lab 是 Rust 實作的台股回測平台。唯一資料源是 marketdata 資料表（`/srv/md/v1`，parquet）：
osmium 直接讀取交易所的逐筆成交與五檔揭示，依交易所序號回放全市場，讓策略的模擬委託對真實行情
撮合（逐筆交易與集合競價），並以整數元記帳（費稅、T+2 交割、當沖、融資融券、公司行動）。

> 狀態：全市場撮合模擬（[ADR-0008](docs/design/decisions/0008-full-market-matching-simulation.md)）
> 的各層都已實作，見[架構的實作狀態](docs/design/architecture.md#實作狀態)。期貨與選擇權尚未
> 支援。

## 快速開始

```sh
cargo build --release
./target/release/osmium strategies          # 內建的範例策略
./target/release/osmium check run.toml      # 檢查設定，列出 run 會讀取的資料
./target/release/osmium run run.toml --out ~/osmium-runs/run-1
```

完整的步驟見[安裝與第一個回測](docs/guide/getting-started.md)；自己的策略見
[寫策略](docs/guide/writing-a-strategy.md)。

## 設計原則

- **正確性優先**：只使用 marketdata 能證明的事實。撮合只用已經到達交易所的訊息，策略只看得到
  已經送達它的行情與回報；模擬委託的成交不超過資料顯示的量；缺少的事實不外推，依規格拒絕、
  停止或記錄診斷。
- **帳務精確**：價格、股數與金額都是整數，比率是十進位數，不使用浮點數；每一筆金額都有分錄
  可以追溯。
- **可重現**：run 記錄讀到的每個 marketdata 檔案的 manifest identity 與所有影響結果的版本；
  相同的資料、版本與設定產生位元組相同的輸出。
- **速度**：marketdata 已是正規化、排序好的欄式資料，osmium 直接讀取；速度與記憶體的最佳化
  不得改變輸出。

## 文件

從[文件地圖](docs/README.md)開始：使用手冊（`docs/guide/`）、規格（`docs/reference/`）、設計
（`docs/design/`）。Agent 的流程在 `AGENTS.md` 與 `.agents/skills/`。

## 開發

```sh
cargo test --workspace
```

完整 gate 見[驗證](docs/development/validation.md)。
