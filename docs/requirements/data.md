# 資料需求

## 1. 文件目的

本文件將[產品需求](../product-requirements.md)中的 `DATA-01` 至 `DATA-05`
細化為可設計、實作與驗證的系統需求。

本文件定義資料取得、本地保存、完整性、衍生回播快取及商品資料的必要行為，
不固定：

- crate 或 module 邊界
- HTTP client 或 retry library
- 本地檔案格式與目錄布局
- checksum algorithm
- database、object store 或 filesystem 的選擇
- CLI syntax

上述選擇由 architecture、design 與 operations 文件記錄。資料設計不得破壞
「已驗證來源資料可重用、衍生快取可重建、回測預設離線」三項邊界。

## 2. 範圍與資料邊界

資料流程分為：

```text
Teralion Feed Archive
-> coverage／商品範圍確認
-> ticks 與每日商品資料分頁下載
-> 暫存與驗證
-> 發布為本地來源資料
-> 正規化／建立衍生回播快取
-> 離線 replay／backtest
```

本文件涵蓋：

- 已結束交易日的 Teralion 歷史資料取得
- opaque cursor 的完整分頁
- 本地來源資料的識別、保存與不可靜默覆寫
- 商品／交易日資料完整性狀態
- 可刪除並重建的 replay cache
- 商品資料及 reference-data provenance
- TAIFEX trading date 歸屬

本文件不涵蓋：

- source tick 到 domain event 的欄位 mapping，見各 market interface 文件及
  [回播需求](replay.md)。
- replay ordering 與 market state，見[回播需求](replay.md)。
- fill、帳務與 P&L，見[模擬需求](simulation.md)。
- 最終檔案 layout 與 serialization，見
  [data sync 設計](../design/data-sync.md)及
  [本地資料操作](../operations/local-data.md)。

## 3. 通用定義

### 3.1 資料單位

第一版本地來源資料的最小管理單位是：

```text
market + trading_date + symbol
```

同一管理單位可以包含多個 source format，但 manifest 必須能分別識別其格式、
筆數、查詢範圍與完整性。

`trading_date` 是交易所業務日，不一定等於 tick 所在的日曆日期。尤其 TAIFEX
夜盤不得僅以本地日期或 UTC 日期切割。

### 3.2 本地來源資料

`本地來源資料` 是由 Teralion 取得、完成必要驗證並發布為可重用的資料。它保留
後續重新正規化所需的來源 payload 與 provenance。

來源資料不是 replay cache。event schema 或排序規則變更時，來源資料應可繼續
使用並重建衍生資料。

### 3.3 暫存資料

`暫存資料` 是下載或驗證尚未完成的工作中產物。暫存資料不得被 replay／backtest
當作完整來源資料，也不得與已發布資料使用無法區分的狀態。

### 3.4 Replay cache

`replay cache` 是從已驗證來源資料產生的 domain event 或其他加速回播的衍生物。
它可以刪除、失效或重建，不是來源資料的唯一副本。

### 3.5 Provenance

`provenance` 是足以回答資料從何而來、用何種查詢取得、經何種版本處理，以及
為何被判定完整或不完整的記錄。

## 4. DATA-01：資料取得

### DATA-01.1 支援的 Teralion 能力

系統必須支援 Teralion Feed Archive 的：

- market coverage
- 單一商品可用範圍
- ticks
- 每日商品資料
- opaque keyset cursor

實際 endpoint、request／response schema 及 market-specific 差異由
[Teralion interface](../interfaces/teralion.md)記錄，並以 API integration tests
固定。

### DATA-01.2 同步計畫

發出下載 request 前，系統必須根據使用者指定的 market、symbol、trading date
及本地狀態建立同步計畫。計畫至少區分：

- 已完整且可直接使用
- 缺少而需要下載
- 建置中或上次中斷而需要恢復／重試
- 不完整而需要明確處理
- 損壞而需要重新取得或修復
- Teralion coverage 不包含

同步計畫不得將「coverage 不包含」當作「成功但零筆」而靜默完成。

### DATA-01.3 已結束交易日

第一版只同步已結束的 trading date，不處理當日增量資料。

系統必須以明確的 market calendar／trading-date 規則判定交易日是否已結束，不得
單純以呼叫時刻的日曆日期判斷。判定所用的規則或版本必須可檢查。

若無法安全確認 trading date 已結束，系統預設不得將資料發布為完整。

### DATA-01.4 Cursor 完整分頁

對回傳 cursor 的查詢，下載器必須：

1. 保存本頁資料及必要的 page metadata。
2. 將 cursor 視為 opaque value，不解析、不改寫、不自行產生。
3. 只在服務回應表示沒有下一頁時結束。
4. 防止因單頁上限而把部分結果誤判為完整。
5. 偵測無法前進、重複或不合法 cursor，並以錯誤停止。

