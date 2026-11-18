# Osmium Lab 文件地圖

每個事實只寫在一處；其他文件用連結指過去。Code 與 tests 是 executable truth。

## 使用者：使用手冊（`guide/`）

- [安裝與第一個回測](guide/getting-started.md)：編譯、設定檔、`check` 與 `run`、看懂輸出。
- [寫策略](guide/writing-a-strategy.md)：一個完整的例子，從 trait 到自己的執行檔。
- [執行](guide/running.md)：指令、選 feed 與商品、延遲、帳戶與費用、公司行動、時間與記憶體。
- [讀結果](guide/results.md)：每個輸出檔怎麼讀、成交依據、驗算、比較兩個 run、結果的限制。
- [疑難排解](guide/troubleshooting.md)：錯誤訊息的原因與做法。

## 精確規格（`reference/`）

- [marketdata](reference/marketdata.md)：依賴的 schema 版本、讀取與一致性檢查、訊息 mapping。
- [回放](reference/replay.md)：訊息的全序、時間線、交易日流程、決定性。
- [撮合模擬](reference/matching.md)：交易模式、委託檢查、逐筆撮合、集合競價、排隊估計、已知限制。
- [帳務](reference/accounting.md)：手續費、證交稅、交割、當沖、融資融券、維持率、公司行動、損益。
- [策略介面](reference/strategy.md)：呼叫順序、`Context`、行情與帳戶視圖、參數與註冊。
- [設定](reference/config.md)：設定檔的每個 section、預設值與檢查。
- [輸出](reference/artifacts.md)：`run.json` 與各 CSV 的欄位。
- [版本](reference/versions.md)：結果依賴的版本與更新規則。

實作狀態見[架構](design/architecture.md#實作狀態)。

## 設計（`design/`）

- [架構](design/architecture.md)：資料流、ownership、不變條件、實作狀態。
- 決策紀錄：
  [ADR-0008 全市場撮合模擬](design/decisions/0008-full-market-matching-simulation.md)、
  [ADR-0007 marketdata-native](design/decisions/0007-marketdata-native.md)、
  [ADR-0006 外部策略與 cash charge](design/decisions/0006-external-strategies-and-cash-charges.md)。
- [規則研究](design/research/README.md)：交易所與主管機關規則的出處、生效日期、信心等級；規格的依據。

## 開發（`development/`）

- [驗證](development/validation.md)：quality gates 與真實資料測試。

## Agent

- [`AGENTS.md`](../AGENTS.md)：repository 規則。
- `.agents/skills/osmium-engineering`：修改程式。
- `.agents/skills/osmium-backtest`：使用系統跑回測。
