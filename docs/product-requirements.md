# osmium-lab 產品需求

## 1. 文件目的

本文件定義 `osmium-lab` 的產品目標、系統範圍、高階需求與驗收方向，是後續 V-model 討論的需求基準。

後續設計與實作必須能追溯到本文件，但本文件不提前固定 crate、檔案格式、目錄布局或完整 API。

需求關鍵字：

- **必須**：對應里程碑不可缺少。
- **應該**：原則上要完成；若不做，必須說明影響。
- **可以**：可選設計，不構成驗收條件。

---

## 2. 產品定位

`osmium-lab` 是以 Rust 建立、使用 Teralion 歷史行情的台灣市場回播與回測平台。

核心目標是：

> 依 `match_time` 重播歷史上可觀察的成交與行情快照，讓策略在不使用未來資料的前提下執行，並以簡單、明確的模型估算交易結果。

產品優先順序：

1. **容易使用**：一份設定即可準備資料並執行回測。
2. **執行快速**：資料下載一次後在本地重複使用，只讀需要的商品。
3. **結果可重現**：相同資料與設定得到相同結果。
4. **符合資料精度**：不宣稱資料無法支持的撮合或排隊精度。
5. **適應台灣市場**：保留市場、商品、五檔、成交與來源 flags 的差異。

---

## 3. Teralion 資料能力

本系統以 Teralion 實際可取得的 tick 為能力上限。

目前樣本顯示：

- TWSE／TPEx quote 提供最佳五檔快照、可選成交、累計量、盤中價格與狀態 flags。
- TAIFEX tick 提供成交或成交批次、完整五檔快照，以及 TAIFEX-specific
  `close`／`stats` 記錄。
- 同一商品會出現不同 `format`，其欄位與語意可能不同。
- 商品資料可提供 symbol、市場、種類、到期日等資訊，但部分欄位可能缺漏。
- Teralion Feed Archive 的 session-stat 記錄只有 `received_at` 與 `seq`，沒有可用的
  `match_time`。
- 資料不是逐筆委託資料，無法得知每張委託、真實排隊順位、隱藏流動性或完整撮合過程。

因此：

- 回播以成交、完整五檔快照與可辨識的狀態為主。
- 只有具有有效 `match_time` 的資料才能進入事件時間軸。
- 沒有 `match_time` 的資料只能作為參考資料或回測結果附件，不得在盤中時間軸任意插入。
- 第一版不將 Teralion `close`／`stats` 記錄正規化為 domain event，也不要求為回測
  下載或建立其 replay cache；既有來源資料可以保留 raw payload，但 replayer 忽略。
- 系統不從五檔快照反推逐筆委託或交易所撮合過程。
- 不完整或未知欄位保留為未知，不自行猜測。

---

## 4. 系統範圍

### 4.1 目標商品

專案目標支援：

- TWSE 股票
- TPEx 股票
- TWSE／TPEx 權證
- TAIFEX 期貨
- TAIFEX 選擇權
- 處置證券的歷史行情

交付順序為：

1. TWSE 股票
2. TAIFEX 期貨
3. TPEx 股票
4. 權證與選擇權

處置證券仍以一般商品回播。若來源提供特殊狀態或不同撮合節奏，平台保存並呈現該資料，但不重新模擬處置規則。

### 4.2 第一版不支援

- 盤中零股、盤後零股、盤後定價與鉅額交易
- 即時交易
- 完整交易所撮合引擎
- 逐筆委託簿重建
- 精確委託排隊順位
- 隱藏流動性推論
- 瞬間價格穩定措施或處置撮合規則的重算
- 由低粒度資料產生不存在的高粒度資料
- 回測時自動存取網路

---

## 5. 使用流程

使用者提供：

- 回測日期
- 商品清單
- 策略與參數
- 資料與模擬設定

平台執行：

```text
建立執行計畫
-> 檢查本地資料
-> 下載缺少的 Teralion tick
-> 驗證並建立本地回播資料
-> 依 match_time 合併事件
-> 更新市場狀態
-> 執行策略
-> 模擬成交與帳務
-> 輸出結果與執行摘要
```

資料同步和回測必須可以分開執行。資料準備完成後，回測必須能在斷網環境執行。

---

## 6. 高階架構