頁面大小、網路 retry 或程序中斷不得改變最終已發布資料的內容與 checksum。

### DATA-01.5 Retry 與中斷恢復

暫時性網路或服務錯誤可以依明確 policy 重試，但必須：

- 不把失敗 response 當成合法空頁。
- 不在 log、暫存 metadata 或錯誤中洩漏 API key。
- 不讓重試造成已發布 tick 的靜默重複或遺失。
- 在 retry exhausted 後保留可診斷狀態，且資料維持建置中或不完整。
- 下一次同步能安全重頭開始或由已驗證 checkpoint 繼續。

是否採 checkpoint、page staging 或整批重抓由 design 決定；可觀察結果必須相同。

### DATA-01.6 零筆結果

零筆 ticks 只有在 coverage、query response 與 trading-date 規則共同支持時，才可
被記錄為已驗證的零筆結果。manifest 必須區分：

- 合法且確認完整的零筆資料
- coverage 不包含
- 查詢失敗或未完成
- 尚未確認是否應有資料

### DATA-01.7 驗收條件

`DATA-01` 至少必須由下列證據驗證：

- coverage 與單一商品範圍的 API integration tests。
- 多頁 cursor 走到終點且沒有截斷的測試。
- opaque cursor 原樣傳遞的測試。
- 中途失敗、重跑及 retry exhausted 的測試。
- 合法零筆與失敗空結果的區分測試。
- 尚未結束 trading date 不發布為完整的測試。
- 相同查詢第二次執行不重新下載完整本地資料的測試。

M2 必須以 Teralion 的 TWSE 2330 單日資料提供第一組端到端證據。

## 5. DATA-02：本地來源資料

### DATA-02.1 Partition identity

每個本地來源 partition 必須能明確識別：

- market
- symbol
- trading date
- source
- source format 集合

不同 identity 的資料不得因檔名碰撞、symbol 相同或日曆日期相同而互相覆寫。

### DATA-02.2 必要保存內容

每個 partition 至少必須保存或可由其 manifest 定位：

- market、symbol、trading date
- source format 與有效 `match_time`
- 正規化所需的 source tick payload
- query 範圍及使用的 endpoint／request identity
- 每個資料集合的筆數
- 內容 checksum 與 checksum algorithm/version
- cursor 是否完整走完
- 下載及驗證是否完成
- 對應的每日商品資料
- source schema 或可識別其版本的資訊

API request 中的秘密不得保存。若 request identity 需要表示認證來源，只能保存
不含 credential 的名稱或安全 reference。

### DATA-02.3 Atomic publish

下載及驗證完成前，資料必須保持暫存／建置中狀態。只有在必要檔案與 manifest
全部成功寫入且驗證通過後，才可原子發布為完整資料。

程序終止、磁碟空間不足或部分寫入不得留下看似完整的 partition。replay／backtest
只能開啟已發布且符合執行 policy 的資料。

### DATA-02.4 不可靜默覆寫

已發布的完整來源資料不得被靜默覆寫。

當相同 identity 再次同步時：

- checksum 與必要 provenance 相同者可以直接重用。
- 來源內容不同者必須停止或建立明確的新 revision。
- 修復、替換或刪除既有來源資料必須是使用者可見且可稽核的操作。
- replay cache 失效不得觸發來源資料的自動覆寫或重新下載。

revision 的實際 layout 由 design 決定，但任一 run 必須能指出使用哪一份來源內容。

### DATA-02.5 Offline reuse

資料準備完成後，replay／backtest 必須能在：

- 無網路
- 無 Teralion API key
- Teralion service 不可用

的環境中使用本地來源資料或有效 replay cache 完成執行。

回測路徑不得為了補齊資料而自動呼叫網路；缺少資料時必須停止並指示使用者另行
執行 sync。

### DATA-02.6 驗收條件

`DATA-02` 至少必須由下列證據驗證：

- partition identity 不碰撞的測試。
- 中斷或部分寫入不會發布為完整的測試。
- 已發布資料相同 checksum 直接重用的測試。
- 不同 checksum 不被靜默覆寫的測試。
- manifest 能重建 query、筆數、format 與 provenance 的檢查。
- 無網路及無 API key 的 replay／backtest integration test。

M2 必須證明第二次執行不重新下載 2330 單日來源資料。

## 6. DATA-03：完整性

### DATA-03.1 狀態集合

每個 market／trading date／symbol partition 至少具有下列狀態：

