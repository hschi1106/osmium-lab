# marketdata 介面

`osmium-md` 是 osmium 唯一的資料來源 adapter：唯讀地讀取 marketdata（預設 `/srv/md/v1`）的
parquet，驗證後解碼成訊息。訊息與商品的型別定義在 `osmium-types`。設計決策見
[ADR-0008](../design/decisions/0008-full-market-matching-simulation.md) 與
[ADR-0007](../design/decisions/0007-marketdata-native.md)。

狀態：本頁描述已實作的行為。跨 feed 的合併與時間線見[回放規格](replay.md)。

## 1. 依賴的 schema 版本

osmium 依賴 marketdata schema 版本 `1`（`osmium_md::SCHEMA_VERSION`）：

- manifest 列的 `schema_version` 必須是 `"1"`。
- 檔案 footer 的 `md.schema_version`、`md.git_commit` 必須與 manifest 列相同。
- osmium 讀取的每一個欄位，在讀任何一列之前先檢查名稱與 Arrow 型別；缺欄或型別不同即
  `MdError::Schema`。沒有讀取的欄位不檢查，因為版本 1 之內表可以加欄。

| 表 | 讀取的欄位與型別 |
| --- | --- |
| `books` | `symbol` string、`exch_ts` timestamp[µs, UTC]、`seq` int64、`phase` string、`has_trade` bool、`n_bid`／`n_ask` int8、`bid_px_1..5`／`ask_px_1..5` float64、`bid_qty_1..5`／`ask_qty_1..5` int64、`bid_mkt_qty`／`ask_mkt_qty` int64、`is_indicative` bool、`indicative_px` float64、`indicative_qty` int64、`delay_open`／`delay_close` bool、`delay_state` int8 |
| `trades` | `symbol` string、`exch_ts` timestamp[µs, UTC]、`seq` int64、`sub_seq` int16、`price` float64、`qty` int64、`cum_qty` int64、`phase` string |
| `instruments` | `symbol` string、`instrument_type` string、`shares_per_lot` int32、`ref_price`／`limit_up`／`limit_down` float64、`match_interval_s` int32、`currency` string、`derived` bool |
| `manifest` | `job`、`table`、`part`、`path`、`schema_version`、`git_commit`、`code_hash`、`inputs_hash` string；`rows`、`seq_gaps`、`seq_dup` int64；`partial` bool |

marketdata 改成不相容的版本（`/srv/md/v2`）時，osmium 必須跟著修改並更新本頁。

## 2. Feed

| `Feed` | marketdata part | 市場 | board | marketdata 的數量單位 | feed rank |
| --- | --- | --- | --- | --- | --- |
| `TwseF6` | `TWSE_F6` | TWSE | 一般（`Regular`） | 張 | 0 |
| `TpexF6` | `TPEX_F6` | TPEx | 一般 | 張 | 1 |
| `TwseF17` | `TWSE_F17` | TWSE | 一般（權證） | 張 | 2 |
| `TpexF17` | `TPEX_F17` | TPEx | 一般（權證） | 張 | 3 |
| `TwseF23` | `TWSE_F23` | TWSE | 盤中零股（`OddLot`） | 股 | 4 |
| `TpexF23` | `TPEX_F23` | TPEx | 盤中零股 | 股 | 5 |

TAIFEX、`sinopac_ticks`、`bars_1m`、`daily` 不讀取。

## 3. 讀取與一致性

```text
DayManifest::load(root, date)
DayInstruments::load(root, &manifest, venue)
FeedDay::load(root, &manifest, feed, &instruments, selection, missing_lot_size)
```

1. `DayManifest::load` 讀當日 `manifest/{YYYY}/{date}/*.parquet`。目錄不存在即 `MissingManifest`；
   沒有該表／part 的列即 `MissingManifestEntry`（當天沒有這份資料）。
2. 開檔時檢查 manifest 的 `path` 符合 layout、`rows` 等於檔案列數，以及第 1 節的版本條件；
   不符即 `Inconsistent` 或 `Schema`。
3. `selection` 是要讀的商品；`None` 讀整個 feed。檔案依 `(symbol, seq)` 排序，以 `symbol` 的
   row-group statistics 剪枝，只解碼需要的欄位。
4. 讀完後重新讀 manifest；讀過的列有任何改變即 `ManifestChanged`。

`FeedDay::identity()`（books 與 trades 的 manifest 列）與 `DayInstruments::manifest_entry()` 含
`code_hash`、`inputs_hash`、`git_commit`，供 run 的輸出記錄。

## 4. 訊息

一則訊息（`osmium_types::Message`）是某個 feed 的一個 `seq`。`FeedDay::messages()` 依 `seq` 遞增，
也就是交易所送出的順序，產生訊息。

