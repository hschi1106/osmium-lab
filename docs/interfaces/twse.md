# TWSE Teralion 介面

適用 normalizer：

- equity：`TeralionTwseQuote`，mapping version `11`。
- warrant：`TeralionTwseWarrant`，mapping version `7`。

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
- 最佳第一檔 wire price 為 `0` 時，依 TWSE B.12.00 表示市價委託揭示；normalizer 將其 quantity 存入 `BookSide.market_order_quantity`，不建立零價 `BookLevel`。零價出現在其他檔位時拒絕。

每筆合法 book 是完整 snapshot。少於五檔代表 trailing empty slots，不沿用上一筆剩餘檔位。市價委託量與 priced levels 分開保存；best bid／ask、mark 與 fill evidence 只使用 priced levels。

## 3. 交易狀態 bit 語意

依 TWSE 行情傳輸規格，`status_flags` bit 7 是試算揭示；bit 6／5 分別是試算後延後開盤／收盤註記，只有 bit 7 為 1 時才有意義；bit 4 是撮合方式，bit 3／2 是開盤／收盤註記。`limit_flags` 最低兩 bit 為瞬間價格趨勢：`00` 一般揭示、`01` 暫緩撮合且瞬間趨跌、`10` 暫緩撮合且瞬間趨漲、`11` 保留。

暫緩撮合的 raw TWSE wire 格式另規定：若成交價量欄位有揭示，價格為最近一筆成交價、成交量為 0，且不揭示買賣價量。因此 instant-trend 是 matching 暫停及方向訊號，**不能單獨證明**該筆 `deal`／book 是 indicative auction trial。唯讀查詢 Teralion `/ticks` 的 2026-08-10 TWSE 3026 stability 序列，觀察到 trigger 記錄具有 `status_flags=16`、`limit_flags=2`、`deal.quantity=0`、空買賣簿；後續 trial 記錄為 `status_flags=128`、有模擬 deal／book 且 cumulative volume 不變。這確認該樣本的 Teralion JSON 欄位形狀符合 raw wire 描述；相同 immutable revision 已通過 partition integrity 與 Teralion adapter lifecycle conformance；raw rows 保留在 ignored validation root，不納入 repository。

