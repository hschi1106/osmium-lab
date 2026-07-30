# 策略需求

## 1. 文件目的

本文件將[產品需求](../product-requirements.md)中的 `STRAT-01` 細化為可設計、
實作與驗證的系統需求。

本文件定義第一版策略必須具備的能力、可觀察資料及不可跨越的邊界，不固定：

- Rust trait、associated type 或 function signature
- ownership、borrowing、lifetime 或 async 模型
- strategy crate 的 layout
- indicator storage representation
- order intent 的具體 Rust type
- 動態載入或跨程序 protocol

上述選擇由 [strategy API 設計](../design/strategy-api.md)記錄。任何 API 設計都必須
保留唯讀市場狀態、無前視及可重現性。

## 2. 範圍與責任

策略層位於 replay／market state 與 execution simulation 之間：

```text
domain event + updated read-only MarketState
-> strategy
-> indicator output + order intent
-> simulation
-> simulated order/fill feedback
-> strategy
```

策略負責：

- 宣告所需商品與參數
- 依目前及過去事件計算指標或決策
- 產生 order intent
- 接收模擬訂單及成交回報

策略不負責：

- 下載、驗證或開啟市場資料
- 排序事件或推進 replay clock
- 修改 MarketState
- 決定 fill、fee、tax 或 accounting 規則
- 讀取尚未發生的事件或最終結果

## 3. 通用定義

### 3.1 Strategy definition

`strategy definition` 是可識別策略實作、版本及其參數 schema 的靜態描述。

### 3.2 Strategy instance

`strategy instance` 是一次 run 使用的策略實作及已驗證參數。相同 definition、
binary identity 與參數必須可在結果中識別。

### 3.3 Universe declaration

`universe declaration` 是策略執行前明確宣告的 market／symbol 集合。第一版只支援
explicit symbol universe，不在回播期間依 metadata 動態新增商品。

### 3.4 Order intent

`order intent` 是策略要求模擬層建立、修改或取消模擬訂單的意圖。它不是交易所
訂單，也不代表必定成交。

### 3.5 Strategy feedback

`strategy feedback` 是模擬層對 order intent 產生的訂單狀態、拒絕、取消或 fill
結果。feedback 不得追溯改變已處理的市場事件。

## 4. STRAT-01：策略能力與邊界

### STRAT-01.1 策略識別與參數

每個 strategy definition 必須能提供或由 run configuration 明確指定：

- 穩定的 strategy name／identifier
- strategy version 或可識別實作 binary 的 identity
- 參數 schema 或等價的驗證規則
- 實際使用的參數值

執行前必須完成參數驗證。缺少必要參數、未知且不允許的參數、不合法型別或超出
明確限制的值必須在讀取市場事件前失敗。

default value 必須明確、可檢查且納入執行結果；不得因本機環境或 iteration order
選出不同 default。

### STRAT-01.2 Universe 宣告

策略必須在 execution plan 完成前宣告所需的 market／symbol 清單。

universe declaration 必須：

- 使用足以避免跨 market symbol 衝突的識別。
- 在資料 stream 開啟前可取得。
- 納入 execution plan 及執行結果。
- 在相同策略與參數下產生相同集合。
- 只允許策略讀取宣告 universe 的市場狀態。

第一版不得在 replay callback 中動態擴張 universe。若策略輸出引用 universe 外商品，
系統必須明確拒絕，不得自動開啟新資料 stream。

### STRAT-01.3 執行階段

第一版策略 API 必須能表達下列邏輯階段，但不要求使用這些 method 名稱：

1. 建立 definition 並驗證參數。
2. 宣告 universe，供平台建立 execution plan。
3. 在第一個事件前初始化一次 strategy instance。
4. 依 deterministic replay order 處理 event callback。
5. 接收模擬訂單及 fill feedback。
6. 在最後事件後完成一次 finalize，產生可用的策略摘要。

平台必須定義各階段的順序及可用 context。初始化或 finalize 不得取得尚未允許的
市場事件。

### STRAT-01.4 Event callback

策略必須能接收[回播需求](replay.md)定義的標準事件。每次 event callback 至少能
識別：

- 目前 event
- 目前 replay time
- 該 event 所屬商品
- 該 event 完成原子更新後的唯讀 MarketState
- 由目前 phase、event annotations 與版本化 market rules 產生的唯讀
  `TradingContext`

callback sequence 必須與 deterministic replay order 相同。相同 `match_time` 的
事件仍逐一 callback；策略只能看到 tie-break 中目前及較早的事件。

