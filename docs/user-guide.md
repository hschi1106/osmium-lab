# 使用指南

本指南說明 `osmium` 的資料生命週期與 Rust strategy 整合方式。命令細節見 [CLI 參考](operations/cli.md)，設定欄位見 [RunConfig 設定參考](config-reference.md)。

## 1. 安裝與設定

從 repository 建立 binary：

```sh
cargo build --release -p osmium-cli
target/release/osmium version
```

建立設定：

```sh
osmium init --path config.yaml
```

編輯 `data.data_root`、`universe`、`strategy`、`simulation` 與 `instrument_economics` 後再驗證。`examples/config.yaml` 是完整結構範例；`examples/smoke.yaml` 搭配 `fixtures/smoke/` 用於 repository smoke flow。

API key 由 process environment 或 `.env` 的 `TERALION_API_KEY` 提供。不要將 key、cookie、bearer token 或 signed URL 寫入 YAML、Git 或 run artifacts。

## 2. 資料準備與離線執行

```sh
osmium config check --config config.yaml
osmium plan --config config.yaml
osmium data sync --config config.yaml
osmium data verify --config config.yaml
osmium cache prepare --config config.yaml
```

`config check` 驗證 schema、strategy registry、parameters、universe 與 economics。`plan` 比較設定與本地 source/cache 狀態，不下載資料。`data sync` 是唯一需要網路與 credential 的命令；後續命令都使用本地 artifact。

執行回播與回測：

```sh
osmium replay --config config.yaml
osmium backtest --config config.yaml --output runs/example
osmium inspect --run runs/example
```

`replay` 驗證 deterministic event merge 與 MarketState，不執行 simulation strategy。`backtest` 執行 strategy、orders、fills、費稅與帳務。`run` 會依 plan 串接 sync、cache 與 replay/backtest；plan 需要下載時會讀取 `TERALION_API_KEY`，需要完全控制網路副作用時請使用上方的分步命令。

output directory 必須不存在。已發布的 run artifacts 是 immutable evidence，不會被覆寫。

## 3. 本地資料管理

source、cache 與 run output 有不同生命週期：

- verified source revision 可跨回測重用，不因 cache 失效而重新下載。
- replay cache 綁定 source checksum 與版本，可刪除後由 `cache prepare` 離線重建。
- run output 綁定 effective config、plan、strategy 與 simulation，不作為下一次回測的輸入。

遇到資料問題時先執行：

```sh
osmium plan --config config.yaml
osmium data verify --config config.yaml
```

不要直接修改 `current.yaml`、published source revision、cache descriptor 或 run manifest。狀態與復原方式見 [本地資料](operations/local-data.md)。

## 4. 行情 TUI

```sh
osmium display --config config.yaml
```

`display` 使用和 replay 相同的已驗證 streams 與 `match_time` 時間軸，但不執行 strategy、simulation 或 artifact publication。

| 按鍵 | 行為 |
| --- | --- |
| ←／→ | 切換標的，不改變共用播放時間 |
| Space | 暫停或繼續 |
| +／=、- | 切換固定播放倍率 |
| R | 從頭播放並恢復 1.0x |
| Q | 離開 |

畫面提供 selected instrument、時間、狀態、倍率、價格、成交量、完整五檔與最近成交；不推導 queue、imbalance 或 trade delta。

## 5. Strategy 整合

### 執行模型

CLI 只載入已編譯進 binary 且加入 registry 的 Rust strategy。YAML 不會動態載入外部程式碼。新增 strategy 的流程：

1. 建立獨立 crate，可參考 `crates/example-strategy`。
2. 實作 `Strategy` 與 `StrategyFactory`，定義固定 identity 與 parameter schema。
3. 將 crate 加入 CLI dependency，並在 `compiled_strategy_registry()` 註冊 factory。
4. 重新編譯 binary，在 config 使用精確的 id 與 version。

generic runner 也能直接執行 `S: Strategy` 或 `Box<dyn Strategy>`；replay-only flow 使用 `strategy_api::run_strategy`，simulation flow 使用 `osmium_runner::run_multi_backtest`。

### Strategy 邊界

strategy 可以讀取：

- 目前 `DomainEvent`。
- event 套用後的 read-only `MarketStateView`。
- `TradingContext`、session phase 與 decision time。
- deterministic strategy feedback 與自身狀態。

strategy 不得修改 market state、event、source、cache 或 replay clock，也不能取得 next event、future state、network、wall clock、filesystem 或未記錄 randomness。來源 flags 的語意由 normalizer 與 `TradingContext` 提供，不由 strategy 解碼 raw Teralion JSON。

### 最小 skeleton

```rust
use strategy_api::{
    CanonicalParamsChecksum, Strategy, StrategyDeclaration,
    StrategyEventContext, StrategyExecutionError, StrategyIdentity,
    StrategyOutputSink,
};

pub struct Observer {
    identity: StrategyIdentity,
    declaration: StrategyDeclaration,
}

impl Strategy for Observer {
    fn identity(&self) -> &StrategyIdentity {
        &self.identity
    }

    fn canonical_params_checksum(&self) -> CanonicalParamsChecksum {
        CanonicalParamsChecksum::for_empty_params()
    }

    fn declaration(&self) -> StrategyDeclaration {
        self.declaration.clone()
    }

    fn on_event(
        &mut self,
        context: StrategyEventContext<'_>,
        output: &mut StrategyOutputSink,
    ) -> Result<(), StrategyExecutionError> {
        let _event = context.event();
        let _state = context.market_state();
        let _trading = context.trading();
        let _ = output;
        Ok(())
    }
}
```

strategy identity 應使用可回溯的 built artifact digest。parameter checksum 必須由版本化 deterministic bytes 計算，不可依賴 `Debug`、map insertion order 或 floating-point formatting。

### Order intent

order intent 需指定 universe 內的 instrument、side、quantity 與 `Market` 或 `Limit` order type。runner 會依 quantity unit、session、`TradingContext` 與 execution policy 驗證；intent 不保證成交。

replay-only sink 不提供 order capability。scheduled order 與 timer 也只在 execution plan 明確啟用相關能力時可用。

### 測試建議

- identity、parameter canonicalization 與 declaration。
- 相同 event sequence 的 output determinism。
- read-only state 與 no-look-ahead compile tests。
- quantity unit、price、universe 與 eligibility rejection。
- callback error／panic 與 failed run publication。
- order、fill 與 feedback 順序。

```sh
cargo test -p strategy-api
cargo test -p osmium-runner
```

## 6. 常見問題

### 自訂 strategy 顯示 not compiled into this binary

確認 crate 已成為 CLI dependency、factory 已加入 `compiled_strategy_registry()`、id/version 完全相符，並使用重新編譯的 binary。

### replay 沒有 orders 或 fills

`replay` 不執行 simulation strategy。請使用 `backtest` 或 `run`。

### sync 完成後仍無法 replay

依序執行 `data verify`、`cache prepare` 與 `plan`。錯誤會指出 source、cache 或 identity 不相容的範圍。

### cache 損壞或版本不相容

移除有問題的 derived cache 後執行 `cache prepare`。只要 verified source 完整，就不需要重新下載。
