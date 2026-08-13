# RunConfig reference

Release 只接受 YAML `config_version: 2`。`config_version: 1` 是 historical material，
會被明確拒絕，不提供 migration。

必要區塊如下：

| 區塊 | 作用 |
| --- | --- |
| `data` | Teralion source 與 user-owned `data_root`、source/cache policy |
| `universe` | trading dates、market、symbol、instrument kind、sessions 與 optional reference |
| `strategy` | linked strategy id、version 與 parameters |
| `replay` | source/cache completeness policy |
| `simulation` | fill、latency、slippage、fee、tax、cash、accounting、marking |
| `instrument_economics` | quantity unit、multiplier、currency 與 provenance |
| `output` | run publication policy |

latency 是非負整數毫秒：

```yaml
simulation:
  execution_policy: scheduled_visible_depth_v1
  scheduled_execution:
    depth_levels: 5
    max_stale_ms: 1000
  market_data_latency_ms: 12
  order_latency_ms: 34
```

兩個欄位會進入 effective config／plan identity。它們只影響 order eligible
`match_time`，不修改 source event 或 replay ordering；缺省值為 `0`。

`execution_policy` 缺省為 `subsequent_event_v1`，此時不得提供 `scheduled_execution`。
選擇 `scheduled_visible_depth_v1` 時，`depth_levels` 必須為 1–5，`max_stale_ms` 必須大於
0；`order_latency_ms` 由策略建立最終 `activate_at` 時使用，runner 不會在 scheduled request
上重複加一次 latency。

若 symbol 無法由內建 fixture 對應到 session，instrument 可明確提供經驗證的
`session_profile`。例如個股期貨日盤：

```yaml
universe:
  instruments:
    - market: taifex
      symbol: "CDFG6"
      instrument_kind: future
      session_profile: taifex_stock_futures
      session_kinds: [regular]
```

profile 必須與 market／`instrument_kind` 相容；省略時維持既有的 metadata resolver。
可用值為 `twse_regular`、`tpex_regular`、`taifex_index_futures`、
`taifex_stock_futures`、`taifex_stock_futures_regular_only` 與
`taifex_index_options`。這個欄位只選擇平台內建且有版本的 profile，不能輸入任意開收盤時間。

現股當沖證交稅是 per-instrument opt-in 設定；法規數值仍使用 exact YAML string：

```yaml
simulation:
  instrument_charges:
    - market: twse
      symbol: "2330"
      fee:
        model: configured_rate
        rate: "0.001425"
        applicable_sides: [buy, sell]
        minimum: "0"
        precision: 0
        rounding: down
        provenance: "broker schedule"
      tax:
        model: configured_rate
        rate: "0.003"
        applicable_sides: [sell]
        minimum: "0"
        precision: 0
        rounding: down
        provenance: "MOF ordinary stock tax"
      day_trade_tax:
        charge:
          model: configured_rate
          rate: "0.0015"
          applicable_sides: [sell]
          minimum: "0"
          precision: 0
          rounding: down
          provenance: "MOF reduced day-trade tax"
        matching: same_account_instrument_trading_date_fifo
        timezone_offset_minutes: 480
        eligible_dates: ["2026-06-23", "2026-06-24"]
        eligibility_required: true
        valid_through: "2027-12-31"
        provenance: "TWSE day-trading eligibility"
```

`eligibility_required: true` 表示每個 run instrument-date 必須列在 `eligible_dates`；
缺少時 config／plan validation 失敗，不可假設為 eligible。配對同時支援先買後賣與先賣後買，
只對同日配對 quantity 使用優惠稅率。

股期等按成交單位計價的費用可用 `fixed_per_unit`；以下表示每成交一口收取 TWD 100，若同一
order 分成多筆 fill，仍依各 fill quantity 加總，不會誤算成每筆 fill 100：

```yaml
fee:
  model: fixed_per_unit
  amount_per_unit: "100"
  applicable_sides: [buy, sell]
  precision: 0
  rounding: down
  provenance: "broker stock-futures schedule"
```

所有 exact numeric values 使用 YAML string，避免先轉換成 `f64`。禁止 credential-bearing
fields、unknown fields、unknown schema version、invalid instrument reference/economics
組合與 negative values。`osmium init` 產生的 skeleton 需要填入完整 universe 與 economics
後，才能通過 `config check`。

完整欄位與 validation 規則見 [CLI contract](operations/cli.md)；effective identity
與 plan boundary 見 [release cleanup](release/release-cleanup.md)。
