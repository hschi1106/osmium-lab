# User guide

這份文件是給使用 `osmium` 的 internal user 與 strategy author。它說明如何準備一份
`config_version: 2` config、完成資料生命週期，以及如何依目前 Rust `Strategy` API
撰寫策略。

先知道一個重要邊界：release CLI 只會執行已編譯進該 binary 且明確加入 registry 的
Rust strategy。這不是 runtime plugin 系統；只有修改 crate dependency 與 registry entry、
重新編譯 `osmium` 後，YAML 才能選擇新的 `strategy.id + version`。

## 1. 準備 `osmium`

使用 internal binary archive 時，確認 binary 已在 `PATH`：

```sh
osmium version
osmium --help
```

從 repository 執行或建立 release binary：

```sh
cargo build --release -p osmium-cli
target/release/osmium version
```

資料下載需要在 `.env` 或 process environment 設定 `TERALION_API_KEY`。不要把 key、
cookie、bearer token 或 signed URL 寫入 YAML、Git 或 run artifact。

## 2. 建立自己的 config

以完整的 release example 為起點，複製到 repository 外的 user-owned path，並修改副本：

```sh
cp examples/config.yaml my-config.yaml
```

`examples/config.yaml` 是保留的 user-facing template；它不是完整日 fixture，也不會
自動提供 `data_root` 的資料。若只想驗證安裝與 CI flow，使用 `examples/smoke.yaml`
搭配 `fixtures/smoke/`。

### 2.1 必要區塊

| 區塊 | 使用者要設定的內容 |
| --- | --- |
| `config_version` | 固定為 `2`；v1 不提供 migration |
| `data` | `source: teralion`、user-owned `data_root`、source/cache policy |
| `universe` | `trading_dates`、market、symbol、instrument kind 與 `session_kinds` |
| `strategy` | strategy id、version 與 versioned parameters |
| `replay` | source/cache completeness policy |
| `simulation` | fill、latency、slippage、fee、tax、cash、accounting、marking |
| `instrument_economics` | quantity unit、unit size、currency、multiplier、provenance |
| `output` | run output publication policy |

最小的使用者修改通常是：

```yaml
data:
  data_root: /path/to/my-local-data

universe:
  trading_dates: ["2026-07-20"]
  instruments:
    - market: twse
      symbol: "2330"
      session_kinds: [regular]

strategy:
  id: acceptance.multi-market
  version: "1"
  parameters: {}

simulation:
  market_data_latency_ms: 12
  order_latency_ms: 34
```

不要只複製這段而省略其他 required sections；完整欄位請以
[`examples/config.yaml`](../examples/config.yaml) 為準。

規則：

- 日期、symbol、market 與 session 必須能由 source catalog／session planner resolve。
- monetary、price、multiplier 等 exact numeric values 使用 YAML string；latency 是
  非負整數 milliseconds。
- `market_data_latency_ms` 與 `order_latency_ms` 會進入 effective config／plan identity。
  latency 只影響 order eligibility time，不改變 source event 或 replay ordering。
- `instrument_economics` 的 quantity unit、multiplier 與 currency 必須和 instrument
  一致。
- config 不接受 unknown fields、secret fields、`config_version: 1` 或 negative latency。

完整欄位說明見 [RunConfig reference](config-reference.md)；command 的 side effect 見
[CLI operations](operations/cli.md)。

## 3. 檢查、準備資料與執行

先做不改資料的 config／plan 檢查：

```sh
osmium config check --config my-config.yaml
osmium plan --config my-config.yaml
```

第一次需要下載 source 時：

```sh
osmium data sync --config my-config.yaml
osmium data verify --config my-config.yaml
osmium cache prepare --config my-config.yaml
```

只有 `data sync` 需要 network 與 API credential。`data verify`、`cache prepare`、
`replay`、`backtest` 與 `inspect` 在 source 已準備完成後都應可離線執行。

執行 replay 或 backtest：

```sh
osmium replay --config my-config.yaml
osmium backtest --config my-config.yaml --output runs/my-first-run
osmium inspect --run runs/my-first-run
```

