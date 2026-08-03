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
selection、source action 與 cache action。版本 1 不屬於 release compatibility。
`data sync`、`data verify`、partition cache build 與 offline multi-stream `replay`／`run` 會沿用同一份
plan；source 與 cache 尚未完成時，offline execution 會明確回報缺少 artifact，不會
退回讀取 raw source 或建立網路連線。

`osmium_fixture_data` 是只供 acceptance 使用的 fixture-to-source adapter，位於
`tools/acceptance/`，不進 production workspace。它不繞過
`TeralionSync`：每個 selected shard 先經 paged response envelope、TAIFEX
`book/close/stats/trade` kind 與 `taifex_fut` market validation，再以
`StagingRevision` atomic publish。production `sync` 與 fixture adapter 因此共用
相同 cursor／source boundary；fixture adapter 不會讀取 gitignored `raw/`。

Release CLI 的 `backtest` 會在 bounded multi-stream replay 上執行 linked
multi-instrument strategy，並以 instrument-isolated simulator 產生 subsequent-event
orders/fills。step 4 會依商品套用明確的 `EquityV1` 或 `FuturesV1` accounting model：
futures 只在平倉時以 `price difference × quantity × multiplier` 反映現金與 realized
P&L，所有商品都會執行 exact decimal reconciliation 與 final marking。

發布的 `run-manifest.yaml` 會標示 `successful`／`full`，並包含 versioned
`ledger.bin`、`positions.yaml`、`performance.yaml`、checksums 與 per-instrument
economics provenance。`inspect` 會驗證 replay、strategy output、orders、fills 與
accounting artifacts 的 checksum；若 open position 沒有合法 final mark，backtest
會在發布前失敗，不產生 successful performance artifact。

`LocalCacheFactory` 會在 `OSMIUM_STREAM_OPEN_AUDIT` 被指定時追加實際 opened
bindings。這是 operational evidence，不進 plan、event 或 result identity；沒有
指定時不產生額外檔案。
