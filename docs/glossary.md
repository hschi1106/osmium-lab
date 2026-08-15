# 詞彙表

| 詞彙 | 定義 |
| --- | --- |
| source partition | 以 source、market、trading date、symbol 與 session plan 識別的來源資料單位。 |
| verified source | 已通過 query、cursor、payload、manifest 與 checksum 驗證的 immutable source revision。 |
| replay cache | 由 verified source 正規化而成、可刪除並重建的 domain event artifact。 |
| `received_at` | Teralion 擷取時間；只用於 archive query 與來源診斷。 |
| `match_time` | 交易所事件時間；唯一 replay clock 與事件第一排序鍵。 |
| domain event | 與 Teralion wire format 分離、通過驗證且具版本的市場事件。 |
| `MarketState` | 由已排序事件歸納的商品狀態，只包含來源可支持的成交、完整 snapshot 與 annotations。 |
| `TradingContext` | 由 session、目前 event 與更新後 state 推導的下單、matching 與 fill eligibility。 |
| explicit universe | strategy 與設定明確列出的 market／symbol 集合。 |
| session plan | 由 trading date、instrument profile 與 session kinds 解析出的版本化時間區段。 |
| WarmUp／Active／CoolDown | session 的策略執行階段；不會合成 market event。 |
| origin event | strategy 產生 order intent 時所處理的 event；該 event 不可成為同一 order 的 fill evidence。 |
| control time | scheduled execution 使用的 activation、expiry 或 timer 時間；不屬於 replay event stream。 |
| degraded run | 使用者明確允許資料或格式降級後執行，且 artifacts 保存完整警告與品質標記的 run。 |
| canonical checksum | 由版本化、確定性 encoding 計算的內容識別；不包含操作時間等非 domain metadata。 |
