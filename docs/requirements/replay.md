# 回播需求

## 1. 文件目的

本文件將[產品需求](../product-requirements.md)中的 `REPLAY-01` 至 `REPLAY-06`
細化為可設計、實作與驗證的系統需求。

本文件定義系統必須呈現的行為，不固定：

- crate 或 module 邊界
- Rust struct、trait 或 function signature
- 儲存格式與目錄布局
- 排序演算法或資料結構
- checksum algorithm
- CLI syntax

上述選擇分別由 architecture、design 與 ADR 文件記錄。若下游文件與本文件衝突，
以產品需求及本文件為準。

## 2. 範圍與邊界

回播從已驗證的本地來源資料或由其建立的回播快取開始，依 execution plan 選取
事件，推進 replay clock，更新市場狀態，再將目前事件與更新後狀態提供給策略。

```text
已驗證的本地資料
-> 正規化 domain events
-> 選取 strategy universe streams
-> deterministic merge
-> replay clock
-> MarketState
-> strategy callback
-> strategy output
```

本文件涵蓋：

- 標準事件的共同語意與原子性
- replay time 與 deterministic ordering
- 市場狀態的可觀察內容與更新規則
- 每個事件的處理順序及無前視保證
- 多商品 stream selection 與合併要求
- 回播錯誤、warning 及降級執行規則

本文件不涵蓋：

- Teralion API、cursor、下載及本地資料生命週期，見
  [資料需求](data.md)。
- 策略生命週期、參數及下單意圖，見[策略需求](strategy.md)。
- fill、fee、position 及 P&L，見[模擬需求](simulation.md)。
- 具體 event type、reducer 或 merge API，見 design 文件。

## 3. 通用定義

### 3.1 Source tick

`source tick` 是資料來源以單一紀錄提供的最小原子 payload。不同 market 或
`format` 的欄位及語意可以不同。

正規化層不得為了建立較方便的 domain event 而改變 source tick 內欄位的時間
關係，也不得將來源沒有提供的資料推算成已知值。

### 3.2 Domain event

`domain event` 是回播器與策略使用的標準事件。它與 Teralion wire type 分離，
只表達經來源 fixture 與 interface 文件確認的語意。

### 3.3 Replay time

`match_time` 是唯一的 replay time。API request time、資料取得時間、檔案修改時間、
本機時間及處理完成時間皆不得作為 replay clock。

### 3.4 Event acceptance

事件只有在完成必要的 schema、時間、價格、數量及來源完整性驗證後才可進入
timeline。`accepted event` 指已通過這些檢查、可安全套用至市場狀態的事件。

### 3.5 Unknown 與 absent

- `absent`：來源 format 明確不提供該欄位，或該筆合法 payload 明確未帶該值。
- `unknown`：來源帶有值，但目前無法安全解讀其語意。
- `invalid`：值違反已確認的來源格式或 domain invariant。

`absent` 與 `unknown` 不得以 `0`、空字串、上一事件值或推算值偽裝成已知資料。
`invalid` 必須依 `REPLAY-06` 處理。

## 4. REPLAY-01：標準事件

### REPLAY-01.1 Event envelope

每個 domain event 必須至少包含：

- market
- symbol
- source format
- `match_time`
- event payload

event schema 必須具有明確版本。版本不相容時，系統必須拒絕讀取或要求由已驗證
來源資料重建回播快取，不得將不相容事件當成目前版本處理。

### REPLAY-01.2 第一版事件集合

第一版允許下列事件：

| Event | 必要語意 |
| --- | --- |
| `QuoteSnapshot` | 完整最佳五檔，以及同一 source tick 中可用的成交、累計量與 flags |
| `BookSnapshot` | 完整最佳五檔，以及可由同一 snapshot 直接取得的一檔 |
| `TradeBatch` | 同一 source tick 中的一筆或多筆成交，以及可用的累計資訊 |
| `IndicativeOpeningAuction` | 開盤試算的 indicative price／quantity／book；不是 actual trade |
| `IndicativeClosingAuction` | 收盤試算的 indicative price／quantity／book；不是 actual trade |

不屬於此集合的來源資料不得臨時建立未版本化 event kind。新增 event kind 必須先
更新需求或設計、event schema version、normalizer fixture tests 及 traceability。

第一版不提供 standalone status event。來源 tick 的 flags／status 是該
`QuoteSnapshot`、`BookSnapshot`、`TradeBatch` 或 auction event 的 annotations，必須
與原始 observation 原子更新。auction event 不得被當作 actual trade、cumulative
volume 或 fill evidence。

