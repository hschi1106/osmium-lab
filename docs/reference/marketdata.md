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
- 參考表（第 9 節）沒有 manifest 列，只檢查檔案 footer：`md.schema_version` 必須是 `"1"`，
  `md.git_commit` 必須存在。
- osmium 讀取的每一個欄位，在讀任何一列之前先檢查名稱與 Arrow 型別；缺欄或型別不同即
  `MdError::Schema`。沒有讀取的欄位不檢查，因為版本 1 之內表可以加欄。

| 表 | 讀取的欄位與型別 |
| --- | --- |
| `books` | `symbol` string、`exch_ts` timestamp[µs, UTC]、`seq` int64、`phase` string、`has_trade` bool、`n_bid`／`n_ask` int8、`bid_px_1..5`／`ask_px_1..5` float64、`bid_qty_1..5`／`ask_qty_1..5` int64、`bid_mkt_qty`／`ask_mkt_qty` int64、`is_indicative` bool、`indicative_px` float64、`indicative_qty` int64、`delay_open`／`delay_close` bool、`delay_state` int8 |
| `trades` | `symbol` string、`exch_ts` timestamp[µs, UTC]、`seq` int64、`sub_seq` int16、`price` float64、`qty` int64、`cum_qty` int64、`phase` string |
| `instruments` | `symbol` string、`instrument_type` string、`shares_per_lot` int32、`ref_price`／`limit_up`／`limit_down` float64、`match_interval_s` int32、`currency` string、`derived` bool、`daytrade` string、`f1_non10_par` bool |
| `qc/calendar` | `trade_date` date32、`twse_open`／`tpex_open` bool |
| `corp_actions` | `symbol` string、`ex_date` date32、`action` string、`before_price`／`ref_price`／`after_price`／`cash_dividend`／`value` float64、`pay_date` date32、`stock_per_1000`／`shares_old`／`shares_new`／`refund_per_share` decimal(18,8) |
| `events` | `symbol` string、`event` string、`start_date`／`end_date` date32 |
| `daily` | `trade_date` date32、`symbol` string、`ref_price`／`limit_down`／`close`／`close_bid_px`／`close_ask_px` float64 |
| `daily_ext` | `trade_date` date32、`symbol` string、`margin_limit`／`short_limit` int64 |
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

TAIFEX、`sinopac_ticks`、`bars_1m`、`fundamentals` 不讀取。`daily` 等參考表只供帳務使用（第 9 節），不產生訊息。

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
| `IntermediateTrade(trade)` | 只在 `trades` 的一列 | 同一筆委託連續成交多個價位時，最後一筆以前的成交；不揭示五檔 |
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
- 五檔：檔數 0–5；檔數內的價量都存在，檔數外都是 null；各檔價格由最佳
  往外嚴格單調；最佳買價低於最佳賣價。
- 試撮列不是成交；`indicative_px`／`indicative_qty` 同時存在或同時為 null。非試撮列沒有試算
  價量，`delay_open`／`delay_close` 不為 true。
- 價格、數量符合第 7 節。

`seq` 的缺口（封包遺失）不是錯誤；manifest 的 `seq_gaps` 記錄缺口數，由回放寫進輸出。

## 6. 舊來源

`TWSE_F6` 在 2022-01-03 → 2024-03-29 由較舊的來源轉換（manifest `partial = true`）。marketdata 在
來源端修正了這段期間的兩處形狀（tick manifest 的 `code_hash` `a5fb45c78b36a1f393a3762722ccd885fa2ae9cc`
起）：市價委託在 `*_mkt_qty`，中間成交只在 `trades`。`osmium-md` 不再辨識舊的形狀；在較早轉換的
資料上，揭示五檔內沒有價格的檔位是 `InvalidRow`。

這段期間仍然與之後不同的地方：

- 沒有 `delay_open`／`delay_close`（null）。
- 來源沒有中間成交的旗標，marketdata 依同一筆委託的連續成交有相同的 `exch_ts` 推定。`seq` 有
  缺口時，缺口旁的成交可能被歸錯類（2022-01-03 → 2024-03-29 的 41 個有缺口的日子，與 osmium
  先前的規則不同的有 31 列）。
- 每一天都是 `partial`，回放在輸出記錄 `data_quality`（[輸出規格](artifacts.md)）。

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
| `day_trade` | `daytrade` | `BOTH` → `Both`（可先買後賣、先賣後買），`BUY_FIRST` → `BuyFirst`，`NONE` → `Neither`（不可當沖），null（來源沒有）→ `None`；其他值是錯誤 |
| `non_10_par` | `f1_non10_par` | 面額不是 10 元；null → `None` |

交易規則用到的商品分類與升降單位由 `osmium-matching` 決定，見[撮合規格](matching.md)。

## 9. 參考表

帳務（[帳務規格](accounting.md)）需要的盤後與公司行動資料。這些表不分日目錄、沒有 manifest 列，
`ReferenceFile`（路徑、列數、`md.git_commit`）供 run 的輸出記錄。價格與每股金額在來源是 float64，
讀取時轉成整數，**不是精確往返即 `Inconsistent`**，不會捨入。