| 狀態 | 語意 |
| --- | --- |
| 缺少（missing） | 本地沒有可用或建置中的資料 |
| 建置中（building） | 同步、恢復或驗證尚未完成 |
| 完整（complete） | 所有必要頁面、payload、metadata 與 checksum 已通過驗證 |
| 不完整（incomplete） | 已知缺頁、缺範圍、缺必要資料或同步未能證明完整 |
| 損壞（corrupt） | 已保存內容無法解析、checksum 不符或違反已確認 invariant |

狀態必須持久保存或可由持久資料確定重建，不得只存在程序記憶體中。

### DATA-03.2 完整判定

partition 只有在下列條件均成立時才可標示 complete：

- query identity 與 coverage 已確認。
- 所有 cursor 頁面已走到明確終點。
- 必要 tick payload 與商品資料已保存。
- 筆數與 manifest 一致。
- checksum 可重新計算且一致。
- 必要 schema 可識別且內容可解析。
- trading date 歸屬可確認。

「沒有觀察到錯誤」不足以作為 complete 證據。

### DATA-03.3 執行 policy

正式 replay／backtest 預設：

- 接受 complete。
- 拒絕 missing、building、incomplete 及 corrupt。

使用者可以明確允許 incomplete 資料的 degraded run，但必須符合
[回播需求 `REPLAY-06`](replay.md)及[操作需求](operations.md)：

- 執行前明確啟用。
- plan 與 result 標示資料不完整。
- 記錄受影響的 partition、範圍與原因。
- 不得繞過時間單調性、event atomicity 或無前視保證。

corrupt 內容不得送入 normalizer 或 replay。explicit degraded run 只有在能明確
隔離損壞 partition／範圍時才可略過它，且必須在 execution plan 與 result 記錄
受影響範圍；無法安全隔離時仍必須停止。

### DATA-03.4 狀態轉換

狀態轉換必須可診斷且不誤導使用者：

```text
missing -> building -> complete
                    -> incomplete
complete ----------> corrupt   （後續驗證失敗）
incomplete --------> building  （重試或修復）
corrupt -----------> building  （明確重新取得或修復）
```

實作可以增加內部狀態，但對使用者至少要能映射至上述五種狀態。

### DATA-03.5 驗收條件

`DATA-03` 至少必須由下列證據驗證：

- 五種狀態的建立、保存與顯示測試。
- 缺頁、筆數不符、checksum 不符及無法解析的分類測試。
- complete 資料後續損壞可被重新驗證發現的測試。
- default run 拒絕非 complete partition 的測試。
- degraded run 清楚標示 incomplete 範圍的測試。
- 中斷恢復不會把 building 誤判為 complete 的測試。

## 7. DATA-04：回播快取

### DATA-04.1 Derived artifact

replay cache 必須是可刪除並由已驗證本地來源資料重建的衍生 artifact。

刪除、損壞或版本失效的 cache 不得造成：

- 來源資料遺失或修改。
- 自動重新下載仍完整的來源資料。
- 將 cache 當作無法追溯的唯一市場資料。

### DATA-04.2 Cache identity

每個 cache partition 至少綁定：

- market、symbol、trading date
- 來源 partition identity 及 source checksum
- event schema version
- normalizer／mapping version 或可判定相容性的 identity
- ordering rule version；若 cache 內容或索引依賴排序
- cache format version

任何會改變 domain event 語意或 canonical encoding 的版本都必須參與相容性判定。

### DATA-04.3 Granularity 與選取

cache 必須支援按 trading date 及 symbol 讀取，使回播器只開啟 strategy universe
需要的 streams。

cache layout 可以合併實體檔案，但不得因此要求讀取、解析或載入全市場 payload
才能取得單一商品。

### DATA-04.4 Invalidation 與 rebuild

遇到下列情況時，cache 必須失效或被拒絕：

- source checksum 不符。
- event schema 不相容。
- normalizer mapping 不相容。
- cache format 不相容。
- ordering dependency 不相容。
- cache checksum 不符或內容損壞。

系統必須能只使用本地完整來源資料重建 cache。重建結果在相同來源與版本下必須
具有相同事件內容、順序及 checksum。

### DATA-04.5 驗收條件

`DATA-04` 至少必須由下列證據驗證：

- cache 刪除後離線重建的 integration test。
- source checksum 或 schema version 改變時 cache 失效的測試。
- cache 損壞不會修改來源資料的測試。
- 按 symbol／trading date 只開啟必要 stream 的測試。
- 相同來源及版本重建出相同 checksum 的測試。
- 有效 cache 路徑不重新解析全部來源 JSON 的測試。

M2 必須以 TWSE 2330 單日資料證明 cache reuse 與 offline rebuild。

## 8. DATA-05：商品資料

### DATA-05.1 最小商品識別

第一版商品資料只要求能保存來源實際提供的：