### REPLAY-01.3 Source fidelity

normalizer 必須遵守：

- 只轉換由實際 source fixture 與 interface 文件確認的欄位。
- 保留 market、symbol、format 與 `match_time` 的來源身分。
- 不從五檔 snapshot 反推逐筆委託、取消、queue position 或 hidden liquidity。
- 不由低粒度資料產生來源不存在的高粒度資料。
- 未知欄位或 flags 保留原值，且不得被靜默解讀。

各 market／format 的欄位 mapping 必須由保存的 fixture 建立固定測試。相同 mapping
的輸入在相同 schema version 下必須產生相同 canonical event。

Teralion 的 TAIFEX `close`／`stats` 及 Feed Archive session-stat 記錄不屬於第一版
domain event 集合：

- normalizer 不為其產生 timeline event。
- replay cache 與 replayer 不需要開啟這些記錄。
- 已存在的 raw source payload 可以保留，以便未來需求重新評估。
- 未來若需要 settlement、open interest 或 order counters，必須先定義具體用途及
  timing semantics，不得重新加入模糊的通用統計事件。

### REPLAY-01.4 Event atomicity

同一 source tick 中可同時觀察到的成交、五檔、累計量及 flags 必須保留在同一
domain event 的一次原子狀態轉換中。

系統不得：

- 將同一 tick 的欄位分配到人造的不同 `match_time`。
- 讓策略看到只套用同一 tick 部分欄位的中間狀態。
- 在事件驗證或狀態更新失敗後留下部分更新。

若一個 source tick 合法映射成包含多筆成交的 `TradeBatch`，batch 內資料仍視為
單一事件原子更新。batch 內順序只有在來源提供且語意明確時才可宣稱為來源順序。

### REPLAY-01.5 驗收條件

`REPLAY-01` 至少必須由下列證據驗證：

- 每個支援 market／format 的 golden fixture normalization tests。
- optional、absent、unknown 與 invalid 欄位測試。
- 同一 tick 的成交、五檔、累計量及 flags 原子更新測試。
- unknown format 拒絕測試。
- event schema 相容與不相容版本測試。

M1 由 `M1-AC-01`、`M1-AC-02`、`M1-AC-08` 及 `M1-AC-09` 提供第一組證據。

## 5. REPLAY-02：事件排序

### REPLAY-02.1 Primary ordering

所有 accepted events 的第一排序鍵必須是 `match_time`，並以時間遞增方式處理。

- replay clock 不得倒退。
- 較晚的事件不得在較早事件之前提供給策略。
- 來源檔案順序不得覆蓋 `match_time` ordering。
- 多 market 或多 symbol 不改變此規則。

`match_time` 的精確表示、精度、時區及跨日 encoding 必須在 design 文件中定義，
並保留來源能提供的精度。TAIFEX trading date 的歸屬不得只以日曆日期推算。

### REPLAY-02.2 Deterministic tie-break

相同 `match_time` 的事件必須使用固定且版本化的 deterministic tie-break。

tie-break 只能使用事件本身可用且跨執行穩定的資料，例如：

- market
- symbol
- source format
- event kind
- 來源提供的計數
- canonical event fingerprint

tie-break 不得使用：

- 記憶體位址
- hash map iteration order
- worker 或 thread 完成順序
- 檔案發現順序
- 未固定 seed 的隨機值
- 本機 clock、locale 或 timezone

完整 ordering key、欄位比較方式與 canonical fingerprint encoding 必須由
[match-time ordering ADR](../architecture/decisions/0001-match-time-ordering.md)記錄。

### REPLAY-02.3 Determinism boundary

相同 source data、event schema、ordering rule 及 execution plan 必須產生：

- 相同事件數及 warning 集合
- 相同事件順序
- 相同 event stream checksum
- 相同 final-state checksum
- 相同策略 callback sequence

輸入檔案枚舉順序或可安全改變的並行處理方式不得改變結果。

若兩個事件的完整 ordering key 與 canonical payload 完全相同，兩者在排序上可以
視為等價，但系統不得因而靜默刪除其中一個。完全相同事件的相對位置不得影響可
觀察結果。

### REPLAY-02.4 語意限制

deterministic tie-break 只保證平台重跑順序一致，不代表：

- 交易所真實的全域封包順序
- 不同商品間的因果關係
- 相同 `match_time` 事件的真實先後
- 逐筆委託或成交的排隊順序

使用者可檢查的執行摘要必須記錄 ordering rule version，避免將平台排序誤認為
來源保證。

### REPLAY-02.5 驗收條件

`REPLAY-02` 至少必須由下列證據驗證：

