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
  market_data_latency_ms: 12
  order_latency_ms: 34
```

兩個欄位會進入 effective config／plan identity。它們只影響 order eligible
`match_time`，不修改 source event 或 replay ordering；缺省值為 `0`。

所有 exact numeric values 使用 YAML string，避免先轉換成 `f64`。禁止 credential-bearing
fields、unknown fields、unknown schema version、invalid instrument reference/economics
組合與 negative values。`osmium init` 產生的 skeleton 需要填入完整 universe 與 economics
後，才能通過 `config check`。

完整欄位與 validation 規則見 [CLI contract](operations/cli.md)；effective identity
與 plan boundary 見 [release cleanup](release/release-cleanup.md)。
