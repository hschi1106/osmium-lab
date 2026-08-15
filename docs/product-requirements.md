# osmium-lab 產品需求

## 1. 產品定位

`osmium-lab` 是以 Rust 建立、使用 Teralion 歷史行情的台灣市場回播與回測平台。系統依 `match_time` 重播可觀察的成交與行情快照，讓策略在沒有未來資料的前提下執行，並以明確、可版本化的模型估算成交與帳務結果。

產品原則依序為：

1. 一份設定可以完成資料規劃、準備與執行。
2. 已驗證的來源資料可跨多次回測重用。
3. 相同資料、版本與設定產生相同結果。
4. 模型不宣稱來源資料無法支持的撮合或排隊精度。
5. 市場、商品、session、五檔、成交與來源 flags 的差異保留在明確的介面契約中。

## 2. 支援範圍

支援的市場與商品：

- TWSE 股票與權證。
- TPEx 股票與權證。
- TAIFEX 期貨與選擇權。
- 處置證券以一般商品方式回播；來源若提供特殊狀態，平台保存並呈現該資訊，但不重算處置撮合規則。

不支援的範圍：

- 盤中零股、盤後零股、盤後定價與鉅額交易。
- 即時交易或自動下單。
- 完整交易所撮合、逐筆委託簿、真實 queue position 或 hidden liquidity 推論。
- 瞬間價格穩定措施、處置撮合規則或其他交易所內部狀態的重建。
- 從低粒度資料合成不存在的高粒度資料。
- 回播或回測期間自動存取網路。

## 3. 資料能力與限制

Teralion tick 是回播能力的上限：

- TWSE／TPEx quote 可提供完整最佳五檔、可選成交、累計量與來源 flags。
- TAIFEX tick 可提供成交批次、完整五檔與非時間軸的 close／stats 記錄。
- 同一商品可能有多種 `format`，每種格式由明確的 normalizer 處理。
- 商品 metadata 可能缺少 multiplier、underlying 或其他欄位；缺值不得自行推定。
- `received_at` 是擷取時間，只用於 archive query 與診斷；`match_time` 是唯一回播時間。
- 沒有有效 `match_time` 的記錄不得插入事件時間軸。

Teralion wire payload 與 domain event 必須分離。未知或不支援的格式需明確拒絕或以設定允許的 degraded 模式略過，並保留可追溯警告。

## 4. 使用流程

```text
RunConfig
  -> execution plan
  -> source sync / verify
  -> replay cache prepare
  -> deterministic replay
  -> strategy callbacks
  -> execution simulation / accounting
  -> immutable run artifacts
```

資料同步與回測可分開執行。source 與 cache 準備完成後，`replay`、`backtest`、`run`、`display` 與 `inspect` 不需要網路或 API credential。

## 5. 資料需求

### DATA-01 資料取得

- 支援 Teralion coverage、symbol range、ticks、daily instrument 與 opaque cursor pagination。
- 所有 cursor pages 必須完整取得；HTTP、schema、cursor 或儲存錯誤不得被視為合法空頁。
- 只有已結束交易日可發布為可重用 source partition。
- credential 只存在於 online sync context，不得寫入 source、cache、log 或 run artifacts。

### DATA-02 本地來源資料

- partition identity 至少包含 source、market、trading date、symbol、session 與 session plan identity。
- published source revision 是 immutable artifact，保存 frozen query、payload、metadata、頁數、筆數與 checksum。
- sync 使用 staging 與 atomic publish；失敗的 staging 不得被視為完整資料。
- 已發布 revision 不得被靜默覆寫。

### DATA-03 完整性

每個 partition 必須可辨識為 `Missing`、`Building`、`Complete`、`Incomplete` 或 `Corrupt`。strict 執行只接受通過 identity、manifest、cursor、payload 與 checksum 驗證的 `Complete` source；degraded 執行必須明確記錄品質與警告。

### DATA-04 回播快取