`--output` 必須是尚不存在的 directory。replay 只驗證 deterministic market replay，
不執行 simulation strategy；backtest 才會執行內建 strategy、orders、fills、fee/tax
與 accounting。

若 cache 被刪除，重新執行 `cache prepare` 即可由 verified local source rebuild；不必
重新下載 immutable source。若 source 不完整、cache 不存在或 config identity 改變，
請重新執行 `plan` 查看需要的 action。

## 4. 寫自己的 strategy

### 4.1 Strategy 的責任

策略只能使用：

- 目前的 `DomainEvent`。
- 該 event 套用後的 read-only `MarketStateView`。
- `TradingContext` 與 session phase。
- 自己的 mutable state、已驗證參數與過去 callback 結果。

策略不得：

- 修改 market state、event、replay clock、source 或 cache。
- 取得 next event、future state、final statistics 或 look-ahead data。
- 在 callback 中開 network、讀 wall clock、filesystem 或未記錄 randomness。
- 依 raw Teralion JSON、`received_at` 或 status bit 自行推導 market semantics。
- 在 replay 中動態擴張 universe。

核心 contract 見 [`Strategy` trait](../crates/strategy-api/src/strategy.rs)、
[Strategy API design](design/strategy-api.md) 與
[strategy requirements](requirements/strategy.md)。

### 4.2 Strategy skeleton

以下是一個只產生 indicator、不送單的最小 observer。它沿用 repository 的
`ExampleStrategy` 模式；實際使用時放在自己的 internal Rust crate／binary 中，並依賴
workspace 的 `strategy-api`、`market-state` 與 `market-types` path crates。

```rust
use market_state::StateField;
use market_types::InstrumentId;
use strategy_api::{
    BinaryIdentity, CanonicalParamsChecksum, DeclarationError, IndicatorValue, SessionKind,
    Strategy, StrategyDeclaration, StrategyEventContext, StrategyExecutionError,
    StrategyIdentity, StrategyOutputSink,
};

const STRATEGY_ID: &str = "my-org.state-observer";
const STRATEGY_VERSION: &str = "1";

pub struct MyStrategy {
    identity: StrategyIdentity,
    declaration: StrategyDeclaration,
}

impl MyStrategy {
    pub fn source_binary_identity() -> Result<BinaryIdentity, DeclarationError> {
        let digest = blake3::hash(include_bytes!("my_strategy.rs"));
        BinaryIdentity::new("strategy-source-blake3", *digest.as_bytes())
    }

    pub fn new(instrument: InstrumentId) -> Result<Self, DeclarationError> {
        let identity = StrategyIdentity::new(
            STRATEGY_ID,
            STRATEGY_VERSION,
            Self::source_binary_identity()?,
        )?;
        let declaration = StrategyDeclaration::new([instrument], [SessionKind::Regular])?;
        Ok(Self {
            identity,
            declaration,
        })
    }
}

impl Strategy for MyStrategy {
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
        output.emit_indicator(
            "state_version",
            IndicatorValue::Unsigned(context.market_state().state_version()),
        )?;
        if let StateField::Known { value, .. } = context.market_state().cumulative_volume() {
            output.emit_indicator("cum_volume", IndicatorValue::Unsigned(value.value()))?;
        }
        Ok(())
    }
}
```

上例的 source digest 是 repository sample 的簡化做法；正式 internal deployment 應改成
可回溯的 built artifact digest，並在 strategy version 或 build metadata 改變時更新
identity。`binary_identity` 是 provenance，不是 secret 或 authorization mechanism。

實作有參數時，請將參數驗證、default 與 canonical encoding 放在 strategy definition；
`canonical_params_checksum()` 必須由 versioned deterministic bytes 計算，不能使用
Rust `Debug`、map insertion order 或 floating-point formatting。

### 4.3 產生 order intent

simulation-enabled runner 才能送出 order intent：

