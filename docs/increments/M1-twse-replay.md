# M1：TWSE 回播核心

## 1. 文件目的

本文件定義 `osmium-lab` 第一個可獨立驗證的增量：使用一份保存在版本控制中的
TWSE 2330 tick fixture，完成從來源資料正規化、依 `match_time` 回播、更新市場
狀態，到執行範例策略並產生可重現 checksum 的最小垂直切片。

M1 的目的不是建立完整回測平台，而是先驗證下列核心邊界：

- Teralion wire format 與 domain event 分離。
- `match_time` 是唯一的回播時間。
- 同一來源 tick 的內容以單一事件原子更新。
- 市場狀態只保存來源資料能支持的成交與五檔快照。
- 策略只能讀取事件及更新後的市場狀態。
- 相同 fixture 與版本會產生相同結果。

產品範圍、術語及優先順序以
[產品需求](../product-requirements.md)為準。

## 2. 交付結果

M1 完成時，專案必須能以單一明確的測試或範例入口執行下列流程：

```text
版本控制中的 2330 Teralion tick fixture
-> 驗證來源資料
-> 正規化為 QuoteSnapshot
-> 依 match_time 與固定 tie-break 排序
-> 推進 replay clock
-> 原子更新 MarketState
-> 呼叫 ExampleStrategy
-> 輸出 event checksum 與 final-state checksum
```

這個流程必須完全在本地執行，不需要 API key 或網路連線。

## 3. 範圍

### 3.1 包含

- 單一交易日。
- 單一市場：TWSE。
- 單一商品：`2330`。
- 一種經實際 fixture 確認的 Teralion TWSE quote `format`。
- 具有有效 `match_time` 的 tick。
- `QuoteSnapshot` domain event。
- 最佳五檔快照、可選成交、累計量與原始 flags。
- 單商品事件排序與回播。
- 單商品唯讀市場狀態。
- 一個不送出訂單的編譯期連結 Rust 範例策略。
- event stream checksum 與 final-state checksum。
- 正常資料及主要錯誤路徑的自動化測試。

### 3.2 不包含

- 呼叫 Teralion API、coverage 查詢、cursor 或資料下載。
- 本地來源資料目錄、manifest、checksum 驗證或回播快取。
- TPEx、TAIFEX、權證、選擇權或多商品合併。
- `BookSnapshot`、`TradeBatch` 或獨立 `MarketStatus`。
- order、fill、slippage、fee、tax、position 或 P&L。
- 動態 strategy loading、腳本策略或外部程序策略。
- CLI 與使用者設定檔的最終形式。
- 效能 benchmark 或正式效能門檻。
- 逐筆委託簿重建、queue position 或撮合過程推論。

以上能力分別屬於 M2 或後續里程碑。M1 可以提供測試或範例執行入口，不因此固定
正式 CLI。

## 4. 輸入 fixture

### 4.1 來源

fixture 必須來自實際取得的 Teralion TWSE 2330 歷史 tick，不得手工創造一個看似
合理但未經來源資料證實的 wire payload。

為避免洩漏或誤用資料，加入版本控制前必須確認：

- Teralion 的授權條款允許保存該測試資料。
- fixture 不包含 API key、request header 或其他秘密。
- fixture 的日期、market、symbol、format 與取得方式有文件紀錄。
- 若完整 tick 不適合提交，可以保存經核准的最小化 fixture；最小化只能刪除與
  測試無關的記錄，不得改寫欄位語意。

若目前沒有可合法提交的 fixture，M1 應先停在 fixture acquisition，不得用推測的
欄位 mapping 取代實際證據。

### 4.2 最小涵蓋情境

fixture 必須至少包含：

- 兩個以上不同的有效 `match_time`。
- 一次可觀察的五檔快照變化。
- 若該 format 提供成交，至少一筆具有來源成交資訊的 tick；若不提供，必須在
  TWSE interface 文件中明確記錄。
- 一次累計量變化。
- 至少一組可保存但不臆測解讀的來源 flags。

相同 `match_time` 的 tick 若能從真實資料取得，應納入 fixture。若樣本沒有此情境，
必須另以由真實 tick 複製後只調整 `match_time` 的排序測試資料驗證 tie-break，並
清楚標示該資料是測試衍生資料，不可當作來源格式證據。

### 4.3 Fixture metadata

fixture 旁必須記錄下列 metadata：

- `market`
- `symbol`
- `trading_date`
- Teralion `format`
- fixture 的內容 checksum
- fixture 是否為原始摘錄或測試衍生資料
- 已移除或遮蔽的欄位

metadata 的實際檔案格式在 M1 實作時以最簡單且可 review 的形式決定，不在本文件
固定。

## 5. Domain contract

### 5.1 Wire type 與 domain type

Teralion response 或 tick payload 的反序列化型別只代表來源格式，不得直接成為
策略或回播器的公開事件型別。

normalizer 負責：