平台可以提供 universe 內其他商品「截至目前已處理事件」的唯讀狀態，但不得提供
任何商品的下一事件、未來狀態或日後才知道的統計。

`TradingContext` 必須分開表達新 order entry 與 matching availability。策略可以用
它避免明知無效的 intent，但不得自行解碼 raw flags 取代平台規則，也不得把 context
視為 fill 保證。具體契約見
[ADR-0004](../architecture/decisions/0004-trading-context-and-eligibility.md)。

### STRAT-01.5 唯讀市場邊界

策略不得修改：

- domain event
- MarketState
- state version
- replay clock
- 已處理的歷史事件
- source data 或 replay cache

API 必須以型別、ownership 或等價的強制邊界防止一般策略直接修改上述資料，而非
只依賴文件約定。

策略自己的內部狀態可以修改，但只能由已允許的設定、event、MarketState 及
feedback 推進。

### STRAT-01.6 無前視

策略在任一 callback 中不得取得：

- 下一 market event 或其 `match_time`
- 尚未處理事件形成的 book、trade、stat 或 flags
- 尚未完成 bar 的 final OHLC／volume
- 當時不可知的 session／day final statistics
- 最終 position、P&L 或 run summary

indicator 必須只使用 callback 當下可取得的目前及過去資料。平台提供的 helper、
iterator、history view 或 cache API 也必須遵守相同限制。

預先載入資料、並行 normalization 或效能最佳化不得擴大策略可見範圍。

### STRAT-01.7 Indicator output

策略必須能產生自訂 indicator 或 observation。每筆輸出至少必須可追溯至：

- strategy identity
- output name
- 目前 replay time
- 觸發輸出的 event 或 state version
- output value

indicator representation 必須避免因 locale、debug formatting 或非 deterministic
iteration order 改變結果。若輸出使用浮點數，其可重現與 serialization policy
必須在 design 中明確定義。

indicator 不得被回寫為市場事實，也不得修改後續 MarketState reducer 的結果。

### STRAT-01.8 Order intent

策略必須能產生模擬層可驗證的 order intent。intent 至少必須表達足以識別：

- 所屬 strategy instance
- 目標 market／symbol
- 動作或訂單目的
- side
- quantity
- order type 需要的價格資料
- 產生 intent 的 replay time 與 event identity

具體欄位與 order type 集合由
[simulation requirements](simulation.md)及
[execution simulation 設計](../design/execution-sim.md)決定。

不合法 quantity、price、unsupported order type 或 universe 外商品必須被明確
拒絕並回報策略，不得轉成另一種意圖或靜默捨棄。

即使 phase baseline 允許新 intent，market-specific `TradingContext` 仍可依
pre-open trial、pre-close trial、closing result、indicative matching 或 unknown
condition 將該 intent restricted、blocked 或 rejected。

order intent 不得修改產生它的 market event。其最早 fill eligibility 由
`SIM-01` 決定，且不得使用產生 intent 的同一事件回填成交。

### STRAT-01.9 模擬結果 feedback

策略必須能接收與其 intent 對應的：

- order accepted／rejected
- order state change
- fill 或 partial fill
- cancellation；若第一版 order lifecycle 支援
- 相關 fee、tax 或 accounting information；若 feedback contract 提供

每筆 feedback 必須：

- 具有穩定 identity。
- 可追溯至原始 order intent。
- 依平台定義的 deterministic processing order 提供。
- 不早於產生該結果所需的 market event。
- 不包含未來 fill 或最終帳務資料。

策略不能修改已產生的 feedback。若策略根據 feedback 產生新 intent，新 intent
同樣受到下一個可用事件才可判定 fill 的限制。

### STRAT-01.10 Deterministic strategy

符合平台可重現性保證的策略必須：

- 對相同 callback sequence、參數及 feedback 產生相同輸出。
- 不依賴未記錄的 wall clock、process ID、thread scheduling 或 filesystem order。
- 不使用未固定且未記錄 seed 的 randomness。
- 不在 backtest callback 中自動存取網路。
- 對 map／set 等無序資料結構採固定輸出順序。

若平台允許 seeded randomness，seed 必須是明確設定並記錄於 run result。第一版
可以不提供 randomness helper。

平台不得把已知使用非 deterministic capability 的 run 標示為符合 `NFR-01`。

### STRAT-01.11 錯誤與 panic

參數錯誤必須在 replay 開始前回報。策略在初始化、callback、feedback 或 finalize
階段回傳錯誤或 panic 時：