| 讀取 | 檔案 | 內容與規則 |
| --- | --- | --- |
| `TradingCalendar::load(root)` | `qc/calendar.parquet` | 每個日曆日一列，依日期遞增（否則錯誤）。`trading_days(venue)` 是該市場開市的日子 |
| `CorporateActions::load(root, from)` | `corp_actions.parquet` | `ex_date ≥ from` 的列；更早的列不轉換，所以舊資料無法表示時不影響 run。`action` 必須是 `dividend`、`rights`、`split`、`par_change`、`reverse_split`、`reduction_loss`、`reduction_cash`。`before_price`、`ref_price` 必填，`after_price` 可為 null（1e-4 元）；`cash_dividend`、`value` 是 1e-10 元（`PerShare`），範圍 0 至 100,000；`value` 不是正數時讀成 `None`（認購權的理論價值可以是負的）。decimal 欄位以 1e-8 為單位讀成整數（`Decimal8`）：`stock_per_1000`、`shares_old`、`shares_new` 必須是正數，`refund_per_share` 不得為負（轉成 `PerShare`），都不得超過 100,000；`shares_old`、`shares_new` 必須同時有或同時沒有 |
| `MarketEvents::load(root, from)` | `events.parquet` | `event` 必須是 `disposition`、`halt`、`short_suspended`、`daytrade_suspended`。保留 `end_date ≥ from` 的列，以及 `end_date` 為 null、`start_date` 在 `from` 之前 366 天以內的列。`end_date < start_date` 是錯誤 |
| `DailyQuotes::load(root, date)` | `daily/{YYYY}.parquet` | 一個交易日的官方盤後價格。以 `trade_date` 的 row-group statistics 剪枝（一個交易日一個 row group）。價格 null 或 0 → `None`。同一天的 `symbol` 必須嚴格遞增 |
| `CreditQuotas::load(root, date)` | `daily_ext/{YYYY}.parquet` | 一個交易日的融資、融券限額（股）；負數是錯誤 |

實測（2026-11，`/srv/md/v1`）：

- `qc/calendar`：1,250 列，2022-01-03 → 2026-10-16，1,160 個交易日；每一列 `twse_open` 都等於
  `tpex_open`。日曆沒有未來的日子，最後幾個交易日的交割日必須由使用者補上
  （[帳務規格](accounting.md) 7.1）。
- `corp_actions`（2022-01-03 起 11,927 列）全部可以精確轉換。`dividend` 11,254 列、`rights` 455、
  `reduction_loss` 107、`reduction_cash` 78、`par_change` 21、`reverse_split` 6、`split` 6。
- 每一列 `dividend` 都有現金股利或 `stock_per_1000`（802 列）；10,906 列有 `pay_date`。
- `after_price` 與 `ref_price` 不同的只有同一天有現金增資的 55 列 `dividend` 與 454 列 `rights`。
- 每一列減資、分割、反分割、面額變更都有 `shares_old`／`shares_new`，減資都有 `refund_per_share`。
  `par_change` 有 1 列不是整數倍（6548，1 股換 2.5 股）。
- `events`：自 2022-01-03 起生效的列 45,290；`short_suspended` 有 1,211 列 `end_date` 為 null。
- `daily`、`daily_ext` 全部可以精確轉換。

## 10. 資料範圍與已知缺口

實測於 2026-11（資料到 2026-10-16）。

| Feed | 有資料的日子 |
| --- | --- |
| `TWSE_F6` | 2022-01-03 起，1,160 天 |
| `TPEX_F6` | 2024-04-01 起，619 天 |
| `TWSE_F17`、`TPEX_F17` | 2025-07-31 起，278／281 天 |
| `TWSE_F23`、`TPEX_F23` | 2025-08-22 起，272／268 天 |

`instruments` 只有 2024-04-01 → 2026-10-12 是交易所播送的資料。其他日子（2022-01-03 →
2024-03-29、2026-10-13 起）的列是推導的（`derived = true`）：

- `shares_per_lot` 每一列都有。每張不是 1,000 股的商品只有 `00636K`、`00657K`、`00668K`、
  `00687C`、`008201`（2022-08-10 以前），每張 100 股。
- `currency` 是實際的計價幣別（`USD`、`CNY` 的商品各有數檔）。
- 漲跌停價只有 153 列是 null。
- `daytrade` 取自 FinMind 的當沖清單，與 `F1` 的旗標偶有不同（marketdata 比對 123 天差 25 列，原因
  未查）。
- `match_interval_s` 都是 null：撮合方式由資料推定（[撮合規格](matching.md) 4.1）。
- 權證與 TPEx 可轉換公司債沒有列：2026-10-13 起的 `TWSE_F17`、`TPEX_F17` 與 `TPEX_F6` 的可轉換
  公司債沒有 `shares_per_lot`，回放這些商品必須用 `MissingLotSize::Assume`。

`TWSE_F6` 在 2022-01-03 → 2024-03-29 的差異見第 6 節。

## 11. 目前的限制

- 一個 feed 一天整個留在記憶體：每個 `books` 列約 8 bytes、每個 `trades` 列 48 bytes、每個 `seq`
  4 bytes。2026-09-01 的 `TWSE_F6`（1,711 萬則）約 263 MB；`TWSE_F17` 一天（約 6,900 萬則）的 run
  峰值約 1.4 GB（[效能](../design/performance.md)）。
- 指定 `selection` 時，為了計算 `observed_time`，會另外讀一次整個 feed 的 `seq` 與 `exch_ts`。

## 12. 檢視與比對

```sh
cargo run --release -p osmium-md --example dump -- /srv/md/v1 2026-10-08 TWSE_F6 2330
```

每則訊息輸出一行文字。驗證方式見[驗證](../development/validation.md)。