- 將同一組輸入以多種順序排列後，事件順序與 checksum 相同。
- 不同 symbol 及 event kind 具有相同 `match_time` 的測試。
- 相同 `match_time`、不同 fingerprint 的測試。
- 完全重複事件不被靜默去重的測試。
- ordering rule version 變更時的相容性或拒絕行為測試。
- replay clock 永不倒退的 invariant test。

M1 由 `M1-AC-03` 及 `M1-AC-04` 提供單商品證據；多商品證據在 M3 補足。

## 6. REPLAY-03：市場狀態

### REPLAY-03.1 State ownership

系統必須為 execution plan 中每個商品維護獨立的市場狀態。市場狀態只能由回播器
依 accepted event 更新；策略及模擬層只能讀取，不得修改。

尚未收到對應事件的欄位必須明確表示為 unavailable，不得預填未來資料、當日最終
統計或由其他商品推算的值。

### REPLAY-03.2 最小狀態

每個商品的市場狀態至少包含來源可支持的：

- 最新完整五檔 snapshot
- 最近成交或成交 batch
- 累計成交量
- 最新 flags
- 最後 `match_time`
- state version

不適用或來源未提供的欄位必須維持 absent／unavailable。不同 market／format 可以
提供不同欄位，但共同狀態不得假裝所有來源具有相同精度。

### REPLAY-03.3 Snapshot semantics

新的完整五檔事件必須完整取代同商品的舊五檔 snapshot。

系統不得：

- 將新 snapshot 未包含的價位沿用為仍有效。
- 將 snapshot 差異解讀為逐筆新增、修改或取消委託。
- 由顯示量推論真實可成交量、queue position 或 hidden liquidity。
- 在沒有來源證據時合成第六檔以上或更細粒度 book。

可由完整 snapshot 直接取得的一檔 view 可以作為衍生讀取結果，但不得成為較 snapshot
更高精度的獨立市場事實。

### REPLAY-03.4 Atomic state transition

每個 accepted event 必須形成一次原子狀態轉換：

- event 所帶的所有已知欄位一起套用。
- `last_match_time` 更新為目前事件的 `match_time`。
- state version 依固定規則單調遞增。
- 策略只能看到更新完成前或更新完成後的狀態，不得看到中間狀態。

若事件無法完整驗證或套用，該事件不得對市場狀態、state version 或策略造成部分
可觀察效果。

同一 `match_time` 的多個事件仍依 `REPLAY-02` 逐一形成狀態轉換。策略處理目前
事件時，可以看到 tie-break 中較早事件的結果，不可看到較晚事件的結果。

### REPLAY-03.5 Unknown handling

未知 format 的原始 source payload 必須留在可檢查的本地來源資料中，但該 payload
不得進入一般 state reducer。default mode 必須停止；explicit degraded mode 只能
記錄並略過，不得猜測 mapping。

未知但可安全保存的 flags 或欄位必須：

- 保留原始值或可無損重建的 representation。
- 產生可檢查 warning。
- 不觸發未經文件確認的市場狀態語意。

### REPLAY-03.6 驗收條件

`REPLAY-03` 至少必須由下列證據驗證：

- 第一個事件前的 unavailable state 測試。
- 完整五檔取代舊 snapshot 的 reducer test。
- 成交、累計量及 flags 的更新測試。
- 每個 accepted event 只造成一次 atomic version transition 的測試。
- reducer 失敗不留下部分狀態的測試。
- unknown value 保留且不被推論的測試。
- event stream checksum 相同時 final-state checksum 相同的測試。

M1 由 `M1-AC-02`、`M1-AC-05`、`M1-AC-06` 及 `M1-AC-09` 提供
`QuoteSnapshot` 證據。

## 7. REPLAY-04：事件處理順序與無前視

### REPLAY-04.1 Processing sequence

每個事件必須依下列順序處理：

```text
選出 deterministic ordering 中的下一事件
-> 推進 replay clock 至事件 match_time
-> 原子更新該商品 MarketState
-> 將目前事件與更新後的唯讀狀態提供給策略
-> 處理並記錄策略輸出
```

下一事件只有在目前事件的策略輸出完成必要處理後才可對策略可見。

### REPLAY-04.2 Strategy view

策略處理目前事件時，只能讀取：

- 目前事件
- deterministic ordering 中已處理的事件所形成的狀態
- 目前事件完成原子更新後的狀態
- execution plan 明確允許的靜態設定與 reference data

策略不得讀取：

- 下一事件或其 `match_time`
- 尚未處理事件形成的市場狀態
- 尚未完成 bar 的最終值
- 盤後或日後才知道的統計
- 回播完成後才計算的 final state 或 result

