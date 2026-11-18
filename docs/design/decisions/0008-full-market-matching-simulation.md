# ADR 0008：全市場撮合模擬架構

## 狀態

Accepted（2026-11-18）。部分取代 [ADR-0007](0007-marketdata-native.md)：第 2 點（不建立 cache）、
第 8 點（範圍）、第 9 點（crate 清單）；以及 [ADR-0006](0006-external-strategies-and-cash-charges.md)
中依賴舊 `osmium-cli` 的部分。

## 背景

ADR-0007 移除 Teralion 後，系統仍保留為「多資料供應商」設計的中性事件層、兩套撮合器、TAIFEX
帳務與 session 表。實際需求收斂為：

- 只用 marketdata 的 TWSE／TPEx book 與 trade，涵蓋全市場所有商品（F6、F17、F23）。
- 回測的目的是精確：撮合與帳務都必須能逐筆追溯，並以明確規則與數值定義。
- 期貨與選擇權目前沒有歷史資料，先不支援。
- 速度與記憶體要做到極致，但在架構定案之後才做。

## 決策

### 1. 範圍

支援 TWSE、TPEx 在 marketdata `trades`／`books` 中的所有商品：一般 board（F6、F17）與盤中零股
board（F23）。不支援 TAIFEX、盤後定價、盤後零股、鉅額交易。TAIFEX 相關型別、帳務與 session
一律刪除，日後有資料時重新設計。

### 2. 資料依賴

- 依賴 marketdata 大版本路徑 `/srv/md/v1` 與 footer `md.schema_version == "1"`。開檔時逐欄比對
  欄名與 Arrow 型別，不符即拒絕。marketdata 開 `v2` 時，osmium 明確升版。
- 每次讀取以 manifest row 為資料 identity，讀取前後比對。
- **允許衍生快取**：為了效能，可以把 marketdata 轉成依 `seq` 排序、適合回放的格式。快取以
  manifest identity 與轉換版本為 key，可刪、可重建，永遠不是事實來源；identity 不符時重建。

### 3. 事件模型

移除中性事件層（`DomainEvent`、`QuoteSnapshot`、`Observation` 三態、canonical encoding、
fingerprint）。事件就是 marketdata 的訊息：

| 訊息 | 來源 |
| --- | --- |
| 揭示（五檔，可能帶成交） | `books`（`has_trade` 時 join 同 `seq` 的 `trades`） |
| 中間成交 | 只在 `trades` |
| 試撮 | `books.is_indicative` |
| 暫緩撮合 | `books.delay_state ≠ 0` |

事件型別以 marketdata 欄位語義命名，數值已轉成精確整數。來源沒有的資訊保留為 `Option`，不推測。

### 4. 商品與 board

- 商品 identity 為 `(venue, symbol)`。每個商品有兩本 book：一般 board、零股 board。
- 部位與成交數量一律以「股」為單位；一般 board 的張數乘以 `instruments.shares_per_lot`
  換算，缺值即拒絕。
- 價格為 1e-4 元的整數 tick；金額與費稅以整數運算並定義捨入規則。

### 5. 回放順序

同一 feed 依 `seq`；跨 feed 依 `observed_time`（feed 內至今最大 `exch_ts`），再依固定 feed rank 與
`seq`。這是決定性的全序。策略看到的資料時間是 `observed_time` 加上設定的行情延遲。

### 6. 撮合模擬

單一撮合引擎，涵蓋：

- **連續交易**：價格優先、時間優先。委託進場時，排在前面的量 = 當時該價位揭示的量。之後該價位
  的成交扣減前面的量；**刪單一律假設發生在我們後面**，因此只有在揭示量小於前面的量時，才把前面的
  量下修為揭示量。主動成交依揭示的對手價量逐檔吃，已被模擬吃掉的量在該價位被新揭示更新前不可
  重複使用。
- **集合競價**（開盤、收盤、零股、分盤、暫緩撮合後）：委託加入競價，依交易所成交價決定原則與
  實際撮合結果判定是否成交與成交量；試撮資訊依交易所揭示規則提供給策略。
- **延遲**：行情延遲與委託延遲皆為明確設定；委託只會被其到達交易所之後的訊息撮合。
- **不影響市場**：模擬委託不改變後續真實行情；此假設與其限制在文件中明確說明。

交易規則的細節（成交價決定原則、同價位分配、零股撮合頻率、瞬間價格穩定措施、升降單位、漲跌停、
委託類型與數量限制）以 `docs/reference/` 中的規格為準，規格必須附交易所來源與生效日期。

### 7. 帳務

全部模擬，並以精確整數運算：

- 現股買賣、現股當沖。
- 融資、融券（成數、保證金、利息、融券手續費、擔保品、維持率、追繳、強制回補、資券相抵）。
- T+2 交割與現金可用性。
- 除權、除息、減資、分割等公司行動（來源 marketdata `corp_actions`）。
- 手續費、證券交易稅依商品類型與生效日期查表，捨入規則明確定義。

### 8. 策略

策略以 Rust trait 實作並編譯進 binary，可訂閱任意商品集合直到全市場。策略只讀已揭示的狀態，
不能取得未來資料。

### 9. Crate 分層

```text
osmium-types     價格、數量、時間、商品、board 等基本型別
osmium-md        marketdata 讀取、schema／manifest 驗證、依 seq 的訊息流、衍生快取
osmium-book      每個商品、每個 board 的五檔與交易階段狀態
osmium-replay    多 feed 合併、replay clock、決定性排序
osmium-strategy  策略 trait、context、下單與計時器
osmium-matching  委託生命週期、連續交易與集合競價撮合
osmium-ledger    現金、部位、信用交易、交割、公司行動、費稅、損益
osmium-runner    串接各層、run 設定、輸出
osmium-cli       指令列入口（binary `osmium`）
osmium-examples  範例策略
```

依賴方向由上而下；`osmium-md` 只被 `osmium-replay` 與 `osmium-runner` 使用。

### 10. 文件

依讀者分層，每個事實只寫在一處：

| 目錄 | 讀者 | 內容 |
| --- | --- | --- |
| `README.md`、`docs/guide/` | 使用者 | 使用手冊：安裝、寫策略、執行、讀結果、疑難排解 |
| `docs/reference/` | 使用者、agent | 精確規格：marketdata、回放、撮合、帳務、設定、輸出、版本 |
| `docs/design/` | 開發者 | 架構、效能、ADR |
| `docs/development/` | 開發者、agent | 測試與驗證流程 |
| `AGENTS.md`、`.agents/skills/` | agent | `osmium-engineering`（改程式）、`osmium-backtest`（使用系統）；只放流程，規則連到 reference |

### 11. 遷移

新 crate 與舊 crate 並存，逐層取代；每一步都通過 workspace gate。舊 crate 與 `docs/design/legacy/`
在被取代時一併刪除。所有版本號在新 crate 重新起算。

## 後果

- 事件與狀態直接對應 marketdata，少一層轉換，正確性更容易對照來源驗證。
- 撮合引入排隊估計，結果比舊的「只看後續事件」模型更接近實際，但依賴「刪單在後」的保守假設。
- 帳務範圍擴大到信用交易、交割與公司行動，需要完整的規則表與生效日期管理。
- 衍生快取帶來額外磁碟用量，但全市場回放不必每次重排上千萬列。