- run 必須停止，除非未來另有明確且安全的 strategy isolation policy。
- result 必須標示失敗，不得輸出看似成功的完整績效。
- 已產生的診斷資料可以保留，但必須標示 partial。
- 錯誤至少包含 strategy identity、階段、replay time 及 event identity；若可用。
- 不得因錯誤修改來源資料或已驗證 replay cache。

panic handling 的技術方式由 design 決定；需求是失敗可見且不污染成功結果。

### STRAT-01.12 第一版載入模型

第一版策略使用 Rust trait 與編譯期連結。

第一版不要求：

- runtime plugin loading
- dynamic library ABI
- Python／WASM／腳本策略
- 遠端或跨程序策略
- hot reload

未來新增其他載入模型時，仍必須維持本文件的 universe、唯讀、無前視、feedback
及 determinism 邊界。

## 5. 驗收條件

`STRAT-01` 至少必須由下列證據驗證：

- 合法與不合法參數在 replay 前驗證的測試。
- explicit universe 建立 execution plan 的 integration test。
- universe 外資料不開啟、universe 外 intent 被拒絕的測試。
- callback 收到目前 event 及更新後 state version 的測試。
- callback 收到與目前 event／state version 對應的唯讀 TradingContext 測試。
- phase baseline allowed、但 market condition restricted／closed 時 intent 被明確
  reject 的測試。
- type／API boundary 不允許修改 event、MarketState 或 replay clock 的 compile-fail
  或等價測試。
- strategy 看不到下一事件及未完成統計的測試。
- indicator 與 order intent 可追溯至 event 的測試。
- order／fill feedback 可追溯且不提前送達的測試。
- 相同輸入多次執行產生相同 indicator、intent 與 feedback sequence 的測試。
- strategy 不自行解碼 raw flags 改寫平台 eligibility 的 API boundary 測試。
- strategy error／panic 不產生成功結果的測試。

[M1：TWSE 回播核心](../increments/M1-twse-replay.md)以不送單的 ExampleStrategy
驗證：

- explicit TWSE 2330 universe
- event callback
- 更新後唯讀 MarketState
- deterministic indicator／observation
- 無前視

M2 再補 order intent、order state 與 fill feedback；M3 補多商品 state view。

## 6. 跨需求不變條件

任何策略 API 與實作都必須維持：

1. 策略在資料 stream 開啟前宣告 explicit universe。
2. 策略只能讀取 event、MarketState 及 replay clock。
3. callback 先看到目前事件更新後狀態，再產生輸出。
4. 策略不能取得下一事件或日後才知道的資料。
5. indicator、intent 與 feedback 皆可追溯。
6. order intent 不代表成交，且不能在產生它的事件回填成交。
7. 相同輸入、版本與設定產生相同策略輸出。
8. strategy failure 不得偽裝成成功 backtest。
9. 新載入模型不得放寬唯讀、無前視或 offline 邊界。

## 7. 與其他需求的關係

| 關係 | 說明 |
| --- | --- |
| `REPLAY-04` | 定義 event、clock、state update 與 strategy callback 的順序 |
| `REPLAY-05` | 使用 strategy universe 決定要開啟的 streams |
| `SIM-01` | 驗證 intent 並在後續可用事件判定 fill |
| `SIM-02` | 將 order／fill 轉成可追溯 accounting feedback |
| `OPS-02` | 保存 strategy identity、參數、輸出及失敗狀態 |
| `NFR-01` | 要求相同輸入產生相同策略輸出 |
| `NFR-03` | 保護 secret 並識別影響結果的版本 |

正式 requirement、design、implementation 與 test mapping 由
[traceability matrix](../traceability.yaml)維護。

## 8. 待下游文件決定的事項

| 議題 | 文件 |
| --- | --- |
| Trait、context、ownership 與 callback signatures | [strategy API 設計](../design/strategy-api.md) |
| Domain event 與 MarketState types | [market types](../design/market-types.md)及[market state](../design/market-state.md)設計 |
| Callback orchestration 與多商品 state view | [replay engine 設計](../design/replay-engine.md) |
| Order intent、feedback 與 fill processing | [execution simulation 設計](../design/execution-sim.md) |
| Strategy config 與 run result presentation | [CLI 操作](../operations/cli.md) |

下游文件不得以 API 便利、效能或擴充性為由讓策略修改市場狀態、讀取未來事件或
在未記錄的情況下引入非 deterministic input。