同一 `match_time` 不構成例外：策略只能看到 tie-break 中目前及較早的事件。

### REPLAY-04.3 Strategy output boundary

策略輸出不得追溯修改目前或歷史 market event。下單意圖的成交資格與最早可判定
事件由 `SIM-01` 規範；回播器不得讓同一事件所產生的訂單使用該事件尚未經過策略
前即可取得的未來資訊。

並行化可以預先讀取或正規化資料，但不得改變策略可觀察的 processing sequence。

### REPLAY-04.4 驗收條件

`REPLAY-04` 至少必須由下列證據驗證：

- strategy callback 看到目前事件更新後 state version 的測試。
- strategy callback 無法取得下一事件的 API boundary test。
- 相同 `match_time` 多事件逐一可見的測試。
- 尚未完成 bar 或日終統計不會提前出現的測試。
- 並行與單執行緒模式產生相同 callback sequence 的測試；若第一版沒有並行模式，
  此項在加入並行時成為必要驗收。

M1 由 `M1-AC-06` 提供單商品、無下單情境的第一組證據。

## 8. REPLAY-05：多商品與資源使用

### REPLAY-05.1 Explicit universe

第一版 execution plan 必須使用明確的 market／symbol 清單。回播器只可開啟該
universe 及指定 trading date 所需的 event streams。

系統不得因本地資料存在其他商品而：

- 開啟或掃描不相關商品的 replay payload。
- 將不相關商品事件加入 merge。
- 將不相關商品狀態提供給策略。

依 metadata 動態選擇近月 futures、options、warrants 或執行期間新增 symbol 不屬於
第一版需求。

### REPLAY-05.2 Streaming merge

回播器必須串流合併已依規則排序的商品／交易日 streams，並遵守 `REPLAY-02` 的
全域 deterministic ordering。

實作不得要求：

- 將全市場資料載入記憶體。
- 將完整回測期間的所有事件同時載入記憶體。
- 每次回測重新下載已驗證的來源資料。
- 每次回測重新解析所有原始 JSON，而忽略有效的 replay cache。

可以使用 bounded buffering、prefetch 或並行 I/O，但結果不得因 buffer size、
worker count 或 stream discovery order 改變。

### REPLAY-05.3 Stream contract

每個輸入 stream 必須能識別：

- market
- symbol
- trading date
- event schema version
- ordering rule version 或相容性
- 來源完整性狀態

回播器必須驗證 stream 事件時間未違反其宣告的 ordering。發現 stream 內時間倒退
或跨 stream 不相容版本時，依 `REPLAY-06` 處理。

來源 checksum、cache invalidation 及資料完整性狀態的保存方式由 `DATA-02` 至
`DATA-04` 規範。

### REPLAY-05.4 驗收條件

`REPLAY-05` 至少必須由下列證據驗證：

- universe 外商品 stream 不被開啟的 spy／integration test。
- 多商品交錯 `match_time` 的 deterministic merge test。
- 不同 stream discovery order 產生相同結果的測試。
- 以 bounded memory 處理大於 buffer 的資料集之測試或 benchmark 證據。
- 有效 replay cache 路徑不重新解析全部來源 JSON 的 integration test。

M1 只提供單一 TWSE 2330 stream 的基礎證據；多商品 merge 在 M3 完整驗收。

## 9. REPLAY-06：錯誤、warning 與降級執行

### REPLAY-06.1 不得靜默繼續的情況

至少下列情況不得靜默繼續：

- 缺少、無法解析或無效的 `match_time`
- stream 或全域事件時間排序錯誤
- 不支援或不相容的 source format
- 不支援或不相容的 event schema／ordering version
- 不合法的價格、數量、五檔結構或 event payload
- 來源資料不完整、損壞或 checksum 不符
- event 的 market、symbol 或 trading date 不屬於 execution plan
- 無法安全維持 event atomicity 或 state atomicity

### REPLAY-06.2 Default behavior

正式回測預設必須停止，不得略過上述資料後產生看似完整的結果。

錯誤必須盡可能包含：

- market
- symbol
- trading date
- source format
- `match_time`；若無效則包含原始值或明確標示缺少
- stream、fixture 或本地資料的安全識別資訊
- 失敗原因
- 建議的處理方式

錯誤及 log 不得包含 API key 或其他秘密。

### REPLAY-06.3 Warning

只有在資料仍符合已確認 schema，且未知內容可以無損保存、不影響已知欄位安全
解讀時，才可以 warning 繼續。例如未知但可保存的 flags。

