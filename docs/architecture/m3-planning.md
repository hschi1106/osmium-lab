# M3 設定與規劃

M3 使用 `config_version: 2`，`universe.instruments` 逐一列出 market、symbol 與
`session_kinds`。planner 會對每個 instrument／trading date 建立一個
`SessionPlan` 與 `SourcePartitionKey`，再從對應 partition repository 和 cache
catalog 讀取 source/cache 狀態，最後產生一個可重現的 `ExecutionPlan`。

```yaml
universe:
  trading_dates: ["2026-07-20"]
  instruments:
    - {market: taifex, symbol: TXFH6, session_kinds: [after_hours, regular]}
    - {market: taifex, symbol: CDFH6, session_kinds: [after_hours, regular]}
    - {market: taifex, symbol: CAFH6, session_kinds: [regular]}
```

`osmium plan --config <file>` 會自動辨識版本 2 並列出所有 partitions、session
selection、source action 與 cache action。版本 1 仍走 M2 planner。M3 的 `sync`、
`verify`、partition cache build 與 offline multi-stream `replay`／`run` 會沿用同一份
plan；source 與 cache 尚未完成時，offline execution 會明確回報缺少 artifact，不會
退回讀取 raw source 或建立網路連線。

目前 M3 CLI 的 `backtest` 只發布 deterministic replay summary（`replay_only`）；
multi-instrument strategy、fill isolation 與 per-instrument accounting 仍由後續 M3
strategy/simulation work 交付，不會把這個 summary 假稱為已完成的 P&L backtest。