```rust
use market_types::{Price, Quantity, QuantityUnit};
use strategy_api::{OrderIntent, OrderSide, OrderType};

let intent = OrderIntent::new(
    instrument.clone(),
    OrderSide::Buy,
    Quantity::new(1, QuantityUnit::TradingUnit)
        .map_err(|_| StrategyExecutionError::new("invalid quantity"))?,
    OrderType::Limit {
        limit_price: Price::parse("100").map_err(|_| {
            StrategyExecutionError::new("invalid limit price")
        })?,
    },
);
output.emit_order_intent(intent)?;
```

送單前仍應檢查 typed `TradingContext` 與 session phase；不要自行解碼 raw flags。order
intent 不是成交保證，fill、rejection、fee、tax 與 accounting 由 simulation layer
決定。replay-only sink 不提供 order capability，呼叫 `emit_order_intent` 會回傳
capability error。

### 4.4 讓 strategy 被執行

generic Rust runner 接受任意 `S: Strategy`，也可透過 `Box<dyn Strategy>` 執行：

- replay-only strategy flow：`strategy_api::run_strategy`
- simulation／multi-market flow：`osmium_runner::run_multi_backtest`

若要讓 `osmium backtest`／`osmium run` 執行自訂 strategy：

1. 建立獨立 strategy crate；可參考 `crates/example-strategy`，不要把第三方策略放入
   `strategy-api` 或 CLI module。
2. 實作 `StrategyFactory`，提供固定 `StrategyDefinition` 與 versioned
   `StrategyParameterSchema`。第一版 schema 只接受 flat object 的 bool、signed/unsigned
   integer、exact decimal string 與 text；unknown field 一律拒絕。
3. 將 crate 加入 `crates/osmium-cli/Cargo.toml`，並在唯一的
   `compiled_strategy_registry()` 明確加入 factory。
4. 重新編譯 `osmium`，再於 config 指定精確 id/version 與參數。

例如 repository 內建示範可直接使用 `examples/smoke-example-strategy.yaml`：

```yaml
strategy:
  id: example.price-threshold-buy-once
  version: "1"
  parameters:
    entry_price: "101" # exact decimal 必須是 YAML string
    # quantity: 1      # schema default 為 1
```

config load 會先套用 default 並建立 strategy，再核對 factory identity、parameter checksum、
explicit universe 與 sessions；任何錯誤都早於下載、cache write、stream open 或 output
staging。成功 run 的 `strategy.json` 會保存 materialized parameters 與實際 binary identity。

### 4.5 Strategy 測試清單

至少為每個 strategy 加入：

- identity、version、binary identity 與 parameter checksum test。
- declaration 的 universe／session test。
- 相同 event sequence 的 output determinism test。
- no-look-ahead test：strategy 看不到 next event 或 future state。
- state 只讀 boundary test。
- order quantity unit、price、universe 與 typed eligibility test。
- callback error／panic 後的 failed run 與 partial output test。

在 repository 內可先執行：

```sh
cargo fmt --check
cargo test -p strategy-api
cargo test -p osmium-runner
cargo clippy --workspace --all-targets -- -D warnings
```

## 5. 常見問題

### Config 可以用 `config_version: 1` 嗎？

不行。release 只接受 v2，請以 `examples/config.yaml` 重新建立 config。

### 為什麼自訂 `strategy.id` 後顯示「not compiled into this binary」？

YAML 不會載入任意外部程式碼。請確認 strategy crate 已成為 CLI dependency、factory 已加入
`compiled_strategy_registry()`，而且目前執行的是重新編譯後的 binary；version 也必須精確
匹配，不會自動選 latest。

### 為什麼 replay 沒有 order 或 fill？

`replay` 是 market replay，不執行 simulation；請使用 simulation-enabled backtest
runner。已註冊的自訂 strategy 請使用 `backtest` 或 `run`。

### 為什麼資料同步後仍不能 replay？

依序執行 `data verify` 與 `cache prepare`，再重新執行 `plan`。source 是可重用的
verified artifact，replay cache 是可刪除、可重建的衍生 artifact。
