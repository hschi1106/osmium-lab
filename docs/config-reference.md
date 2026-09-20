# RunConfig 設定參考

CLI 只接受 YAML `config_version: 3`。所有正式區塊拒絕 unknown fields；完整基準範例是
[`examples/config.yaml`](../examples/config.yaml)。本頁只定義設定 contract；執行順序見
[使用指南](user-guide.md)。

## 頂層 schema

| Field | Required | Contract |
| --- | --- | --- |
| `config_version` | 是 | 必須為 integer `3` |
| `data` | 是 | source namespace、root 與 lifecycle policy |
| `universe` | 是 | 非空 dates 與唯一 instruments |
| `strategy` | 是 | compiled strategy id/version/parameters |
| `strategy_reference` | 否 | 外部 immutable reference artifact identity |
| `replay` | 是 | data completeness policy |
| `simulation` | 是 | execution、cost 與 accounting model |
| `instrument_economics` | 是 | universe 商品的 quantity/currency/multiplier |
| `output` | 是 | immutable publication policy |

Config 內不得出現 `api_key`、`authorization`、`bearer`、`cookie` 或 `credential` 字樣。
`TERALION_API_KEY` 只由 runtime environment／`.env` 讀取。

## `data`

```yaml
data:
  source: teralion
  data_root: data
  source_policy: strict
  cache_policy: reuse_or_rebuild
```

| Field | Allowed values / validation | Default |
| --- | --- | --- |
| `source` | 非空 stable `SourceId`；current CLI 可 composition 的 provider 為 `teralion` | 無 |
| `data_root` | filesystem path；不得藏 credential | 無 |
| `source_policy` | `strict`, `explicit_degraded` | 無 |
| `cache_policy` | 只接受 `reuse_or_rebuild` | 無 |

## `universe`

```yaml
universe:
  trading_dates: ["2026-07-27"]
  instruments:
    - market: twse
      symbol: "2330"
      instrument_class: equity
      session_kinds: [regular]
```

`trading_dates` 與 `instruments` 都不可為空；instrument identity（market + symbol）不可重複。

| Instrument field | Allowed values / validation |
| --- | --- |
| `market` | `twse`, `tpex`, `taifex` |
| `symbol` | 非空 byte-stable symbol |
| `instrument_class` | `equity`, `warrant`, `future`, `option`；必填 |
| `session_kinds` | 非空 subset：`regular`, `after_hours` |
| `contract_shape` | future 必填：`outright` 或 `calendar_spread`；其他 class 不得提供 |
| `session_profile` | TAIFEX future 必填；其餘依 market/class 內建 profile |
| `reference` | warrant／option 必填；equity／future 不需 |

合法 market/class 組合為 TWSE/TPEx equity 或 warrant，以及 TAIFEX future 或 option。Profiles：

- `twse_regular`
- `tpex_regular`
- `taifex_index_futures`
- `taifex_stock_futures`
- `taifex_stock_futures_regular_only`
- `taifex_calendar_spread_regular_only`
- `taifex_index_options`

Profile 必須支援所選 market/class/shape；calendar spread profile 只允許 regular session。

### Warrant／option `reference`

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

`underlying`、`provenance` 不可空；`expiry` 是 trading date；`strike`、`multiplier` 與
`units_per_trading_unit` 必須大於零。`option_side` 是 `call|put`，`currency` 目前只接受
`TWD|twd`，quantity unit 是 `share|trading_unit|contract`。Reference 的 quantity/currency/
multiplier 必須與同商品 `instrument_economics` 完全相符。

## `strategy` 與 `strategy_reference`

```yaml
strategy:
  id: example.price-threshold-buy-once
  version: "1"
  parameters:
    entry_price: "101"
```

`id`/`version` 必須精確匹配目前 binary 的 registered factory。`parameters` 是 mapping，value 只可為
boolean、signed/unsigned integer 或 string；各 strategy schema 決定 allowed keys、types 與 defaults。
Exact decimal strategy parameter 應使用 string。

可選 reference artifact：

```yaml
strategy_reference:
  schema_version: 1
  path: references/model.bin
  checksum: "<64 lowercase-or-uppercase hex chars>"
```

Relative path 以 config 所在目錄解析；檔案 BLAKE3 必須等於 checksum。這是 immutable input
identity，不是 runtime plugin loader。

## `replay`

```yaml
replay:
  data_policy: strict
```

`data_policy` 接受 `strict` 或 `explicit_degraded`，無 default。Degraded policy 仍須保留 warnings 與
quality identity，不代表可跳過 canonical/integrity validation。

## `simulation`

基準形狀：