```text
Teralion Feed Archive
        |
        v
資料同步與驗證
        |
        v
本地來源資料
        |
        v
標準事件 / 可重建回播快取
        |
        v
市場回播器 -> 市場狀態 -> 策略
                        |
                        v
                    基礎成交模擬
                        |
                        v
                    帳務與回測結果
```

責任邊界：

- **資料層**：呼叫 Teralion、處理 cursor、保存資料並檢查完整性。
- **正規化層**：將各 market／format 轉為少量標準事件。
- **回播層**：排序事件、推進時鐘並更新市場狀態。
- **策略層**：讀取市場狀態並產生指標或下單意圖。
- **模擬層**：以明確且保守的規則估算成交、費用、部位與損益。

---

## 7. 資料需求

### DATA-01 資料取得

系統必須支援 Teralion 的：

- coverage 與單一商品可用範圍
- ticks
- 每日商品資料
- opaque keyset cursor

下載必須走完所有 cursor，不得因單頁上限而截斷。

第一版只同步已結束的交易日，不處理當日增量資料。

### DATA-02 本地資料

本地資料以 `market + trading_date + symbol` 管理，至少保存：

- market、symbol、format 與 `match_time`
- 回播需要的 tick payload
- 查詢範圍、筆數與 checksum
- 下載是否完成
- 對應的商品資料

已完成的資料不得被靜默覆寫。同步失敗的暫存資料不得被視為可回測資料。

### DATA-03 完整性

每個商品／交易日必須至少區分：

- 缺少
- 建置中
- 完整
- 不完整
- 損壞

正式回測預設拒絕不完整或損壞的資料。使用者可以明確允許降級執行，但結果必須標示資料不完整。

### DATA-04 回播快取

系統可以從本地來源資料建立較快的回播快取。快取必須：

- 可刪除並重建
- 綁定來源 checksum 與事件 schema 版本
- 支援按交易日及商品讀取
- 失效時不需要重新下載來源資料

### DATA-05 商品資料

商品資料以 Teralion 實際提供的欄位為主。第一版只要求能識別：

- symbol
- market
- 商品種類
- 可用的到期日、履約價與買賣權別

缺少 multiplier、root 或名稱時不得自行猜測。需要計算損益的必要欄位，可以由明確設定或後續 reference-data source 補充，並記錄來源。

TAIFEX 跨日資料必須依正確的 trading date 回播，不可單純依日曆日期切割。

---

## 8. 事件與回播需求

### REPLAY-01 標準事件

第一版只需要少量事件類型：

- `QuoteSnapshot`：最佳五檔、可選成交、累計量及 flags
- `BookSnapshot`：完整五檔及可用的衍生一檔
- `TradeBatch`：一筆或多筆成交及可用的累計資訊

第一版不建立獨立的市場狀態事件。來源 tick 內可明確辨識的 flags／status 必須保留
在同一個 quote、book 或 trade event 中，不能拆成另一個時間點。

每個事件至少包含：

- market
- symbol
- format
- `match_time`
- payload

同一 Teralion tick 內的成交、五檔與 flags 必須作為一個事件原子更新，不任意拆成不同時間點。

### REPLAY-02 排序

所有事件第一排序鍵都是 `match_time`。

相同 `match_time` 時，使用固定且版本化的 deterministic tie-break。tie-break 只使用事件本身可用的市場、商品、format、事件種類、來源計數或內容 fingerprint。

這項規則只保證重跑順序一致，不代表真實市場的全域封包順序。

### REPLAY-03 市場狀態

每個商品只維護 Teralion 資料可以支持的狀態：

- 最新五檔快照
- 最近成交或成交批次
- 累計成交量
- 最新 flags
- 最後 `match_time`
- 狀態版本

新的五檔事件直接取代舊快照。系統不維護或推論逐筆委託。

未知 format 或 flags 不得被靜默解讀；系統應保留原值並提出警告。

### REPLAY-04 處理順序

每個事件依下列順序處理：

```text
選出下一事件
-> 推進 replay clock
-> 原子更新市場狀態
-> 策略讀取事件與更新後狀態
-> 處理策略輸出
```

策略只能看到目前及過去事件，不能看到下一個 tick、尚未完成的 bar 或日後才知道的統計資料。

### REPLAY-05 多商品與效能

回播器必須：

- 只開啟使用者指定的商品
- 串流合併已排序事件
- 不要求將全市場或完整回測期間載入記憶體
- 不在每次回測重新下載或解析全部原始 JSON

