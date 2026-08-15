# osmium-lab 文件

本目錄記錄 `osmium-lab` 的產品範圍、現行架構、資料介面與操作方式。文件以目前發布版本的實作為準，不記錄開發里程碑或已淘汰的設計過程。

## 開始使用

- [快速開始](quickstart.md)：從設定檢查到離線回測的最短流程。
- [使用指南](user-guide.md)：資料生命週期、策略整合與常見問題。
- [設定參考](config-reference.md)：`config_version: 2` 的欄位與驗證規則。
- [CLI 參考](operations/cli.md)：命令、副作用、輸出格式與 exit status。

## 產品與架構

- [產品需求](product-requirements.md)：產品範圍、必要行為與驗收原則；為本專案的需求基準。
- [架構總覽](architecture/overview.md)：元件責任、依賴方向與線上／離線邊界。
- [資料流程與儲存](architecture/data-flow.md)：source、cache 與 run artifact 的生命週期。
- [回播模型](architecture/replay-model.md)：domain event、排序、session、MarketState 與策略 callback。
- [模擬與帳務](architecture/execution-model.md)：委託、成交證據、排程模型與帳務限制。

## 資料介面

- [Teralion Feed Archive](interfaces/teralion.md)
- [TWSE](interfaces/twse.md)
- [TPEx](interfaces/tpex.md)
- [TAIFEX](interfaces/taifex.md)

## 操作與維護

- [本地資料](operations/local-data.md)：目錄結構、狀態、檢查與復原。
- [驗證](operations/validation.md)：repository checks、smoke test 與 release archive 驗證。
- [發布](operations/release.md)：發布內容與部署前提。
- [支援政策](operations/support.md)：支援邊界與問題分級。
- [追溯矩陣](traceability.yaml)：需求、實作與驗證入口。
- [詞彙表](glossary.md)

外部資料格式連結是介面判讀參考；實際支援範圍仍以 normalizer、fixture 與測試固定的契約為準。