1. 驗證 M1 支援的 market、format 與 symbol 表示。
2. 驗證 `match_time`、價格、數量及必要欄位。
3. 將已確認的來源欄位轉成 `QuoteSnapshot`。
4. 保留未知 flags 的原始值並產生可檢查的 warning。
5. 對未知 format 或無法安全解讀的 payload 回傳明確錯誤。

M1 execution plan 負責將 universe 限制為 TWSE `2330`；可重用的 domain type 或
normalizer 不得將 `2330` 硬編碼為唯一合法 symbol。

實際欄位 mapping 必須記錄在 [TWSE interface](../interfaces/twse.md)，並由 fixture
測試固定。未出現在真實 fixture 或來源文件中的欄位不得自行猜測。

### 5.2 `QuoteSnapshot`

M1 的 `QuoteSnapshot` 至少表達：

- market
- symbol
- source format
- `match_time`
- 最佳五檔買價與數量
- 最佳五檔賣價與數量
- 來源 tick 中可用的成交資訊
- 來源 tick 中可用的累計量
- 原始 flags

缺少的可選欄位必須表達為未知或不存在，不得以 `0`、前一筆資料或推算值代替。
價格與數量使用能避免浮點不確定性的表示方式；其精確 Rust 型別由
[market types 設計](../design/market-types.md)定義。

同一來源 tick 內的五檔、成交、累計量及 flags 必須組成一個 `QuoteSnapshot`，
不得拆成多個不同時間點的事件。

## 6. 排序與回播

### 6.1 Replay time

- `match_time` 是 M1 唯一的 replay time。
- API 接收時間、下載時間、檔案順序及本機時間不得推進 replay clock。
- 缺少或無效 `match_time` 的 tick 不得進入事件時間軸。
- replay clock 不得倒退。

### 6.2 Deterministic ordering

所有事件首先依 `match_time` 遞增排序。

相同 `match_time` 的事件使用一個具版本號的 deterministic ordering key。M1 的
ordering key 必須只由事件本身可用且跨執行穩定的資料組成，例如：

```text
market
-> symbol
-> source format
-> event kind rank
-> 可用的來源計數
-> canonical event fingerprint
```

不得使用記憶體位址、hash map iteration order、執行緒完成順序或本機隨機值。
完整 key 與 canonical encoding 必須在
[match-time ordering ADR](../architecture/decisions/0001-match-time-ordering.md)中定義。

若兩個事件的 ordering key 與 canonical payload 完全相同，它們在排序上視為等價，
但不得因此靜默刪除重複事件。

### 6.3 每個事件的處理順序

每個事件必須依下列順序處理：

```text
選出下一事件
-> 將 replay clock 推進至事件 match_time
-> 原子更新該商品的 MarketState
-> 將事件與更新後的唯讀狀態交給策略
-> 記錄策略輸出
```

策略 callback 執行期間不得取得下一事件或尚未發生的最終狀態。

## 7. MarketState contract

M1 每個商品只維護：

- 最新完整五檔快照
- 最近一筆可用成交資訊
- 最新累計成交量
- 最新原始 flags
- 最後 `match_time`
- 單調遞增的 state version

新的五檔直接完整取代舊的五檔。不得：

- 將缺少的價位沿用為仍存在的價位。
- 從連續快照推論新增、取消或排隊中的逐筆委託。
- 推論隱藏流動性或真實撮合順序。

一次 `QuoteSnapshot` 只產生一次原子狀態轉換及一次 state version 遞增。若事件驗證
失敗，狀態、版本與 replay clock 不得留下部分更新。

詳細 reducer 規則由 [market state 設計](../design/market-state.md)定義。

## 8. ExampleStrategy contract

M1 的範例策略用於證明策略邊界，而不是證明交易模型。它必須：

- 以 Rust trait 與編譯期連結實作。
- 宣告只需要 TWSE `2330`。
- 接收目前的 `QuoteSnapshot`。
- 讀取更新後的 `MarketState`。
- 產生至少一項只依目前及過去資料計算的可重現指標或觀察結果。
- 不修改事件、replay clock 或 market state。
- 不送出 order。

測試必須證明策略看到的 state version 已包含目前事件，且無法讀取下一事件。
詳細 API 由 [strategy API 設計](../design/strategy-api.md)定義。

## 9. 輸出與可重現性

### 9.1 必要輸出

每次 M1 執行至少產生：

- fixture checksum
- event schema version
- ordering rule version
- 正規化事件數
- warning 數及可檢查的 warning 內容
- first／last `match_time`
- event stream checksum
- final-state checksum
- 範例策略的可重現輸出

### 9.2 Checksum 邊界

event checksum 必須以排序後 domain events 的 canonical encoding 計算，不能直接對
來源 JSON 的空白、object key 順序或檔案路徑計算。

final-state checksum 必須以回播完成後 market state 的 canonical encoding 計算。

canonical encoding 必須：

- 不依賴平台、locale、timezone 或 Rust debug output。
- 對欄位順序、整數寬度、optional value 與 enum representation 有明確版本。
- 在 schema 或語意不相容時更新版本。

