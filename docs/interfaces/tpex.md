# TPEx Teralion 介面

適用 normalizer：

- equity：`TeralionTpexQuote`，mapping version `1`。
- warrant：`TeralionTpexWarrant`，mapping version `1`。

## 1. 支援範圍

| Profile | Timeline formats | Known skip |
| --- | --- | --- |
| Equity | `STOCK_SNAPSHOT`、`STOCK_REALTIME` | `INTRADAY_ODDLOT_REALTIME` |
| Warrant | `WARRANT_SNAPSHOT`、`WARRANT_REALTIME` | `INTRADAY_ODDLOT_REALTIME` |

regular session 為 09:00–13:30，download 與 replay window 為 `[08:55, 13:35)`。archive selection 使用 `received_at`；event、state 與 strategy 只使用 `match_time`。

## 2. Wire 與 snapshot

- `type=quote`、`market=tpex`，symbol 與 profile 必須符合 partition。
- `bids`／`asks` 各為 0–5 檔且由 best 到較差排列。
- 每筆 quote 是完整 snapshot，trailing empty slots 不沿用上一筆資料。
- price 使用 exact decimal；quantity 與 cumulative volume 使用 `TradingUnit`。
- `deal=null` 或來源定義的 zero-quantity sentinel 映射為 `NoObservation`，不建立零數量成交。
- `status_flags` 與 `limit_flags` 以 `TpexQuoteAnnotations` 保存。

`open_price`、`high_price`、`low_price` 保留在 source lineage，不會在缺少正式 domain 欄位時塞入 `QuoteSnapshot`。

## 3. Domain mapping

一般完整 quote 產生：

```text
QuoteSnapshot(
  complete book,
  optional TradePrint,
  cumulative volume observation,
  TpexQuoteAnnotations
)
```

realtime intermediate/final group 分別產生 `TradeBatch` 與 `QuoteSnapshot`，並驗證 final cumulative volume。group 不完整時 strict reject，不從 book 差分、page order 或 `received_at` 推定成交。

trial record 只有在 marker 與 session window 能唯一分類時才產生 `IndicativeOpeningAuction`／`IndicativeClosingAuction`。無法唯一分類的 in-session trial 保持普通 quote observation，不猜測 auction phase。

## 4. Reject／skip policy

identity mismatch、unknown format、缺少必要欄位、無效時間、非法價量、超過五檔、level ordering 錯誤或不完整 match group 均在 strict mode 拒絕。known odd-lot format 只保留 source 與 skip reason，不進 cache timeline。

normalizer 不產生 sequence、aggressor、queue、latency 或未經來源證實的 market semantics。source 沒有 sequence 時，以 canonical content tie-break 保證可重現。

## 5. Warrant profile

warrant profile 使用獨立 format registry 與 mapping identity，不套用 equity format 名稱。underlying、expiry、strike、option side、currency、quantity unit 與 multiplier 需由 reference／economics 明確提供。

repository fixture 位於 [`fixtures/teralion/tpex`](../../fixtures/teralion/tpex)，只固定合成的 quote、auction、annotation 與 state mapping，不代表完整交易日。

官方參考：[TPEx 上櫃股票 IP 行情網路規格書](https://dsp.tpex.org.tw/storage/regular_system/%E4%B8%8A%E6%AB%83%E8%82%A1%E7%A5%A8IP%E8%A1%8C%E6%83%85%E7%B6%B2%E8%B7%AF%E8%A6%8F%E6%A0%BC%E6%9B%B8%28V.12.18_TCPIP%29.pdf)。共通規則見 [回播模型](../architecture/replay-model.md)。