- replay cache 由 verified source 建立，可刪除並離線重建。
- cache identity 綁定 source checksum、normalizer mapping、event schema、ordering rule 與 cache format。
- cache 失效或損壞不應觸發 source 重新下載。
- replayer 只開啟 execution plan 中需要的商品與日期 streams。

### DATA-05 商品資料

- identity 與可用的 kind、expiry、strike、option side、currency、multiplier 需保留來源。
- 影響帳務的 quantity unit、multiplier 與 currency 必須由已驗證 metadata 或明確設定提供。
- TAIFEX 跨日資料依 exchange trading date 與 session plan 歸屬，不以本地日曆日期切割。

## 6. 回播需求

### REPLAY-01 標準事件

domain event 集合為：

- `QuoteSnapshot`：完整五檔，以及同一 source observation 的成交、累計量與 annotations。
- `BookSnapshot`：完整五檔與 annotations。
- `TradeBatch`：同一 source observation 的一筆或多筆成交與可用累計量。
- `IndicativeOpeningAuction`、`IndicativeClosingAuction`：試算資訊，不是實際成交。

每個 event 包含 instrument、trading date、source format、`match_time`、可選 source sequence 與 payload。同一 source observation 的不可分割內容以單一 event 原子處理。

### REPLAY-02 排序

`match_time` 是第一排序鍵與唯一 replay clock。相同時間依版本化內容鍵排序：market rank、symbol、source format、source phase、event kind、source sequence 與 event fingerprint。此順序只保證可重現，不代表交易所的全域封包順序。

### REPLAY-03 市場狀態

每個商品的 `MarketState` 保存目前完整五檔、最近成交、累計量、annotations、最後 `match_time` 與 state version。新的完整 snapshot 取代舊 snapshot；系統不重建逐筆委託或 queue。

### REPLAY-04 處理順序

```text
select event
  -> advance replay clock
  -> atomically reduce MarketState
  -> derive TradingContext
  -> invoke strategy with post-event read-only state
  -> process strategy output and feedback
```

策略不能取得下一事件、未完成狀態或日後才知道的統計值。

### REPLAY-05 Session 與多商品

- universe 使用明確 market／symbol 清單與 semantic session kinds。
- planner 以版本化 calendar／profile 產生 session plan 與前後五分鐘 window。
- 多商品以 bounded streaming merge 執行，不需將整段資料載入記憶體。
- WarmUp、Active 與 CoolDown 控制策略可見性與交易資格，不合成市場事件。

### REPLAY-06 錯誤處理

缺少或無效時間、順序錯誤、未知格式、非法價量、資料不完整或 checksum 不符時，系統依 strict policy 停止，或依明確選擇的 degraded policy 繼續並留下警告。

## 7. 策略需求

### STRAT-01 策略能力與邊界

- strategy 以 Rust trait 實作，編譯進 binary 並加入 registry。
- strategy identity 包含 id、version、binary identity 與 canonical parameter checksum。
- strategy 宣告 explicit universe 與 session kinds。
- callback 只能讀取目前 event、更新後的 `MarketStateView`、`TradingContext`、session context 與 deterministic feedback。
- strategy 可產生 indicator、order intent、scheduled request 與 timer；能力由 runner 明確授予。
- strategy 不得修改 market state、replay clock 或 historical event，也不得讀取網路、wall clock、未記錄 randomness 或 future data。
- callback error 或 panic 必須使 run 明確失敗，不能發布成功結果。

## 8. 模擬與帳務需求

### SIM-01 成交模型

預設 `subsequent_event_v1` 模型只在 origin event 之後的 eligible event 判定成交：

- market order 使用後續可觀察價格並套用 slippage。
- limit order 需有後續成交或行情穿越限價的證據。
- quantity 可受觀察成交量或顯示量限制。
- 不確定時採保守結果，不推定真實 queue position。

