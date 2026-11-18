# marketdata 介面

`osmium-md` 是 osmium 唯一的資料來源 adapter：直接讀取 marketdata（預設 `/srv/md/v1`）的 parquet，
不建立第二份 cache，也不寫入 marketdata 目錄。設計決策見
[ADR-0007](../design/decisions/0007-marketdata-native.md)。

## 1. 支援範圍

| `Feed` | marketdata part | 市場 | board | 數量單位 |
| --- | --- | --- | --- | --- |
| `TwseF6`／`TpexF6` | `TWSE_F6`／`TPEX_F6` | TWSE／TPEx | 一般 | `TradingUnit`（張） |
| `TwseF17`／`TpexF17` | `TWSE_F17`／`TPEX_F17` | TWSE／TPEx | 一般（權證） | `TradingUnit` |
| `TwseF23`／`TpexF23` | `TWSE_F23`／`TPEX_F23` | TWSE／TPEx | 盤中零股 | `Share` |

event 的 `source_format` 即 part 名稱。TAIFEX、`sinopac_ticks`、`bars_1m`、`daily` 尚未支援。

目前限制：

- 一般與零股 book 共用同一個 `InstrumentId`；同一 run 不可同時載入同一 symbol 的 F6 與 F23。
- 已選 symbol 的列以 compact 形式全部留在記憶體，event 於 stream 讀取時才 mapping；全市場單日
  回播的記憶體用量尚未最佳化。
- `MarketAnnotations` 一律為 `None`；漲跌停狀態（`limit_state`）尚未轉成中性欄位。

## 2. 讀取與一致性

`FeedDay::load(root, date, feed, symbols)`：

1. 讀當日 `manifest/{YYYY}/{date}/*.parquet`。目錄不存在即 `MissingManifest`；沒有該 table／part 的
   row 即 `MissingManifestEntry`（代表當天沒有這份資料）。
2. 開檔時驗證 manifest `path`、`rows`，以及 footer 的 `md.schema_version`、`md.git_commit` 與
   manifest 一致。
3. 以 `symbol` row-group statistics 剪枝（檔案依 `(symbol, seq)` 排序），只解碼需要的欄位。
4. 讀完後重新讀 manifest；books、trades、instruments 任一 row 改變即 `ManifestChanged`。

`FeedDay::identity()` 回傳三個 manifest row（含 `code_hash`、`inputs_hash`、`git_commit`），供 run
manifest 記錄。

## 3. 數值

價格必須是 1e-4 decimal 的最近 double：`ticks = round(x × 10⁴)`，且 `ticks / 10⁴ == x` 才接受，
否則 `NonCanonicalPrice`。數量必須為正整數。揭示檔數（`n_bid`／`n_ask`）內的價量必須存在，超出
的必須為 null；`n_*` 為 null 表示該列沒有揭示委託簿。

## 4. 訊息與時間

同一 symbol 的 `books` 與 `trades` 依 `seq` 合併成訊息；`has_trade` 必須與 trades 是否有同 `seq`
的列一致。`sub_seq ≠ 0`（一則訊息多筆成交）目前拒絕。

- `match_time` = `exch_ts`。同一 symbol 內 `exch_ts` 倒退即錯誤。
- `observed_time` = 整個 feed（含未選取的 symbol，books 與 trades）中 `seq ≤` 本訊息者的最大
  `exch_ts`。因此 observed time 與 universe 選擇無關。
- `source_sequence` = `seq`。

## 5. Mapping

| 訊息 | Payload | `MarketSignal` |
| --- | --- | --- |
| `has_trade` books 列 + 同 `seq` trade | `QuoteSnapshot`（trade `Regular`，`cum_qty` 為 `Set`） | 依 phase |
| 只有 trade（中間成交，僅 `CONTINUOUS`） | `TradeBatch`（`Intermediate`） | `Continuous` |
| 一般 books 列 | `BookSnapshot` | 依 phase |
| `is_indicative` 試撮 | `IndicativeAuction`；null 價量為 `Clear` | 由 auction observation 推導 |
| `delay_state ≠ 0` 暫緩撮合（無委託簿） | `MarketStatus` | `AuctionCollecting` |

依 phase 的 signal：`CONTINUOUS` → `Continuous`；其他 phase 的非試撮列 → `AuctionUncross`。

Auction observation：

| phase | purpose | delayed | direction |
| --- | --- | --- | --- |
| `OPEN_AUCTION` | `Opening` | 一般 board：`delay_open`（null → `Unknown`）；零股：`NoObservation` | `NoObservation` |
| `CLOSE_AUCTION` | `Closing` | 同上，使用 `delay_close` | `NoObservation` |
| `PERIODIC_AUCTION`，零股或 `match_interval_s > 0` | `Periodic` | 本輪是否出現過暫緩撮合列 | `NoObservation` |
| `PERIODIC_AUCTION`，`match_interval_s == 0` 且前有暫緩撮合列 | `VolatilityInterruption` | `NoObservation` | 暫緩撮合列的方向 |
| `PERIODIC_AUCTION`，其餘（含 instruments 缺 `match_interval_s`） | `Unknown` | `NoObservation` | `NoObservation` |

`delay_state` 1／-1 分別為趨漲／趨跌。`disposal` 一律 `NoObservation`。進入 `CONTINUOUS` 時清除
volatility interruption 狀態；periodic round 於 uncross 列後結束。開盤／收盤集合競價中的暫緩撮合列、
`CONTINUOUS` 的試撮列等未觀察過的組合一律拒絕，不猜測語義。

## 6. Market-state profile

`Feed::state_profile()` 接受上述五種 event kind，累計成交量以該 feed 的數量單位檢查非遞減。