| 欄位 | 定義 |
| --- | --- |
| `feed`、`seq`、`symbol` | 來源的 feed、交易所序號、商品代號 |
| `exch_ts` | 交易所時間，µs |
| `observed_time` | 整個 feed 中 `seq` 不大於本訊息的所有訊息的最大 `exch_ts`；包含沒有被選取的商品，所以與 `selection` 無關 |
| `phase` | `OpenAuction`、`Continuous`、`PeriodicAuction`、`CloseAuction`（marketdata 的 `phase`） |
| `body` | 下表四種之一 |

| `MessageBody` | 來源 | 內容 |
| --- | --- | --- |
| `Disclosure { book, trade: None }` | `books` 一列，`has_trade = false` | 五檔變動後的五檔 |
| `Disclosure { book, trade: Some }` | `books` 一列（`has_trade`）與同 `seq` 的 `trades` 一列 | 一筆成交與成交後的五檔 |
| `IntermediateTrade(trade)` | 只在 `trades` 的一列，或第 6 節的形狀 | 同一筆委託連續成交多個價位時，最後一筆以前的成交；不揭示五檔 |
| `Indicative { book, matched, delay_open, delay_close }` | `books` 一列，`is_indicative` | 試撮：五檔、試算成交價量（沒有時為 `None`）、延後開盤／收盤的預告（來源沒有時為 `None`） |
| `Delay(direction)` | `books` 一列，`delay_state` 為 1（`Up`，趨漲）或 -1（`Down`，趨跌） | 暫緩撮合；不揭示五檔 |

- `Book`：買賣各最多 5 檔（`BOOK_DEPTH`）限價價量，最佳在前，另有市價委託數量 `market_qty`。
  空的一邊是 0 檔；`Book::EMPTY` 表示兩邊都沒有委託，與「沒有揭示五檔」（`IntermediateTrade`、
  `Delay`）不同。
- `Trade`：`price`、`qty`、`cum_qty`（當日累計成交量；來源沒有時為 `None`）。

## 5. 檢查

下列任何一項不成立即 `MdError::InvalidRow`（含檔案、商品、`seq` 與原因），整個讀取失敗：

- 檔案依 `symbol` 遞增排序，同一商品內 `seq` 嚴格遞增；同一個 feed 內 `seq` 不重複。
- `symbol` 是 1–8 個 ASCII 英數字；`phase` 是四種之一；`sub_seq = 0`。
- `has_trade` 的 `books` 列必有同 `seq` 的 `trades` 列，反之亦然；兩列的商品、`phase`、`exch_ts`
  相同。
- 只在 `trades` 的列（中間成交）只出現在 `CONTINUOUS`。
- 暫緩撮合列（`delay_state ≠ 0`）：`delay_state` 是 ±1，沒有五檔（`n_bid`／`n_ask` 為 null），不是
  成交也不是試撮，沒有試算價量。
- 其他列一定揭示五檔（`n_bid`／`n_ask` 不是 null）。
- 五檔：檔數 0–5；檔數內的價量都存在（第 6 節的例外除外），檔數外都是 null；各檔價格由最佳
  往外嚴格單調；最佳買價低於最佳賣價。
- 試撮列不是成交；`indicative_px`／`indicative_qty` 同時存在或同時為 null。非試撮列沒有試算
  價量，`delay_open`／`delay_close` 不為 true。
- 價格、數量符合第 7 節。

`seq` 的缺口（封包遺失）不是錯誤；manifest 的 `seq_gaps` 記錄缺口數，由回放寫進輸出。

## 6. 舊來源的兩種形狀

`TWSE_F6` 在 2022-01-03 → 2024-03-29 由較舊的來源轉換（manifest `partial = true`），有兩處與之後
的日子不同。`osmium-md` 依結構辨識，所以輸出的訊息與其他日子同一個模型。

**市價委託在第 1 檔。** 交易所把市價委託送成「第 1 檔、沒有價格」。marketdata 的規則是把它移到
`*_mkt_qty`，但這段期間來源以 null 記錄這個價格，轉換沒有移動它，於是它留在第 1 檔並計入
`n_*`。規則：`n_* > 0`、`*_mkt_qty` 為 null、第 1 檔價格為 null 時，第 1 檔的數量是市價委託數量，
第 2 檔以後依序前移。其他位置的 null 價格仍是錯誤。

**中間成交也在 `books`。** 這段期間來源沒有「中間成交」的旗標，所以每筆成交都同時寫進 `trades`
與 `books`；中間成交的 `books` 列是 `has_trade` 且買賣檔數都是 0，與「成交後委託簿真的空了」的
形狀相同。規則：`CONTINUOUS` 中 `has_trade`、兩邊 0 檔、沒有市價委託的列，如果同一個商品的
下一則訊息有相同的 `exch_ts`，且是成交或暫緩撮合，它就是 `IntermediateTrade`；否則是空的
`Disclosure`。