可選的 `scheduled_visible_depth_v1` 以 execution control time、market-data latency、order latency、snapshot staleness 與最多五檔可見量執行。control action 不得寫入 replay event stream，也不得修改 `MarketState` 或 event/state checksum。

### SIM-02 帳務

- 所有 fill、cash、position、fee、tax、realized／unrealized P&L 與 marking 變化可追溯至 order intent。
- exact 金額與比率不經 binary floating-point。
- equity、futures 與 options 使用明確的 instrument economics 與 accounting model。
- 每次執行結束需 reconciliation；失敗不得發布 successful performance。

## 9. 操作與非功能需求

### OPS-01 操作

- CLI 提供 `init`、`config check`、`plan`、`data sync`、`data verify`、`cache prepare`、`replay`、`backtest`、`run`、`display` 與 `inspect`。
- `display` 是只讀 TUI，共用 `match_time` 時間軸，可暫停、切換固定倍率與標的；不建立策略、委託或 run artifacts。
- 錯誤需指出 category 與可辨識的 market、symbol、date、format 或 artifact context。

### OPS-02 執行結果

run artifacts 至少保存 effective config checksum、execution plan identity、source/cache checksum、版本集合、strategy identity 與 materialized parameters、warnings、orders、fills、positions、P&L、event checksum 與 final-state checksum。output directory 以 staging 建立並以 atomic publish 完成。

### NFR-01 可重現

相同 source、cache identity、版本、strategy 與 effective config 必須得到相同事件順序、策略輸出、orders、fills 與帳務結果。並行與效能最佳化不得改變 domain result。

### NFR-02 效能

系統優先避免重複下載、JSON parsing 與 universe 外 I/O，並以 bounded stream 處理多商品。benchmark 使用固定資料與版本化輸入。

### NFR-03 安全與版本

credential 不得進入設定、資料 artifact、log 或版本控制。source interface、normalizer mapping、event schema、ordering、cache、strategy output、fill model、accounting 與 run manifest 均需有相容性 identity；不相容時拒絕或由 verified source 重建。

## 10. 驗證原則

| 需求面 | 主要證據 |
| --- | --- |
| 資料同步 | cursor、resume、atomic publish、checksum 與 second-run reuse tests |
| 來源正規化 | TWSE／TPEx／TAIFEX fixture tests 與 unknown-format negative tests |
| 回播 | shuffled-input ordering、multi-stream merge、state reducer 與 checksum tests |
| 策略 | read-only compile tests、no-look-ahead、callback transaction 與 registry tests |
| 模擬帳務 | market／limit、scheduled depth、latency、fee／tax、P&L 與 reconciliation tests |
| 操作 | CLI contract、offline flow、TUI state 與 release smoke tests |

需求與程式入口的對照見 [追溯矩陣](traceability.yaml)，操作驗證見 [驗證文件](operations/validation.md)。

## 11. 參考資料

- [Teralion Feed Archive API](https://docs.teraliontech.com/feed-archive/)
- [TWSE TCP/IP 證券交易資訊網路文件](https://dsp.twse.com.tw/tcpipTradingFiles/list)
- [TPEx 上櫃股票 IP 行情網路規格書](https://dsp.tpex.org.tw/storage/regular_system/%E4%B8%8A%E6%AB%83%E8%82%A1%E7%A5%A8IP%E8%A1%8C%E6%83%85%E7%B6%B2%E8%B7%AF%E8%A6%8F%E6%A0%BC%E6%9B%B8%28V.12.18_TCPIP%29.pdf)
- [TAIFEX 逐筆行情資訊傳輸作業手冊](https://www.taifex.com.tw/cht/8/techDocsDetails?idx=67)

介面行為以 [Teralion](interfaces/teralion.md)、[TWSE](interfaces/twse.md)、[TPEx](interfaces/tpex.md) 與 [TAIFEX](interfaces/taifex.md) 文件中由 fixture 與測試固定的範圍為準。