```yaml
simulation:
  fill: { evidence: top_of_book, quantity: observed }
  execution_policy: subsequent_event_v1
  market_data_latency_ms: 0
  order_latency_ms: 0
  allocation: acceptance_sequence
  slippage: { model: adverse_fixed_delta, delta: "0" }
  fee: { model: configured_rate, rate: "0", applicable_sides: [buy, sell], minimum: "0", precision: 0, rounding: half_up, provenance: "example" }
  tax: { model: configured_rate, rate: "0", applicable_sides: [sell], minimum: "0", precision: 0, rounding: down, provenance: "example" }
  initial_cash: { currency: TWD, amount: "10000000" }
  position_accounting: average_cost_v1
  marking: { model: last_observable_mark_v1, allow_midpoint_fallback: false }
```

| Field | Allowed values / validation | Default |
| --- | --- | --- |
| `fill.evidence` | `top_of_book`, `trade_print` | 無 |
| `fill.quantity` | `unlimited`, `observed` | 無 |
| `execution_policy` | `subsequent_event_v1`, `scheduled_visible_depth_v1` | `subsequent_event_v1` |
| `scheduled_execution` | 只可與 scheduled policy 一起出現 | absent |
| `market_data_latency_ms` | non-negative `u64` milliseconds | `0` |
| `order_latency_ms` | non-negative `u64` milliseconds | `0` |
| `allocation` | 只接受 `acceptance_sequence` | 無 |
| `slippage.model` | 只接受 `adverse_fixed_delta` | 無 |
| `slippage.delta` | exact decimal string | 無 |
| `instrument_charges` | per-instrument overrides | `[]` |
| `initial_cash.currency` | `TWD|twd` | 無 |
| `initial_cash.amount` | exact decimal string | 無 |
| `position_accounting` | `average_cost_v1` | 無 |
| `marking.model` | `last_observable_mark_v1` | 無 |
| `marking.allow_midpoint_fallback` | boolean | 無 |

Scheduled policy 額外要求：

```yaml
execution_policy: scheduled_visible_depth_v1
scheduled_execution:
  depth_levels: 5
  max_stale_ms: 1000
```

`depth_levels` 必須在 1–5，`max_stale_ms` 必須大於零。Subsequent-event policy 不得帶
`scheduled_execution`。

### Charge schema

`fee`、`tax` 與 per-instrument charge 共用：

| Field | Contract |
| --- | --- |
| `model` | `configured_rate` 或 `fixed_per_unit` |
| `rate` | rate model 必填，fixed model 禁止；exact decimal string |
| `amount_per_unit` | fixed model 必填，rate model 禁止；exact decimal string |
| `applicable_sides` | 含 `buy`、`sell` 或兩者，至少一個 |
| `minimum` | rate model optional，default `"0"`；fixed model不使用 |
| `precision` | decimal places，`u8` |
| `rounding` | `down`, `half_up`, `up` |
| `provenance` | nonempty provenance text |

Per-instrument override：

```yaml
instrument_charges:
  - market: twse
    symbol: "2330"
    fee: { ... }
    tax: { ... }
```

可選 `day_trade_tax` 內含 `charge`、固定
`matching: same_account_instrument_trading_date_fifo`、`timezone_offset_minutes`、
`eligible_dates`、`eligibility_required`、`valid_through` 與 `provenance`。若
`eligibility_required: true`，plan 的 instrument/date 必須有明確 eligibility。

## `instrument_economics`

```yaml
instrument_economics:
  - market: twse
    symbol: "2330"
    quantity_unit: trading_unit
    units_per_trading_unit: 1000
    currency: TWD
    multiplier: "1"
    provenance: "user release configuration"
```

每筆必須對應 universe instrument。`quantity_unit` 是 `share|trading_unit|contract`，currency 目前
只接受 TWD；unit count、multiplier 與 provenance 必須通過 planner/economics validation。不能確認的
economics 不套用猜測 default。

## `output`

```yaml
output: { publication: create_new }
```

`publication` 只接受 `create_new`。Run output directory 已存在時拒絕，不覆寫 evidence。

## Exact values 與 validation summary

Money、price、rate、slippage、strike 與 multiplier 使用 decimal string，避免 `f64`。Dates 使用
`YYYY-MM-DD`。Validation 同時檢查：schema/unknown fields、nonempty/unique universe、market/class/
profile 相容性、strategy declaration、reference/economics 一致性、charge model、scheduled policy 與
publication policy。使用 `osmium config check --config <file>` 驗證 schema，再以 `osmium plan`
驗證本地 source/cache 與完整 execution identity。