warning 必須：

- 包含足以定位 market、symbol、trading date、format 與事件的 context。
- 記錄原始未知值或安全 representation。
- 納入執行摘要及 warning count。
- 在相同輸入下產生 deterministic warning 集合。

warning 不得把 invalid value、unknown format 或資料完整性錯誤降成一般通知。

### REPLAY-06.4 Explicit degraded mode

M2 起可以提供明確的 degraded mode，但必須符合：

- 由使用者在執行前明確啟用，不得自動推測。
- execution plan 及結果清楚標示不完整或降級。
- 記錄被拒絕、略過或無法解讀的資料範圍與原因。
- 輸出資料 checksum、事件數、warning 及略過數量。
- 相同資料與設定仍產生 deterministic result。
- 不得繞過 event atomicity、時間單調性或無前視保證。

若某錯誤無法在維持上述 invariant 下隔離，系統即使在 degraded mode 仍必須停止。

M1 不提供 degraded mode；任何 `REPLAY-06.1` 錯誤都必須停止該次執行。

### REPLAY-06.5 驗收條件

`REPLAY-06` 至少必須由下列證據驗證：

- 各類 invalid time、format、value、checksum 及 completeness error tests。
- error context 包含可取得定位欄位的測試。
- 未知但可保存 flags 產生 warning 的測試。
- warning count 與內容可重現的測試。
- default mode 拒絕不完整資料的 integration test。
- degraded mode 明確標示、記錄略過範圍且維持 invariant 的 integration test。

M1 由 `M1-AC-07`、`M1-AC-08` 及 `M1-AC-09` 提供不含 degraded mode 的證據。

## 10. 跨需求不變條件

任何設計與實作都必須維持：

1. `match_time` 是唯一 replay time。
2. deterministic tie-break 不宣稱真實市場全域順序。
3. 同一 source tick 不產生策略可見的部分狀態。
4. 五檔 snapshot 取代舊 snapshot，不重建逐筆委託。
5. 策略只看見目前及過去事件形成的唯讀狀態。
6. universe 外 stream 不被回播器開啟。
7. 效能或並行最佳化不改變事件、狀態或策略輸出。
8. unknown、invalid 與 degraded data 不得被靜默當成完整已知資料。
9. derived replay cache 可以失效並重建；已驗證來源資料不因 cache 失效而重新下載。
10. 第一版已知但排除的 `close`／`stats` source records 不進入 event timeline 或
    MarketState。

## 11. 驗證與追溯摘要

下表的 `M1-AC-*` 對應
[M1：TWSE 回播核心](../increments/M1-twse-replay.md)中的驗收情境。

| Requirement | 主要驗證層級 | M1 證據 | 後續證據 |
| --- | --- | --- | --- |
| REPLAY-01 | fixture unit／golden tests | M1-AC-01、02、08、09 | M3 TAIFEX、M4 market formats |
| REPLAY-02 | unit／property／system tests | M1-AC-03、04 | M3 multi-symbol merge |
| REPLAY-03 | reducer unit tests | M1-AC-02、05、06、09 | M3 trade／book state |
| REPLAY-04 | strategy integration tests | M1-AC-06 | M2 order intent、M3 multi-symbol |
| REPLAY-05 | I/O integration／benchmark | single-stream baseline | M2 cache、M3 multi-stream |
| REPLAY-06 | negative／integration tests | M1-AC-07、08、09 | M2 integrity／degraded mode |

正式的 requirement、design、implementation 與 test mapping 由
[traceability matrix](../traceability.yaml)維護。

## 12. 待下游文件決定的事項

下列事項在不違反本文件的前提下，由對應文件決定：

| 議題 | 文件 |
| --- | --- |
| 完整 ordering key、event rank 與 fingerprint | [match-time ordering ADR](../architecture/decisions/0001-match-time-ordering.md) |
| Event 與時間的 Rust representation | [market types 設計](../design/market-types.md) |
| Snapshot reducer 與 state version 規則 | [market state 設計](../design/market-state.md) |
| Stream merge、clock 與 callback orchestration | [replay engine 設計](../design/replay-engine.md) |
| Strategy read-only API | [strategy API 設計](../design/strategy-api.md) |
| Source／cache integrity 與 degraded data 設定 | [資料需求](data.md)及[data sync 設計](../design/data-sync.md) |
| Run summary、CLI error presentation | [操作需求](operations.md)及[CLI 操作](../operations/cli.md) |

下游文件不得以「尚待設計」為由放寬本文件的時間、原子性、無前視、資料精度或
determinism 邊界。
