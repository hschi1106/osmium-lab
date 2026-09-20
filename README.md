# Osmium Lab

<p align="center">
  <img src="assets/logo-readme.png" alt="Osmium Lab" width="320">
</p>

<p align="center">台灣市場歷史行情回播與策略回測平台</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-2024-orange" alt="Rust 2024">
  <img src="https://img.shields.io/badge/Market-TWSE%20%7C%20TPEx%20%7C%20TAIFEX-0a7ea4" alt="Taiwan Markets">
  <img src="https://img.shields.io/badge/Release-0.1.0-brightgreen" alt="Release 0.1.0">
  <img src="https://img.shields.io/badge/License-AGPL--3.0--only-red" alt="AGPL-3.0-only License">
</p>

Osmium Lab 是 Rust 實作、可接入不同歷史行情供應商的 market replay 與 backtesting
platform，目前內建 Teralion adapter。它將下載、驗證、正規化、deterministic replay、編譯期
Rust strategy、成交模擬、帳務與結果 artifacts 串成可重現的離線工作流程。

## 能力與邊界

目前支援 TWSE／TPEx 股票與權證、TAIFEX 期貨與選擇權，以及：

- verified historical source、可重建 replay cache 與 selective multi-stream replay；
- 完整五檔 snapshot replacement、成交、auction／volatility-interruption evidence；
- multi-instrument、multi-date deterministic replay 與 backtest；
- 編譯並註冊的 Rust strategy、market／limit Day order、partial fill；
- subsequent-event 與 opt-in scheduled visible-depth execution；
- market-data／order latency、slippage、fee、tax、multiplier、cash、position 與 P&L；
- event、state、ledger 與 run artifact checksums。

資料精度就是模型上限。Osmium 不提供 live trading、完整交易所撮合、逐筆委託簿重建、真實
queue position、hidden liquidity、aggressor 或 source sequence 推定，也不重算交易所 trigger
或完整處置證券撮合規則。盤中／盤後零股、盤後定價與鉅額交易不在目前範圍。Strategy 必須
編譯並註冊進 binary；目前沒有 runtime plugin。

## Mental model

```text
Provider wire
    ↓  adapter / normalizer
Verified source → versioned DomainEvent cache
                         ↓
              deterministic replay
                         ↓
                    MarketState
                         ↓ read-only
                      Strategy
                         ↓ intents
              Execution → Accounting
                         ↓
                  Run artifacts
```

`match_time` 是唯一 replay time；同時間事件使用版本化 tie-break。Verified source 是可跨回測
重用的 immutable fact，replay cache 是可刪除、可離線重建的衍生物。Provider wire type 不會
進入 replay core；strategy 只能讀取目前與過去已提交的狀態。

## 五分鐘開始

需要 Rust `1.97.1`；repository 的 toolchain 檔會選擇正確版本。

```sh
cargo build --release --locked -p osmium-cli --bin osmium
target/release/osmium version
cp examples/config.yaml config.yaml
target/release/osmium config check --config config.yaml
target/release/osmium plan --config config.yaml
```

調整 `config.yaml` 的 `data_root`、universe、strategy 與 economics。第一次取得資料時，以
process environment 或 repository root 的 `.env` 提供 `TERALION_API_KEY`：

```sh
target/release/osmium data sync --config config.yaml
target/release/osmium data verify --config config.yaml
target/release/osmium cache prepare --config config.yaml
```

此後可離線執行：

```sh
target/release/osmium replay --config config.yaml
target/release/osmium backtest --config config.yaml --output runs/example
target/release/osmium inspect --run runs/example
```

`--output` 必須是尚不存在的目錄。`osmium run --config config.yaml --output runs/example`
可串起規劃、必要資料準備與回測；若 plan 要求下載，它會使用網路與 credential。完整步驟、
各命令輸入輸出與失敗處理見[使用指南](docs/user-guide.md)。

## Strategy 入口

新增 strategy 時實作 `Strategy` 與 `StrategyFactory`，定義 identity、parameter schema、universe
與 sessions，加入 `StrategyRegistry` 後重新編譯 CLI。Canonical example 是
[`crates/example-strategy`](crates/example-strategy/src/lib.rs)；完整 lifecycle 與註冊方式見
[Strategy 開發](docs/user-guide.md#strategy-開發)。

## 文件

- [文件地圖](docs/README.md)：依使用者、strategy、core、provider 與 release 維護者分流。
- [使用指南](docs/user-guide.md)：從 build 到 inspect 的完整 tutorial 與 artifacts 說明。
- [設定參考](docs/config-reference.md)：`config_version: 3` schema、defaults 與 validation。
- [CLI 參考](docs/operations/cli.md)：commands、flags、output 與 error contract。
- [架構總覽](docs/architecture/overview.md)：元件 ownership 與依賴邊界。
- [回播模型](docs/architecture/replay-model.md)：event、observation、auction 與 MarketState semantics。
- [執行與帳務](docs/architecture/execution-model.md)：orders、fills、latency、fees 與 P&L。
- [產品需求](docs/product-requirements.md)：產品範圍與驗收基準。

Release archive 內另附可獨立閱讀的 [`docs/quickstart.md`](docs/quickstart.md)，供沒有 repository
導覽內容的 binary 使用者快速啟動。

## 授權

Core 以 `AGPL-3.0-only` 發布。獨立撰寫、只透過公開 `strategy-api` 連結的 strategy 可適用
Strategy Linking Exception；細節見[授權文件](docs/operations/licensing.md)與 [LICENSE](LICENSE)。
外部資料仍受其供應商條款約束。