第一版 universe 使用明確 symbol 清單。依 metadata 動態選擇近月期貨、選擇權或權證屬於後續功能。

### REPLAY-06 錯誤

遇到以下情況不得靜默繼續：

- 缺少或無效的 `match_time`
- 時間排序錯誤
- 不支援的 format
- 不合法的價格或數量
- 資料不完整或 checksum 不符

系統必須依設定停止，或以明確的降級模式繼續並記錄警告。

---

## 9. 策略與回測需求

### STRAT-01 策略

策略必須能：

- 宣告商品清單與參數
- 接收標準事件
- 讀取唯讀市場狀態
- 產生自訂指標與下單意圖
- 接收模擬訂單及成交結果

策略不得修改市場狀態、回播時鐘或歷史事件。

第一版策略使用 Rust trait 與編譯期連結。

### SIM-01 基礎成交模型

第一版只提供保守且容易理解的成交模型，不模擬真實排隊：

- 策略在某事件產生的訂單，最早從下一個可用事件開始判定成交。
- market order 使用下一個可用價格，再套用設定的 slippage。
- limit order 只有在後續可觀察成交或行情穿越限價時才可成交。
- 可選擇以後續成交量或顯示量限制成交數量。
- 無法確認時不成交，或由使用者選擇更寬鬆的模型。

每次結果必須記錄使用的 fill model、slippage、fee、tax 與商品乘數來源。

### SIM-02 帳務

第一版帳務至少包含：

- 訂單與成交紀錄
- 現金與部位
- 手續費與交易稅
- 已實現及未實現損益
- 基本績效摘要

所有帳務變化必須能追溯至策略下單意圖及模擬成交。

---

## 10. 使用性與非功能需求

### OPS-01 操作

常見工作應能由一份設定與一個頂層命令完成：

```text
plan -> sync -> verify -> replay/backtest -> inspect
```

執行前應顯示需要下載、可直接使用及資料不完整的商品／交易日。

錯誤訊息必須指出 market、symbol、trading date、format 與建議處理方式。

### OPS-02 執行結果

每次回測必須輸出：

- 資料 checksum
- 事件 schema 與排序規則版本
- 策略與參數
- fill model 與費用設定
- 事件數、警告與略過資料
- 交易、部位、損益與執行時間
- 可用的事件及最終狀態 checksum

### NFR-01 可重現

相同資料、版本與設定必須得到相同的事件順序、策略輸出、成交及損益。

並行或效能優化不得改變結果。

### NFR-02 效能

系統應優先減少重複下載、JSON 解析與無關商品 I/O。

首版效能門檻應使用實際的 2330 與 TAIFEX futures 資料建立 benchmark 後決定，不在高階規格中任意指定。

### NFR-03 安全與版本

API key 不得寫入資料檔、log、執行結果或版本控制。

來源資料 schema、標準事件、排序規則與 fill model 必須有版本；不相容時應重建快取或拒絕執行。

---

## 11. V-model 驗證

| 需求面 | 驗證方式 | 驗收證據 |
| --- | --- | --- |
| Teralion 同步 | API 整合測試 | cursor 走完、中斷重跑、缺資料才下載 |
| 正規化 | fixture 單元測試 | TWSE quote、TAIFEX trade／book、未知 format |
| 事件排序 | 單元與系統測試 | 打亂輸入後仍依 `match_time` 產生固定 checksum |
| 市場狀態 | reducer 單元測試 | 快照取代、成交、累計量、flags 與非法資料 |
| 無前視 | 系統測試 | 策略及訂單不能使用下一事件 |
| 成交與帳務 | 模型單元測試 | market／limit、slippage、部分成交、費稅與損益 |
| 使用性與效能 | 驗收與 benchmark | 單一工作流程、斷網回測、峰值記憶體及吞吐量 |

所有 normalizer 都必須使用保存的 Teralion tick 建立固定 fixture。golden result 只能因需求或來源格式變更而更新。

---

## 12. 里程碑

### M1 TWSE 回播核心

使用本地 2330 fixture 完成：

```text
Teralion TWSE regular tick
-> QuoteSnapshot / TradeBatch
-> Market State
-> match_time replay
-> Example Strategy
-> event/state checksum
```