依據：同一筆委託的連續成交有相同的 `exch_ts`，最後一筆才揭示五檔；連續成交在瞬間價格穩定措施
的界線停下時，以暫緩撮合列結束。委託簿空了之後，下一則訊息不可能是成交。實測（2022-01-03、
2022-06-15、2023-03-15、2024-03-29 的 `TWSE_F6`，2025-04-07、2026-10-08 的 `TWSE_F6`、`TPEX_F6`、
`TPEX_F17`）：

- 有旗標的日子，每一筆只在 `trades` 的中間成交，同商品的下一則訊息都是同 `exch_ts` 的成交或
  暫緩撮合；帶五檔的成交後面從來不是同 `exch_ts` 的成交。
- 沒有旗標的日子，每天約 10,000–15,000 列符合規則；不符合的（2022-06-15 有 1 列）下一則訊息
  時間較晚，是真的空了。

`seq` 有缺口時，缺口旁的這類列可能被歸錯類。

## 7. 數值

| 項目 | 規則 |
| --- | --- |
| 價格 | marketdata 以最接近的 float64 儲存十進位價格。轉成 `Price`（單位 1e-4 元的整數）：`units = round(x × 10⁴)`，必須 `units / 10⁴ == x`、`0 < x ≤ 400,000`，否則是錯誤 |
| 數量 | 正整數，轉成股數 `Shares`：一般 board 乘以 `instruments.shares_per_lot`，零股不變 |
| `cum_qty` | 正整數或 null，同樣轉成股數 |
| 時間 | `Timestamp` 是 Unix epoch 起算的 µs；`Date` 是交易日 |

一般 board 的商品在 `instruments` 沒有 `shares_per_lot`（值為 null，或沒有這個商品的列）時，依
`MissingLotSize`：

- `Reject`：讀取失敗（`MdError::MissingLotSize`）。
- `Assume(n)`：以每張 `n` 股換算，並列在 `FeedDay::assumed_lot_sizes()`。

## 8. `instruments`

`DayInstruments` 是一個市場一天的商品基本資料，依 `symbol` 排序。

| `Instrument` 欄位 | 來源 | 說明 |
| --- | --- | --- |
| `instrument_type` | `instrument_type` | `STOCK`、`ETF`、`ETN`、`TDR`、`REIT`、`WARRANT_C`、`WARRANT_P`、`OTHER`；其他值是錯誤 |
| `shares_per_lot` | `shares_per_lot` | null → `None` |
| `ref_price`、`limit_up`、`limit_down` | 同名 | null 或 0 → `None`（來源以 0 表示沒有） |
| `match_interval_s` | `match_interval_s` | 撮合循環秒數，0 是逐筆交易；null → `None` |
| `currency` | `currency` | `TWD` → `Twd`，其他 → `Foreign` |
| `derived` | `derived` | 這一列不是交易所播送的資料，而是由盤後資料推導 |

交易規則用到的商品分類與升降單位由 `osmium-matching` 決定，見[撮合規格](matching.md)。

## 9. 資料範圍與已知缺口

實測於 2026-11（資料到 2026-10-16）。

| Feed | 有資料的日子 |
| --- | --- |
| `TWSE_F6` | 2022-01-03 起，1,160 天 |
| `TPEX_F6` | 2024-04-01 起，619 天 |
| `TWSE_F17`、`TPEX_F17` | 2025-07-31 起，278／281 天 |
| `TWSE_F23`、`TPEX_F23` | 2025-08-22 起，272／268 天 |

`instruments` 只有 2024-04-01 → 2026-10-12 是交易所播送的資料。其他日子（2022-01-03 →
2024-03-29、2026-10-13 起）的列是推導的（`derived = true`）：

- `shares_per_lot`、`match_interval_s` 都是 null：一般 board 必須用 `MissingLotSize::Assume`。
  每張不是 1,000 股的商品（實測只有以外幣計價的 ETF `00636K`、`00657K`、`00668K`、`00687C`，
  每張 100 股）會被換算錯。
- 權證與 TPEx 可轉換公司債沒有列。
- `currency` 一律是 `TWD`，包含外幣計價的商品。
- 漲跌停價有許多是 null。

`TWSE_F6` 在 2022-01-03 → 2024-03-29：`delay_open`／`delay_close` 是 null，另有第 6 節的兩種形狀。

## 10. 目前的限制

- 一個 feed 一天解碼後整個留在記憶體，每則訊息約 140 bytes：`TWSE_F6` 一天約 1–5 GB，
  `TWSE_F17` 一天（約 7,400 萬則）約 10 GB。串流讀取與衍生快取排在效能階段。
- 指定 `selection` 時，為了計算 `observed_time`，會另外讀一次整個 feed 的 `seq` 與 `exch_ts`。

## 11. 檢視與比對

```sh
cargo run --release -p osmium-md --example dump -- /srv/md/v1 2026-10-08 TWSE_F6 2330
```

每則訊息輸出一行文字。驗證方式見[驗證](../development/validation.md)。
