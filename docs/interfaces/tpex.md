# TPEx Teralion mapping

本頁只定義 TPEx source evidence → neutral domain mapping。共通 observation、auction lifecycle 與
MarketState 見[回播模型](../architecture/replay-model.md)；orders/fills 見
[執行模型](../architecture/execution-model.md)。

## Mapping identity 與範圍

- equity：`TeralionTpexQuote` mapping version `9`。
- warrant：`TeralionTpexWarrant` mapping version `8`。

| Profile | Timeline formats | Known skip |
| --- | --- | --- |
| Equity | `STOCK_SNAPSHOT`, `STOCK_REALTIME` | `INTRADAY_ODDLOT_REALTIME` |
| Warrant | `WARRANT_SNAPSHOT`, `WARRANT_REALTIME` | `INTRADAY_ODDLOT_REALTIME` |

Regular session 是 09:00–13:30，planner margin 後 window 為 `[08:55, 13:35)`。Known odd-lot format
只記 source/skip reason；unknown、wrong profile 或 malformed format 在 strict mode reject。

## Wire 與 complete book

Record 必須是 `type=quote`、`market=tpex`，symbol/format/session符合 partition。Price 是 exact
decimal，quantity/cumulative volume 是 `TradingUnit`。Bid/ask 各 0–5 檔、依 best 到較差排序，
price/quantity 成對；每筆是完整 snapshot，不沿用 trailing levels。

第一檔 wire price `0` 表示市價委託揭示，其量存入 `BookSide.market_order_quantity`；不建立零價
level。其他檔位零價 reject，best price/mark/fill depth只用 priced levels。`open_price`、
`high_price`、`low_price` 留在 source lineage，不塞入缺少正式欄位的 domain event。

## Source flags → neutral evidence

`status_flags` bit 7 是 trial，bit 6/5 是 delayed opening/closing，bit 4 是撮合方式，bit 3/2 是
opening/closing marker。`limit_flags` 低兩 bit表示 normal/down/up/reserved instant trend。

| Source evidence | Neutral output |
| --- | --- |
| firm quote + complete book | `QuoteSnapshot` |
| firm `deal=null` | trade `NoObservation` |
| realtime intermediate prints | source-ordered `TradeBatch` |
| trial record | `IndicativeAuction` + partial `AuctionObservation` |
| non-trial VI trend + zero-quantity deal + empty book | `MarketStatus` + `AuctionCollecting(VolatilityInterruption)` |

Zero-quantity pause sentinel 不建立 trade、不清空 firm book/trade。其他 zero-quantity deal reject。
Trial record不成為 actual trade、firm cumulative volume、mark 或 fill evidence。Opening/closing 優先
使用 explicit delayed flag，再看 marker/session window；來源不能證明 purpose 的盤中 trial 保留
unclassified evidence，不猜 `Periodic`。Normalizer 不依固定兩分鐘 duration合成 start/end/resume。

## Realtime group validation

同一 market/date/symbol/format/`match_time` group 的 intermediate records 形成 `TradeBatch`，final
形成 `QuoteSnapshot` 或 status-only `MarketStatus`。Normalizer要求：

- intermediate 都有 trade、沒有 book；
- intermediate auction/trial phase彼此一致，且與 final一致；
- 恰有一筆 final，book/shape合法；
- final cumulative volume 與 intermediate relationship一致。

任何條件不符都 reject 整組，不由 book diff、page order、`received_at` 或最大 cumulative volume
修補。這項 group-phase validation 是 mapping v9/v8 的一部分；舊 mapping cache必須重建。

## Warrant 與限制

Warrant 有獨立 format registry/mapping。Underlying、expiry、strike、option side、currency、quantity
unit 與 multiplier 必須來自 reference/economics。Normalizer 不產生 sequence、aggressor、queue、
latency、disposal rule或 exchange trigger；source 沒 sequence 時由 canonical content tie-break只保證
determinism，不宣稱交易所全域順序。

Fixtures：[`fixtures/providers/teralion/tpex`](../../fixtures/providers/teralion/tpex)。官方參考：
[TPEx 上櫃股票 IP 行情網路規格書](https://dsp.tpex.org.tw/storage/regular_system/%E4%B8%8A%E6%AB%83%E8%82%A1%E7%A5%A8IP%E8%A1%8C%E6%83%85%E7%B6%B2%E8%B7%AF%E8%A6%8F%E6%A0%BC%E6%9B%B8%28V.12.18_TCPIP%29.pdf)。
