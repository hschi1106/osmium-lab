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
selection、source action 與 cache action。版本 1 仍走 M2 planner；M3 的 sync、cache
build、multi-stream replay 尚未在此階段開放，避免把未完成的執行路徑誤當成可用功能。
