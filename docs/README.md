# Osmium Lab 文件地圖

正式文件描述目前 release contract；設計取捨以 code、tests 與
[`traceability.yaml`](traceability.yaml) 對照。請依角色閱讀，避免從 provider 文件推導 core
語義，或從操作手冊推導架構 ownership。

## 第一次使用 Osmium

1. [Repository README](../README.md)：產品能力、限制與五分鐘 happy path。
2. [使用指南](user-guide.md)：從 build、資料準備到 backtest artifacts 的完整流程。
3. [RunConfig 參考](config-reference.md)：設定欄位、defaults 與 validation。
4. [CLI 參考](operations/cli.md)：command、flags、output 與 error contract。
5. [本地資料](operations/local-data.md)：source/cache layout、狀態與復原。

Release archive 使用者可先讀[獨立 quickstart](quickstart.md)；它只依賴 archive 內附的 binary、
example 與操作文件。

## Strategy developer

1. [使用指南：Strategy 開發](user-guide.md#strategy-開發)：compiled registration 與範例。
2. [執行與帳務模型](architecture/execution-model.md)：callbacks、orders、fills、latency 與 accounting。
3. [`example-strategy`](../crates/example-strategy/src/lib.rs)：canonical compiled example。
4. [授權](operations/licensing.md)：Strategy Linking Exception 與 core 邊界。

## Core／architecture developer

1. [產品需求](product-requirements.md)：scope 與 correctness firewall。
2. [架構總覽](architecture/overview.md)：元件 ownership 與依賴方向。
3. [資料流程](architecture/data-flow.md)：source、cache、lineage 與 artifacts。
4. [回播模型](architecture/replay-model.md)：DomainEvent、observation、auction 與 MarketState。
5. [執行與帳務模型](architecture/execution-model.md)：strategy、execution 與 reconciliation。
6. [ADR-0006](architecture/decisions/0006-external-strategies-and-cash-charges.md)：外部 strategy 與 cash charge 邊界。

## Provider／market-data developer

1. [Teralion adapter contract](interfaces/teralion.md)：network、credential、source 與 mapping identity。
2. [TWSE mapping](interfaces/twse.md)、[TPEx mapping](interfaces/tpex.md)、
   [TAIFEX mapping](interfaces/taifex.md)：market-specific wire evidence 與 reject 規則。
3. [資料流程](architecture/data-flow.md)：verified source 與 cache publication。
4. [回播模型](architecture/replay-model.md)：provider-neutral event semantics。

## Operations／release maintainer

1. [CLI 參考](operations/cli.md)與[本地資料](operations/local-data.md)。
2. [驗證](operations/validation.md)：repository、fixture、offline smoke 與 docs checks。
3. [發布](operations/release.md)：archive contract 與 release steps。
4. [支援](operations/support.md)與[授權](operations/licensing.md)。

共通詞彙見[詞彙表](glossary.md)。外部資料格式連結只作來源判讀參考；實際支援範圍由 current
normalizer、fixture 與 tests 固定。
