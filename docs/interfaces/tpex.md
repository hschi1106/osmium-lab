# TPEx Teralion 介面

適用 normalizer：

- equity：`TeralionTpexQuote`，mapping version `8`。
- warrant：`TeralionTpexWarrant`，mapping version `7`。

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
- 最佳第一檔 wire price 為 `0` 時表示市價委託揭示；quantity 存入 `BookSide.market_order_quantity`，不建立零價 `BookLevel`。零價出現在其他檔位時拒絕；best bid／ask、mark 與 fill evidence 只使用 priced levels。
- 一般 quote 的 `deal=null` 映射為 `NoObservation`。`deal.quantity=0` 只有在 `limit_flags` 明確表示 volatility-interruption trend、不是 intermediate print，且買賣簿皆為空時，才視為 pause sentinel；整筆 observation 映射為 `MarketStatus`，只更新 cumulative volume 與 annotations，不清空先前 firm book／trade。其他 zero-quantity deal 拒絕，避免把非法輸入靜默當成缺值。唯讀查詢 Teralion `/ticks` 的 2026-08-10 TPEx 2948 stability 序列觀察到 trigger 記錄為 `status_flags=16`、`limit_flags=2`、零量 deal、空簿；後續 trial 記錄有 deal／book、`status_flags=128` 且 cumulative volume 不變，約 120 秒後正式成交令累計量增加。相同 immutable revision 已通過 partition integrity 與 Teralion adapter lifecycle conformance；raw rows 保留在 ignored validation root，不納入 repository。
- `status_flags` 與 `limit_flags` 以 `TpexQuoteAnnotations` 保存。

`open_price`、`high_price`、`low_price` 保留在 source lineage，不會在缺少正式 domain 欄位時塞入 `QuoteSnapshot`。

## 3. 交易狀態 bit 語意

依 TPEx IP 行情網路規格，`status_flags` bit 7 是試算揭示；bit 6／5 分別是試算後延後開盤／收盤註記，bit 4 是撮合方式，bit 3／2 是開盤／收盤註記。`limit_flags` 最低兩 bit 為瞬間價格趨勢：`00` 一般揭示、`01` 暫緩撮合且瞬間趨跌、`10` 暫緩撮合且瞬間趨漲、`11` 保留。延後開收盤 bit 只有在 trial 狀態下有意義。

TPEx 對暫緩撮合的揭示語意指出：價格欄位代表最近成交價、成交量為 0，且不揭示買賣價量。故 instant-trend 是 matching 暫停及方向訊號，**不能單獨證明**該筆 Teralion `deal`／book 是 indicative auction trial。上述真實樣本確認 trigger 的 Teralion JSON 欄位形狀符合 raw wire 描述；trial 與正式撮合的區分則由 `status_flags`、累計量變化及序列共同支持，不可只看一個 flag。

參考：[TPEx 上櫃股票 IP 行情網路規格書 V.12.18](https://dsp.tpex.org.tw/storage/regular_system/%E4%B8%8A%E6%AB%83%E8%82%A1%E7%A5%A8IP%E8%A1%8C%E6%83%85%E7%B6%B2%E8%A6%8F%E6%A0%BC%E6%9B%B8%28V.12.18_TCPIP%29.pdf)。

## 4. Domain mapping

一般完整 quote 產生：

```text
QuoteSnapshot(
  complete book,
  optional ObservedTrade,
  cumulative volume observation,
  TpexQuoteAnnotations
)
```

realtime intermediate/final group 分別產生 `TradeBatch` 與 `QuoteSnapshot`；若 final 是 status-only pause sentinel，則產生不覆寫 firm book／trade 的 `MarketStatus`。兩者都驗證 final cumulative volume。group 不完整時 strict reject，不從 book 差分、page order 或 `received_at` 推定成交。

目前 normalizer 將所有 `trial=true` record 產生為 `IndicativeAuction`。opening／closing 優先
使用明確 delayed flag，再使用 marker／session window；無法由來源證明 purpose 的盤中 trial
保留為 unclassified partial `AuctionObservation`，不映射為 provider-neutral
`AuctionPurpose::Periodic`；`Periodic` 只在有明確來源或 neutral evidence 時使用。並保留 instant-trend annotations。明確標示的試算價量、五檔與
cumulative volume 只進 indicative state，不得覆寫 firm state 或成為一般 fill evidence。

`trial=false` 且有 instant-trend 的 status-only observation 產生 `MarketStatus`，映射為
`AuctionCollecting(VolatilityInterruption)` 並保留原始 annotation；normalizer 不依兩分鐘
duration 合成 lifecycle event。真實 TPEx 樣本的正式撮合 record 在約 120 秒後以 `trial=false`
出現、成交量與 cumulative volume 增加，之後才恢復逐筆揭示。`EquityIndicativeObservation` 只接受 typed `IndicativeAuction`，不再接受
trial `QuoteSnapshot` workaround。

## 5. Reject／skip policy

identity mismatch、unknown format、缺少必要欄位、無效時間、非法價量、超過五檔、level ordering 錯誤或不完整 match group 均在 strict mode 拒絕。known odd-lot format 只保留 source 與 skip reason，不進 cache timeline。

normalizer 不產生 sequence、aggressor、queue、latency 或未經來源證實的 market semantics。source 沒有 sequence 時，以 canonical content tie-break 保證可重現。

## 6. Warrant profile

warrant profile 使用獨立 format registry 與 mapping identity，不套用 equity format 名稱。underlying、expiry、strike、option side、currency、quantity unit 與 multiplier 需由 reference／economics 明確提供。

repository fixture 位於 [`fixtures/providers/teralion/tpex`](../../fixtures/providers/teralion/tpex)，只固定合成的 quote、auction、annotation 與 state mapping，不代表完整交易日。零價市價委託的 source contract 依 TPEx IP 行情格式固定；目前 2948 validation partition 未出現該形狀，故另以 adapter regression 驗證。

官方參考：[TPEx 上櫃股票 IP 行情網路規格書](https://dsp.tpex.org.tw/storage/regular_system/%E4%B8%8A%E6%AB%83%E8%82%A1%E7%A5%A8IP%E8%A1%8C%E6%83%85%E7%B6%B2%E8%B7%AF%E8%A6%8F%E6%A0%BC%E6%9B%B8%28V.12.18_TCPIP%29.pdf)。共通規則見 [回播模型](../architecture/replay-model.md)。
## 7. Stability 暫緩撮合

TPEx 的瞬間價格穩定措施期間只接受限價 ROD，並刪除既有一般市價委託。平台將來源
signal 映射為 `AuctionCollecting(VolatilityInterruption)`；一般與 scheduled execution 都會
拒絕 pause 中新進場的 market ROD，並取消已進場但尚未完成的 market ROD，limit ROD 則依各自
fill policy 保留。尚未 activation 的 scheduled request 尚未送至交易所，不會僅因 trigger
取消；試算價量不作正式 fill evidence。此處只建模已觀察到的限制，不推算 stability start/end。
來源依據：[TPEx 交易制度說明](https://www.tpex.org.tw/zh-tw/mainboard/trading/rules/continuous.html)。目前 `OrderIntent` 只建模 ROD，IOC／FOK 不在支援範圍。
