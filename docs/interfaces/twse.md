# TWSE Teralion mapping

本頁只定義 TWSE source evidence → neutral domain mapping。`Observation`、auction round transition、
MarketState 與 matching 的共通語義以[回播模型](../architecture/replay-model.md)為準；order restriction
與 fill 以[執行模型](../architecture/execution-model.md)為準。

## Mapping identity 與範圍

- equity：`TeralionTwseQuote` mapping version `11`。
- warrant：`TeralionTwseWarrant` mapping version `7`。

| Profile | Timeline formats | Known skip |
| --- | --- | --- |
| Equity | `STOCK_SNAPSHOT`, `STOCK_REALTIME` | `INTRADAY_ODDLOT_REALTIME` |
| Warrant | `WARRANT_SNAPSHOT`, `WARRANT_REALTIME` | `INTRADAY_ODDLOT_REALTIME` |

未列出的 format、盤中／盤後零股、盤後定價與鉅額交易不進 replay。Known skip 保留 raw record、
count 與 reason；unknown 或 profile-incompatible format 在 strict mode reject。TWSE regular session 是
09:00–13:30，planner margin 後 download/replay window 為 `[08:55, 13:35)`；archive selection 使用
`received_at`，timeline 只使用 `match_time`。

## Wire 與 complete book

每筆 record 必須是 `type=quote`、`market=twse`，且 symbol/format/session屬於 frozen partition。
Timestamp 必須有效且有 offset；price 以 exact decimal 解析，quantity/cumulative volume 使用
`TradingUnit`。

Bid/ask 各 0–5 檔、依 best 到較差排序，price/quantity 必須成對，empty slot 後不可再出現 level。
每筆合法 book 是完整 snapshot，trailing empty slots 不沿用前態。第一檔 wire price `0` 依 TWSE
B.12.00 表示市價委託揭示，quantity 存入 `BookSide.market_order_quantity`，不建立零價 level；其他
檔位零價 reject。Best price、mark 與 fill depth只使用 priced levels。

## Source flags → neutral evidence

`status_flags` bit 7 是 trial；bit 6/5 是 trial 後 delayed opening/closing；bit 4 是撮合方式；bit 3/2
是 opening/closing marker。`limit_flags` 低兩 bit 為 instant trend：normal、down、up、reserved。
Delayed bits 只在 trial record 有意義；reserved/矛盾組合不猜測。

| Source evidence | Neutral output |
| --- | --- |
| firm quote + valid book | `QuoteSnapshot` with complete book |
| firm `deal=null` | trade `NoObservation` |
| realtime intermediate prints | source-ordered `TradeBatch` |
| trial record | `IndicativeAuction` + partial `AuctionObservation` |
| non-trial VI trend + zero-quantity deal + empty book | `MarketStatus` + `AuctionCollecting(VolatilityInterruption)` |

Zero-quantity VI trigger 是 status sentinel，不是成交，也不以空陣列清除最後 firm book/trade。若沒有
明確 VI trend、仍是 intermediate print 或帶非空 book，zero-quantity deal reject。

所有 trial record 都走 `IndicativeAuction`，不回落成 firm quote。Opening/closing 使用 delayed flag、
marker 與 session window分類；無法證明 purpose 的盤中 trial保留 unclassified partial evidence，
不猜成 `Periodic`。Instant-trend direction 保留 annotation，只有明確證據才成為 VI direction。
Indicative deal/book/volume 不覆寫 firm state，也不是一般 fill evidence。

## Realtime group validation

同一 `match_time` 的 `STOCK_REALTIME` group 可以是：

```text
one-or-more intermediate prints → TradeBatch
one final record               → QuoteSnapshot or MarketStatus
```

Group identity 包含 market、trading date、symbol、format、`match_time`，可跨 API page。Normalizer
要求 intermediate 有 trade 且無 book、恰有一筆合法 final，並驗證 cumulative-volume 關係；shape、
trial phase 或 final consistency 不符時整組 reject，不用 input/page/received order 修補。Ordering
rule 的 source-phase rank保證 intermediate batch 先於同時間 final event。

## Warrant 與限制

Warrant 使用獨立 format registry/mapping，不落入 equity branch。Underlying、expiry、strike、option
side、currency、quantity unit 與 multiplier 由 verified reference/economics 提供，不由 symbol推定。

Normalizer 不合成 queue、aggressor、latency、resume timer、disposal rule或 stability lifecycle。Source
flag 只成為 neutral evidence；execution 是否接受/cancel market Day order由 core policy決定。

Fixtures：[`fixtures/providers/teralion/twse`](../../fixtures/providers/teralion/twse)。官方參考：
[TWSE TCP/IP 證券交易資訊網路文件](https://dsp.twse.com.tw/tcpipTradingFiles/list)。
