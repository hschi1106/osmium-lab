# Osmium Lab 文件地圖

Domain truth 以本目錄的 canonical docs 為準；code 與 tests 是 executable truth。

> 架構已改為 marketdata-native（見 [ADR-0007](architecture/decisions/0007-marketdata-native.md)）；
> CLI 與 runner 串接仍在重建中。

## Core／architecture developer

1. [ADR-0007：marketdata-native 架構](architecture/decisions/0007-marketdata-native.md)
2. [產品需求](product-requirements.md)：scope 與 correctness firewall。
3. [架構總覽](architecture/overview.md)：元件 ownership 與依賴方向。
4. [回播模型](architecture/replay-model.md)：event、ordering、auction 與 MarketState。
5. [marketdata 介面](interfaces/marketdata.md)：`osmium-md` 讀取、驗證與 event mapping。
6. [執行與帳務模型](architecture/execution-model.md)：strategy、execution 與 reconciliation。
7. [ADR-0006](architecture/decisions/0006-external-strategies-and-cash-charges.md)：外部 strategy 與 cash charge 邊界。

## 驗證

[驗證](operations/validation.md)。共通詞彙見[詞彙表](glossary.md)。
