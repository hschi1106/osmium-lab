# 輸出規格

一個 run 寫出的檔案：內容、欄位與識別資訊。由 `osmium-runner` 寫出。

狀態：已實作；格式版本 `output_version = 2`（[版本](versions.md)）。

## 1. 原則

- 輸出目錄由呼叫端指定（`osmium run <config> --out <dir>`），必須不存在或是空的；run 不會覆寫
  既有的結果。
- `run.json` 最後才寫。**目錄裡有 `run.json` 才是完整的結果**；失敗的 run 留下沒有 `run.json` 的
  目錄。
- 輸出只由輸入決定（[回放規格](replay.md)第 6 節）：相同的 marketdata 檔案、設定、策略與 osmium
  版本產生位元組相同的檔案。輸出不含執行時間、主機名稱等環境資訊。
- 表格是 CSV：UTF-8、`\n` 換行、第一列是欄位名稱。空欄位表示沒有值。含逗號、引號或換行的文字欄位
  以雙引號包住，內部的雙引號重複一次。
- 金額是整數元；價格是十進位文字（`100.5`）；數量是股；時刻是台灣時間
  `2026-10-15T09:00:05.001100+08:00`（微秒）。
- 每個表格的列依產生順序排列，也就是時間線的順序。

## 2. `run.json`

| 欄位 | 內容 |
| --- | --- |
| `output_version` | 本規格的版本 |
| `versions` | `osmium`、`md_schema`、`decoder`、`ordering`、`matching_rules`、`ledger_rules`（[版本](versions.md)）|
| `strategy` | `name` 與 `version` |
| `config_blake3` | `config.toml` 的 BLAKE3 |
| `md_root`、`start`、`end` | 設定的資料根目錄與日期範圍 |
| `reference` | 讀取的每個參考表檔案在 marketdata `fm/_build` 的身分：`path`（相對於 `md_root`）、`rows`、`git_commit`、`content_hash`（內容的雜湊，與壓縮與檔案切法無關）、`file_sha256`（檔案位元組的 SHA-256）|
| `days` | 每個交易日：`date`、`messages`、`orders`、`fills`、`cash`、`equity`，以及 `files` |
| `days[].files` | 當天讀取的每個檔案的 manifest identity：`table`、`part`、`path`、`rows`、`git_commit`、`code_hash`、`inputs_hash` |
| `summary` | `trading_days`、`messages`、`orders`、`fills`、`initial_cash`、`final_cash`、`final_equity` |
| `files` | 其他每個輸出檔案的 `name`、`rows`（資料列數）與 `blake3` |

兩個 run 的輸入是否相同，比對 `versions`、`strategy`、`config_blake3`、`reference` 與 `days[].files`。
參考表（`ref/calendar`、`corp_actions`、`events`、`daily`、`daily_ext`）在 marketdata 沒有 manifest
列，以 `fm/_build` 的列識別；`content_hash` 相同就是相同的內容（[marketdata](marketdata.md)第 9 節）。

## 3. `config.toml`

設定檔的原文，逐位元組複製。

## 4. `orders.csv`

策略送出的每一筆新委託與帳務送出的每一筆強制委託，各一列。

| 欄位 | 內容 |
| --- | --- |
| `order_id` | 整個 run 內唯一，依送出順序遞增 |
| `date` | 交易日 |
| `sent_time` | 送出時刻；強制委託是 08:30:00 |
| `arrival_time` | 到達交易所的時刻；強制委託是該盤別開始收單的時刻 |
| `symbol`、`board`、`side`、`kind` | `board` 是 `REG`／`ODD`；`side` 是 `buy`／`sell`；`kind` 見[策略介面規格](strategy.md)第 3.1 節 |
| `price` | 限價，或 `market` |
| `time_in_force` | `ROD`、`IOC`、`FOK` |
| `qty` | 股數 |
| `forced` | 強制委託的原因：`MarginCall`、`Expired`、`ShortSuspension`、`UncoveredSellFirst`；策略的委託為空 |

改單與刪單的請求不在這個表；它們的結果在 `order_events.csv`。

## 5. `order_events.csv`

每一筆回報一列，`time` 是事件在交易所發生的時刻。

| `event` | 填寫的欄位 |
| --- | --- |
| `Accepted` | — |
| `Rejected` | `request`（`New`、`Cancel`、`Reduce`、`Reprice`）、`reason` |
| `Filled` | `price`、`qty`、`basis`、`trigger_feed`、`trigger_seq` |
| `Cancelled` | `qty`、`reason`（`Requested`、`IocRemainder`、`FokUnfilled`、`MarketRemainder`）|
| `Reduced` | `qty`（減少的量）、`remaining` |
| `Repriced` | `price` |
| `Expired` | `qty` |
| `PriceBandLimited` | — |