M1 可以先在測試輸出中呈現 checksum；正式 run manifest 屬於 M2。

## 10. 錯誤與 warning

下列情況必須拒絕執行，不得靜默略過：

- fixture 無法解析。
- market、symbol 或 trading date 不屬於 M1 execution plan。
- format 不在 M1 的明確支援清單。
- 缺少或無效的 `match_time`。
- 價格、數量或五檔結構不合法。
- 正規化後的事件時間倒退。
- canonical encoding 或 checksum version 不受支援。

未知但可安全保留的 flags 可以繼續處理，但必須：

- 保存原始值。
- 產生包含 market、symbol、`match_time`、format 的 warning。
- 將 warning 納入執行摘要。

M1 不提供允許損壞或不完整資料的降級模式；該設定屬於 M2。

## 11. 驗收情境

M1 至少要有下列自動化驗收證據：

| ID | 情境 | 預期結果 |
| --- | --- | --- |
| M1-AC-01 | 載入合法 2330 fixture | 所有支援 tick 正規化為預期的 `QuoteSnapshot` |
| M1-AC-02 | 檢查一筆含五檔、累計量、flags 及來源可用成交欄位的 tick | 單一事件原子保存所有可用內容 |
| M1-AC-03 | 以打亂順序的同一組事件執行多次 | event checksum 與 final-state checksum 完全相同 |
| M1-AC-04 | 兩個事件具有相同 `match_time` | 依具版本的 tie-break 產生固定順序 |
| M1-AC-05 | 新五檔快照到達 | 舊快照被完整取代，不重建逐筆委託 |
| M1-AC-06 | 策略處理事件 | 策略看到更新後狀態，但看不到下一事件 |
| M1-AC-07 | tick 缺少或具有無效 `match_time` | 執行失敗，錯誤包含定位資料所需的 context |
| M1-AC-08 | 遇到未知 format | 執行失敗，不猜測 mapping |
| M1-AC-09 | 遇到未知但可保留的 flags | 原值保留並產生可檢查 warning |
| M1-AC-10 | 在無網路及無 API key 的環境執行 | 完整流程成功且結果相同 |

固定的 golden checksum 只能因下列原因更新：

- 經 review 的產品需求變更。
- 經 fixture 或來源文件證實的 mapping 修正。
- 明確的 event schema、ordering rule 或 canonical encoding 版本變更。

更新 golden value 的變更必須同時說明原因及預期行為差異。

## 12. 需求追溯

M1 直接驗證下列產品需求：

| 產品需求 | M1 證據 |
| --- | --- |
| REPLAY-01 | fixture normalizer 與 `QuoteSnapshot` golden tests |
| REPLAY-02 | 打亂輸入與相同 `match_time` ordering tests |
| REPLAY-03 | MarketState reducer tests |
| REPLAY-04 | replay／strategy callback ordering test |
| REPLAY-06 | invalid time、unknown format 及 invalid value tests |
| STRAT-01 | 唯讀 ExampleStrategy integration test |
| OPS-02（部分） | schema／ordering version、事件數、warning 與 checksum 輸出 |
| NFR-01 | repeated-run golden checksum test |
| NFR-03（部分） | versioned event、ordering 與 canonical encoding |

DATA-01 至 DATA-04、SIM-01、SIM-02 及完整 OPS-01／OPS-02 不由 M1 驗收，留待 M2。
正式 machine-readable mapping 由 [traceability matrix](../traceability.yaml)維護。

## 13. 完成條件

只有在下列條件全部成立時，M1 才視為完成：

- 合法且有來源紀錄的 2330 fixture 已加入測試資產。
- TWSE interface 文件記錄該 fixture 使用的 format 與欄位 mapping。
- wire type 與 domain type 在程式結構及 API 上分離。
- `QuoteSnapshot`、MarketState、replayer 與 ExampleStrategy 已串成完整流程。
- 本文件的所有驗收情境都有自動化測試或明確對應的測試證據。
- event checksum 與 final-state checksum 已固定為 golden result。
- focused tests、`cargo fmt --check` 及 workspace `cargo test` 成功。
- 需求、設計、ADR、測試與實作之間可以透過
  [traceability matrix](../traceability.yaml)追溯。
- 不需要網路、Teralion API key 或 M2 元件即可重跑。

## 14. 建議實作切分

M1 應維持小型、可 review 的 commit，建議依序：

1. 加入合法 fixture、metadata 與 TWSE format mapping 測試。
2. 定義最小 market types、`QuoteSnapshot` 及 canonical encoding。
3. 實作 normalizer 與錯誤 context。
4. 實作 snapshot-based MarketState reducer。
5. 實作 `match_time` ordering、replay clock 與 checksum。
6. 加入唯讀策略 API 及 ExampleStrategy。
7. 串接端到端 golden test，補齊 traceability。

每一步先執行該模組的 focused tests；完成整個 M1 後再執行 workspace validation。