參考：[TWSE 行情資訊傳輸作業手冊 B.12.00](https://www.twse.com.tw/staticFiles/product/broker/O-104-A10%20TWSE%E8%A1%8C%E6%83%85%E8%B3%87%E8%A8%8A%E5%82%B3%E8%BC%B8%E4%BD%9C%E6%A5%AD%E6%89%8B%E5%86%8A(B.12.00)(202003).pdf)。

## 4. Domain mapping

一般 snapshot／realtime final quote 映射為：

```text
QuoteSnapshot(
  complete book,
  optional ObservedTrade,
  cumulative volume observation,
  TwseQuoteAnnotations
)
```

一般 quote 的 `deal=null` 保留為 `NoObservation`，不建立成交。真實 `/ticks` 樣本確認 zero quantity 會出現在具明確 VI trend、空買賣簿的暫緩撮合 observation；TWSE normalizer 現將這種沒有 firm book／trade 的形狀映射為 `MarketStatus`，只攜帶 cumulative volume 與 typed annotations。它不以空陣列清除暫停前最後一份 firm book，也不把零量 recent-price sentinel 當成交。若沒有 VI trend、仍是 intermediate print 或帶有非空 book，則拒絕零量 deal。另以 verified 3026 partition 確認 stability 結束後會大量出現第一檔零價市價委託揭示，現改以獨立 quantity 保存。這些修正令 equity／warrant mapping identity 升為 v11／v7，舊 cache 必須重建。

目前 normalizer 將所有 `trial=true` record 產生為 `IndicativeAuction`，不回落為 firm
`QuoteSnapshot`；opening／closing 由 delayed flag、marker 與 session window 分類。無法由
來源證明 purpose 的盤中 trial 保留為 unclassified partial `AuctionObservation`，不映射為
`AuctionPurpose::Periodic`；`Periodic` 只在有明確來源或 neutral evidence 時使用。instant-trend
的方向仍保留在來源 annotations，只有有明確證據時才映射為 `VolatilityInterruption` observation。明確標示的
試算 observation 不可成為 actual trade、firm cumulative volume、一般 mark 或 fill evidence。

`trial=false` 且有 instant-trend 的 status-only observation 產生 `MarketStatus`，signal 映射為
`AuctionCollecting(VolatilityInterruption)`；matching context 受限但不合成
`StabilityStarted/Ended`。已觀察到的 TWSE 樣本中，trigger 後約 120 秒出現 trial=false、累計量增加的正式撮合記錄，隨後回到 `status_flags=16` 的逐筆報價；該正式結果仍是 firm event。這是可由 immutable partition 重跑的 adapter certification；通用 domain／Alpha 驗證另以 provider-neutral observations 執行。

## 5. Intermediate／final group

`STOCK_REALTIME` 可能在相同 `match_time` 提供一筆以上的 intermediate trades 與一筆 final quote：

```text
one-or-more intermediates -> TradeBatch（依來源順序保留）
one final                 -> QuoteSnapshot 或 status-only MarketStatus
```

group identity 使用 market、trading date、symbol、source format 與 `match_time`，可跨 API page boundary。normalizer 驗證每筆 intermediate 有成交且沒有 book observation、恰有一筆 final 及合法完整 book，並驗證 cumulative volume 關係。final 若為明確 VI trend、空簿的 zero-quantity trigger，累計量須與最後一筆 intermediate 相同；這種 final 產生 `MarketStatus`，不取代 firm book／trade。group shape 不完整或不一致時拒絕整組，不以 input order、`received_at` 或最大 cumulative volume 修補。

ordering rule 的 `source_phase_rank` 保證 intermediate batch 先於同時間 final event。batch 內 source sequence 保留，但同 timestamp 的多筆 intermediate 合併為一次 state transition／strategy callback；final quote 仍是獨立 event。

## 6. TradingContext

TWSE annotations 只在 provider boundary 轉成 provider-neutral `MarketSignal`；strategy 與
execution simulation 只讀 `MatchingMethod`、`AuctionObservation` 與 order-entry restriction，
不直接解碼 raw bits。

漲停／跌停、瞬間趨勢或處置狀態是 source observation，不代表平台重算交易所規則。沒有明確 resume evidence 時，不以 wall clock 自動解除限制。

## 7. Warrant profile

warrant 使用獨立 mapping identity 與 source formats，不落入 equity branch。quote、完整 snapshot、annotations、auction 與 `match_time` 語意與已驗證的 TWSE quote contract一致；underlying、expiry、strike、option side、currency 與 multiplier 由 instrument reference／economics 明確提供。

repository fixture 位於 [`fixtures/providers/teralion/twse`](../../fixtures/providers/teralion/twse)，僅代表合成契約案例，不代表特定上市商品或完整交易日。

官方格式參考：[TWSE TCP/IP 證券交易資訊網路文件](https://dsp.twse.com.tw/tcpipTradingFiles/list)。共通 event/state 規則見 [回播模型](../architecture/replay-model.md)。
## 8. Stability 暫緩撮合

TWSE 的瞬間價格穩定措施期間只接受限價 ROD；既有市價 ROD 會自動刪除。平台將來源
signal 映射為 `AuctionCollecting(VolatilityInterruption)`，一般與 scheduled execution 都
會拒絕 pause 中新進場的 market ROD，並取消已進場但尚未完成的 market ROD，limit ROD 則依
各自 fill policy 保留。尚未 activation 的 scheduled request 尚未送至交易所，不會僅因
trigger 取消。此行為不代表平台重建完整 stability lifecycle，也不把試算價量當成正式成交。
來源依據：[TWSE 交易制度說明](https://www.twse.com.tw/zh/products/system/trading.html)、[投資人知識網交易制度說明](https://www.twse.com.tw/zh/about/company/guide.html)。目前 `OrderIntent` 只建模 ROD，IOC／FOK 不在支援範圍。