- `basis` 是成交依據（[撮合規格](matching.md)第 8 節）。
- `trigger_feed`、`trigger_seq` 是促成這筆成交的行情訊息；委託到達時立即成交（`Aggressive`）沒有
  觸發訊息。
- 拒絕原因的值與意義見撮合規格第 5.2 節（交易所的檢查）與[帳務規格](accounting.md)第 4 節（帳戶的
  檢查）。

## 6. `entries.csv`

帳務的現金分錄：每一筆會在交割日改變 `cash` 的金額。

| 欄位 | 內容 |
| --- | --- |
| `date` | 發生的交易日 |
| `settles` | 入帳（交割）日 |
| `symbol` | 商品 |
| `order_id` | 來源委託；不屬於單一委託的分錄（利息、股利、借券費）為空 |
| `item` | 見下表 |
| `amount` | 整數元；收入為正、支出為負 |
| `approx` | `true` 表示金額依近似規則計算（帳務規格標示為近似的項目）|

`item`：`TradeValue`（成交價金）、`Fee`（手續費）、`Tax`（證交稅）、`ShortFee`（融券手續費）、
`MarginLoan`（融資金額）、`MarginInterest`、`ShortMargin`（融券保證金）、`ShortCollateral`
（融券擔保價款）、`ShortInterest`、`CashDividend`、`FractionCash`（畸零股折現）、`CapitalRefund`
（減資退還股款）、`NhiPremium`（二代健保補充保費）、`BorrowFee`（先賣後買未回補的借券費）。

期末 `cash` = `account.initial_cash` + 所有 `settles` 不晚於最後一個交易日的 `amount` 之和。

## 7. `realized.csv`

每一次平倉（或配對）一列。

| 欄位 | 內容 |
| --- | --- |
| `date` | 認列的交易日 |
| `symbol` | 商品 |
| `kind` | `Cash`（現股賣出）、`DayTrade`（現股當沖配對）、`CreditOffset`（資券相抵）、`Margin`（融資賣出）、`Short`（融券回補）、`SellFirstCover`（先賣後買隔日回補）、`CorporateAction`（公司行動產生的損益）|
| `shares` | 平倉的股數 |
| `opened` | 開倉日 |
| `proceeds` | 賣出淨額 |
| `cost` | 買進成本 |
| `interest` | 利息（融資利息為正的費用；融券利息為收入）|
| `pnl` | 已實現損益 |
| `approx` | 同 `entries.csv` |

各欄位的計算見[帳務規格](accounting.md)第 14 節。

## 8. `daily.csv`

每個交易日日結後一列。

| 欄位 | 內容 |
| --- | --- |
| `date` | 交易日 |
| `settles` | 當天成交的交割日 |
| `cash` | 已交割的現金 |
| `pending` | 尚未交割的待交割款淨額 |
| `receivable` | 應收（付）的公司行動款項 |
| `equity` | 權益（帳務規格第 14 節）|
| `maintenance` | 整戶維持率，小數 6 位、無條件捨去；沒有融資融券部位時為空 |
| `margin_call` | 當天是否處於追繳 |

## 9. `positions.csv`

每個交易日日結後，每個有部位的商品一列（依代號排序）。沒有部位的日子沒有列。

| 欄位 | 內容 |
| --- | --- |
| `date`、`symbol` | |
| `cash_shares` | 現股股數 |
| `margin_shares` | 融資股數 |
| `short_shares` | 融券股數 |
| `owed_shares` | 先賣後買尚未回補的股數 |
| `price` | 評價價格（帳務規格第 12.1 節）；從未有過價格時為空 |

## 10. `diagnostics.csv`

run 照常進行、但使用者應該知道的情形。

| `kind` | 意義 |
| --- | --- |
| `feed_skipped` | 當天缺少這個 feed，依 `data.missing_days = "skip"` 跳過；`detail` 是 feed |
| `lot_size_assumed` | 當天有商品的交易單位採用 `data.assume_lot_size`；`detail` 是商品數 |
| `data_quality` | 讀取的檔案在 manifest 標示 `seq_gaps`、`seq_dup` 不為 0 或 `partial` |
| `untradable_books` | 當天被回放但不能模擬委託的 book 數與原因（`UnknownInstrument`、`UnsupportedInstrument`）|
| `no_credit_quotas` | 前一交易日的 `daily_ext` 沒有任何列：當天不能融資買進或融券賣出 |
| `stale_position` | 持有的商品不在當天的商品資料中，沿用上一次的評價價格 |
| `aggressor_unknown`、`limit_unknown` | 撮合模擬當天無法判斷主動方、缺少漲跌停價的次數（[撮合規格](matching.md)第 10 節）|
