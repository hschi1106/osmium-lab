# TWSE Teralion 介面

適用 normalizer：

- equity：`TeralionTwseQuote`，mapping version `4`。
- warrant：`TeralionTwseWarrant`，mapping version `1`。

## 1. 支援範圍

| Profile | Timeline formats | Known skip |
| --- | --- | --- |
| Equity | `STOCK_SNAPSHOT`、`STOCK_REALTIME` | `INTRADAY_ODDLOT_REALTIME` |
| Warrant | `WARRANT_SNAPSHOT`、`WARRANT_REALTIME` | `INTRADAY_ODDLOT_REALTIME` |

盤中／盤後零股、盤後定價、鉅額交易與未列出的 format 不進入 replay。known skip 會保留 raw record、計數與 reason；unknown format 或 profile 不相容 format 在 strict mode 拒絕。

TWSE regular session 為 09:00–13:30，download 與 replay window 為 `[08:55, 13:35)`。download 依 `received_at`，timeline 只依 `match_time`。

## 2. Wire 驗證

每筆 quote 需滿足：

- `type=quote`、`market=twse`。
- symbol 等於 partition identity，不作數值轉換。
- `format` 符合 instrument profile。
- `match_time`、`received_at` 是含 offset 的有效時間。
- `bids`／`asks` 各為 0–5 檔，由 best 到較差排列。
- price 以 exact decimal 解析，quantity 與 cumulative volume 使用 `TradingUnit`。
- populated level 的 price/quantity 必須同時有效；空槽後不可再出現 populated level。

每筆合法 book 是完整 snapshot。少於五檔代表 trailing empty slots，不沿用上一筆剩餘檔位。

## 3. Domain mapping

一般 snapshot／realtime final quote 映射為：

```text
QuoteSnapshot(
  complete book,
  optional TradePrint,
  cumulative volume observation,
  TwseQuoteAnnotations
)
```

`deal` 缺少或使用來源定義的 zero sentinel 時保留為 `NoObservation`，不建立零數量成交。status／limit flags 以 typed TWSE annotations 保存；normalizer 不產生獨立 status event。

來源明確標示的 opening／closing trial record 產生 `IndicativeOpeningAuction` 或 `IndicativeClosingAuction`。它們可以更新試算 state 與觸發 callback，但不是 actual trade 或 fill evidence。

## 4. Intermediate／final group

`STOCK_REALTIME` 可能在相同 `match_time` 提供 intermediate trade 與 final quote：

```text
intermediate -> TradeBatch
final        -> QuoteSnapshot
```

group identity 使用 market、trading date、symbol、source format 與 `match_time`，可跨 API page boundary。normalizer 驗證 intermediate 有成交且沒有 book observation、final 有成交與合法完整 book，並驗證 cumulative volume 關係。group shape 不完整或不一致時拒絕整組，不以 input order、`received_at` 或最大 cumulative volume 修補。

ordering rule 的 `source_phase_rank` 保證 intermediate event 先於同時間 final event。兩個 source observations 都保留，各自形成 state transition 與 callback。

## 5. TradingContext

TWSE annotations 由 market rule evaluator 轉成 new-order、matching 與 fill eligibility。trial、opening、continuous、closing、限制撮合與 unknown／reserved flags 都使用 typed reason code；strategy 不直接解碼 raw bits。

漲停／跌停、瞬間趨勢或處置狀態是 source observation，不代表平台重算交易所規則。沒有明確 resume evidence 時，不以 wall clock 自動解除限制。

## 6. Warrant profile

warrant 使用獨立 mapping identity 與 source formats，不落入 equity branch。quote、完整 snapshot、annotations、auction 與 `match_time` 語意與已驗證的 TWSE quote contract一致；underlying、expiry、strike、option side、currency 與 multiplier 由 instrument reference／economics 明確提供。

repository fixture 位於 [`fixtures/teralion/twse`](../../fixtures/teralion/twse)，僅代表合成契約案例，不代表特定上市商品或完整交易日。

官方格式參考：[TWSE TCP/IP 證券交易資訊網路文件](https://dsp.twse.com.tw/tcpipTradingFiles/list)。共通 event/state 規則見 [回播模型](../architecture/replay-model.md)。