- symbol
- market
- 商品種類
- 到期日；若可用
- 履約價；若可用
- 買賣權別；若可用

商品資料必須與適用的 trading date 或有效期間建立關聯，避免使用未來或不適用的
metadata。

### DATA-05.2 不得猜測缺漏值

若來源缺少 multiplier、root、名稱或其他欄位，系統必須表示為 absent／unknown，
不得依 symbol pattern、今日商品資料或市場慣例自行補值。

需要完成 fill、帳務或 P&L 的必要欄位可以由：

- 使用者明確設定
- 經識別的後續 reference-data source

補充，但必須記錄 value、source、適用範圍及版本。來源衝突時不得靜默選擇其中
一個。

### DATA-05.3 每日商品資料

每日商品資料的下載、保存、checksum、完整性及不可靜默覆寫規則，必須與對應的
tick partition 一樣可追溯。

若 ticks 完整但必要商品資料缺漏，partition 是否可用必須依工作類型區分：

- 只需事件回播時，可以在不臆測欄位的前提下執行。
- 需要缺漏 multiplier 等欄位的 P&L 時，必須取得明確補充設定，否則停止。

執行結果必須記錄實際使用的商品資料來源。

### DATA-05.4 TAIFEX trading date

TAIFEX 跨日資料必須依確認的 exchange trading-date 規則歸屬，不能單純以
`match_time` 的日曆日期切割。

trading-date 規則必須：

- 具版本或可識別來源。
- 可由 fixture 測試驗證日盤與夜盤邊界。
- 在無法判定時拒絕發布為 complete。
- 不以回測日後才知道的資料改變盤中 replay time。

### DATA-05.5 驗收條件

`DATA-05` 至少必須由下列證據驗證：

- optional instrument fields 的保存測試。
- 缺少 multiplier 不被猜測的測試。
- 使用者設定或 reference source 補值及 provenance 測試。
- 衝突 metadata 明確失敗的測試。
- 只回播與需要 P&L 時對缺漏欄位採不同 policy 的測試。
- TAIFEX 日盤／夜盤 trading-date fixture tests。

M3 必須提供 TAIFEX futures 的 trading date 與 multiplier provenance 證據；M4
先固定 TPEx regular-equity 的 market metadata；M5 再補 options／warrants 的
underlying、履約價、到期日及買賣權別。

## 9. 跨需求不變條件

任何資料設計與實作都必須維持：

1. 已驗證本地來源資料可跨多次 backtest 重用。
2. replay cache 是可刪除並重建的衍生 artifact。
3. 暫存、建置中、不完整及損壞資料不得偽裝成 complete。
4. 已發布來源資料不得被靜默覆寫。
5. cursor 必須走到明確終點，不得因單頁上限截斷。
6. backtest 預設不存取網路。
7. cache 失效不要求重新下載仍完整的來源資料。
8. unknown metadata 保持 unknown，補充值具有 provenance。
9. API key 不進入資料、manifest、log、結果或版本控制。
10. 相同來源內容與版本產生相同衍生事件及 checksum。

## 10. 驗證與里程碑摘要

| Requirement | 主要驗證層級 | 首次完整里程碑 |
| --- | --- | --- |
| DATA-01 | Teralion API integration tests | M2：2330 cursor download |
| DATA-02 | filesystem／repository integration tests | M2：local reuse and offline run |
| DATA-03 | state／negative／workflow tests | M2：verify and default rejection |
| DATA-04 | cache integration tests | M2：cache reuse and rebuild |
| DATA-05 | fixture／metadata tests | M2：TWSE basics；M3：TAIFEX fields |

正式 requirement、design、implementation 與 test mapping 由
[traceability matrix](../traceability.yaml)維護。

## 11. 待下游文件決定的事項

| 議題 | 文件 |
| --- | --- |
| Teralion endpoint、cursor 與 wire schema | [Teralion interface](../interfaces/teralion.md) |
| 各 market／format mapping | [TWSE](../interfaces/twse.md)、[TPEx](../interfaces/tpex.md)、[TAIFEX](../interfaces/taifex.md) interfaces |
| 本地 layout、manifest schema、publish 與 recovery | [data sync 設計](../design/data-sync.md) |
| 使用者資料檢查與修復操作 | [本地資料操作](../operations/local-data.md) |
| CLI workflow 與錯誤呈現 | [CLI 操作](../operations/cli.md) |
| Replay stream consumption | [replay engine 設計](../design/replay-engine.md) |
| Benchmark dataset 與 cache performance | [效能驗證](../verification/performance.md) |

下游文件不得以實作便利為由降低 cursor 完整性、來源不可變性、離線重用、資料
狀態可見性或 provenance 要求。
