# RunConfig 設定參考

CLI 接受 YAML `config_version: 3`。所有區塊都會拒絕 unknown fields；credential 不得寫入設定。每個 `universe.instruments[]` 必須明確提供 `instrument_class`，不再依 market 猜測商品類別。完整範例見 [`examples/config.yaml`](../examples/config.yaml)。

## 頂層區塊

| 區塊 | 內容 |
| --- | --- |
| `data` | `source`、`data_root`、source 與 cache policy |
| `universe` | trading dates、instrument、kind、session 與可選 reference |
| `strategy` | compiled strategy id、version 與 parameters |
| `strategy_reference` | 可選的外部 reference artifact identity |
| `replay` | source/cache completeness policy |
| `simulation` | fill、latency、slippage、費稅、cash、accounting 與 marking |
| `instrument_economics` | quantity unit、trading unit size、currency、multiplier 與 provenance |
| `output` | run publication policy |

## Universe

```yaml
universe:
  trading_dates: ["2026-07-20"]
  instruments:
    - market: taifex
      symbol: "CDFG6"
      instrument_class: future
      contract_shape: outright
      session_profile: taifex_stock_futures
      session_kinds: [regular]
```

`market` 支援 `twse`、`tpex`、`taifex`。每個 instrument 必須明確指定 `instrument_class`；TAIFEX future 另須指定 `contract_shape` 與 `session_profile`。`calendar_spread` 使用只允許 regular session 的 profile，並會進入 effective config identity。可用 profile：

Option／warrant 必須提供 `reference`（underlying、expiry、strike、option side、currency、multiplier、quantity unit、trading-unit size 與 provenance）。reference 會進入 versioned effective instrument contract identity；修改任一欄位會改變 run checksum，且 reference economics 必須與 `instrument_economics` 相符。

```yaml
      reference:
        underlying: TXO
        expiry: "2026-12-16"
        strike: "24000"
        option_side: call
        currency: TWD
        multiplier: "50"
        quantity_unit: contract
        units_per_trading_unit: 1
        provenance: "verified instrument reference"
```

- `twse_regular`
- `tpex_regular`
- `taifex_index_futures`
- `taifex_stock_futures`
- `taifex_stock_futures_regular_only`
- `taifex_calendar_spread_regular_only`
- `taifex_index_options`

TWSE／TPEx 與 TAIFEX options 使用內建 class profile；TAIFEX futures 不做 symbol 推定。profile 是版本化識別，不接受任意開收盤時間。

## Strategy

```yaml
strategy:
  id: example.price-threshold-buy-once
  version: "1"
  parameters:
    entry_price: "101"
```

strategy 必須已編譯進目前 binary 並加入 registry。parameters 由該 strategy 的 schema 驗證與套用 default；unknown parameter 會被拒絕。

外部 Rust binary 可使用 `osmium_cli::run_with_registry_provider` 注入額外 compiled factories；lab 仍負責所有 command、TUI、錯誤分類與執行流程。strategy-specific 商業政策（例如借貸比例、利率或轉換時間）必須是 strategy parameters，不得加入通用 `simulation` schema。策略若需扣除非 fill 成本，應輸出通用 `CashChargeRequest`。

## Simulation

預設 execution policy 是 `subsequent_event_v1`。latency 為非負整數毫秒，會進入 effective config 與 plan identity，但不修改 source event 或 replay ordering。

```yaml
simulation:
  market_data_latency_ms: 12
  order_latency_ms: 34
```

選擇 `scheduled_visible_depth_v1` 時必須提供：

```yaml
simulation:
  execution_policy: scheduled_visible_depth_v1
  scheduled_execution:
    depth_levels: 5
    max_stale_ms: 1000
  market_data_latency_ms: 12
  order_latency_ms: 34
```

`depth_levels` 範圍為 1–5，`max_stale_ms` 必須大於 0。scheduled request 的 `activate_at` 已包含 order latency，runner 不會重複套用。

## 費用與稅

比率、金額、price 與 multiplier 使用 YAML string，避免經過 `f64`。charge model 支援 `configured_rate` 與 `fixed_per_unit`；適用邊、最低金額、precision、rounding 與 provenance 必須明確設定。

現股當沖優惠稅率是 per-instrument opt-in 設定。`eligibility_required: true` 時，每個 run 的 instrument/date 都必須列在 `eligible_dates`，否則 config validation 失敗。配對方式為同帳戶、同商品、同交易日 FIFO，支援先買後賣與先賣後買。

## 驗證規則

- `data_root` 不得包含 credential。
- universe 不得為空，instrument identity 不得重複。
- strategy identity、parameter schema、universe 與 sessions 必須一致。
- quantity unit、currency、multiplier 與 instrument kind 必須相容。
- 所有 monetary/exact decimal 欄位使用可精確解析的字串。
- negative latency、invalid date、unknown field、unsupported enum 或缺少必要 economics 會被拒絕。
- `output.publication` 使用 `create_new`；既有 output directory 不會被覆寫。

先執行：

```sh
osmium config check --config config.yaml
osmium plan --config config.yaml
```

欄位的實際解析入口位於 `crates/osmium-config`，命令行行為見 [CLI 參考](operations/cli.md)。