M1 fixture 必須涵蓋 `STOCK_SNAPSHOT` 與 `STOCK_REALTIME`。final quote 以
`QuoteSnapshot` 表達；realtime intermediate print 以 `TradeBatch` 表達，並驗證
同 `match_time` 的 intermediate／final ordering。

### M2 真實資料與離線回測

使用 Teralion 的 2330 單日資料完成：

- cursor 下載與本地保存
- 資料驗證與回播快取
- 斷網重播
- 第二次執行不重新下載
- 基礎 market／limit fill 與損益

### M3 TAIFEX 與多商品

加入 TAIFEX futures：

- `TradeBatch`
- 五檔 `BookSnapshot`
- 跨日 trading date
- 2330 與 futures 多商品回播
- 缺少商品乘數時的明確設定與錯誤

### M4 擴充市場

依序加入 TPEx、權證與選擇權。每個市場先以實際 Teralion fixture 確認資料格式，再擴充事件 mapping 與測試。

每個里程碑必須拆成小型、可獨立驗證的 commit。

---

## 13. 已確定決策

- Rust 是核心實作語言。
- Teralion 是第一個歷史資料來源。
- `match_time` 是唯一的回播時間。
- 第一版以 explicit symbol universe 為主。
- 市場狀態以成交與完整五檔快照為主。
- 不重建逐筆委託簿，不模擬精確 queue position。
- 第一版不將 Teralion `close`／`stats` 正規化為 domain event 或 MarketState。
- 基礎成交從策略下單後的下一個可用事件開始判定。
- 資料同步與回測分離，回測預設離線。
- 回播只讀取策略需要的商品。

---

## 14. 後續詳細規格議題

- 各 Teralion market／format 的 event mapping
- 相同 `match_time` 的 tie-break 細節
- trading calendar 與 TAIFEX trading date
- 本地來源資料與回播快取格式
- status／limit flags 的可用語意
- instrument multiplier 與 reference data 來源
- market／limit fill model 的精確規則
- fee、tax、slippage 與部分成交設定
- benchmark dataset 與效能門檻
- CLI 設定、run manifest 與錯誤分類

---

## 15. 參考資料

資料 API：

- [Teralion Feed API](https://docs.teraliontech.com/feed/)
- [Teralion Feed Archive API](https://docs.teraliontech.com/feed-archive/)

台灣市場格式：

- [TWSE 集中市場即時交易資訊傳輸規格書（B.12.13）](https://dsp.twse.com.tw/public/static/downloads/computerPlanningOperationsDepartment/TWSE%E9%9B%86%E4%B8%AD%E5%B8%82%E5%A0%B4%E5%8D%B3%E6%99%82%E4%BA%A4%E6%98%93%E8%B3%87%E8%A8%8A%E5%82%B3%E8%BC%B8%E8%A6%8F%E6%A0%BC%E6%9B%B8%28B.12.13%29%28202612%29_20260515151841.pdf)
- [TPEx 上櫃股票 IP 行情網路規格書（V.12.18）](https://dsp.tpex.org.tw/storage/regular_system/%E4%B8%8A%E6%AB%83%E8%82%A1%E7%A5%A8IP%E8%A1%8C%E6%83%85%E7%B6%B2%E8%B7%AF%E8%A6%8F%E6%A0%BC%E6%9B%B8%28V.12.18_TCPIP%29.pdf)
- [TAIFEX 逐筆行情資訊傳輸作業手冊（V1.11.0）](https://www.taifex.com.tw/file/taifex/CHINESE/11/TechDocs/67/%E9%80%90%E7%AD%86%E8%A1%8C%E6%83%85%E8%B3%87%E8%A8%8A%E5%82%B3%E8%BC%B8%E4%BD%9C%E6%A5%AD%E6%89%8B%E5%86%8A%28V1.11.0%29.pdf)

回測設計參考：

- [NautilusTrader Backtesting](https://nautilustrader.io/docs/latest/concepts/backtesting/)
- [QuantConnect LEAN Algorithm Engine](https://www.quantconnect.com/docs/v2/writing-algorithms/key-concepts/algorithm-engine)
- [QuantConnect LEAN Fill Models](https://www.quantconnect.com/docs/v2/writing-algorithms/reality-modeling/trade-fills/key-concepts)

設計判斷優先順序：

1. 本文件確認的產品決策
2. Teralion 實際可取得的欄位與精度
3. 交易所文件對來源格式的解釋
4. 其他回測系統的設計經驗
