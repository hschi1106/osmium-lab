# 設定規格

一個 run 的設定檔：TOML，由 `osmium-runner` 的 `RunConfig::parse` 讀取。各參數的意義由擁有它的規格
定義，這裡定義檔案格式、預設值與檢查。

狀態：已實作於 `osmium-runner`。

## 1. 原則

- 不認得的 section 或 key 是錯誤，不會被忽略。
- 所有錯誤訊息以 key 的名稱開頭（例如 `data.feeds: ...`）。
- 金額是整數元。比率、每股金額與價格是十進位數，**帶小數時必須寫成文字**（`"0.001425"`）；TOML 的
  浮點數是二進位，會被拒絕。整數可以直接寫（`1`）。
- 日期寫成文字 `"YYYY-MM-DD"`。
- 設定檔的原文會原封不動複製到輸出（[輸出規格](artifacts.md)）。

## 2. 最小設定

```toml
[run]
start = "2026-09-01"
end = "2026-09-30"

[data]
feeds = ["TWSE_F6"]

[latency]
market_data = 200
order = 1000
report = 1000

[account]
initial_cash = 5000000

[strategy]
name = "buy_and_hold"

[strategy.params]
symbol = "2330"
```

其餘的 section 都可以省略，省略時採用下列各節的預設值。

## 3. `[run]`

| Key | 必填 | 說明 |
| --- | --- | --- |
| `start` | 是 | 第一天（含） |
| `end` | 是 | 最後一天（含），不得早於 `start` |

run 處理範圍內 marketdata `qc/calendar` 的每個交易日。範圍內沒有交易日是錯誤。範圍內有只開一個
市場（上市或上櫃）的日子時 run 失敗。

## 4. `[data]`

| Key | 預設 | 說明 |
| --- | --- | --- |
| `md_root` | `"/srv/md/v1"` | marketdata 的根目錄 |
| `feeds` | 必填 | 要回放的 feed：`"TWSE_F6"`、`"TWSE_F17"`、`"TWSE_F23"`、`"TPEX_F6"`、`"TPEX_F17"`、`"TPEX_F23"` 中的一個以上，不得重複（[marketdata](marketdata.md)）|
| `symbols` | 全部 | 只回放這些商品；省略時回放所選 feed 的所有商品。不得為空列表 |
| `missing_days` | `"error"` | 交易日缺少所選 feed 時：`"error"` 使 run 失敗，`"skip"` 記錄診斷後跳過那個 feed 的那一天（[回放規格](replay.md)第 4 節）|
| `assume_lot_size` | 無 | 被回放的整股商品在 marketdata 沒有 `shares_per_lot` 時假設的交易單位（股）。省略時遇到這樣的商品 run 失敗。`instruments` 是推導資料的日子（2024-04-01 以前、2026-10-13 起）必須設定，通常是 `1000`（[marketdata](marketdata.md)第 10 節）|

- 策略只看得到、也只能交易被回放的 book。要交易零股必須選 `F23` 的 feed。
- 強制委託也需要它的 book 被回放；沒有被回放時強制委託被拒絕，連續 6 個交易日無法成交時 run 失敗
  （[帳務規格](accounting.md)第 12.4 節）。

## 5. `[latency]`

| Key | 必填 | 說明 |
| --- | --- | --- |
| `market_data` | 是 | 行情送達策略的延遲 |
| `order` | 是 | 委託到達交易所的延遲 |
| `report` | 是 | 回報送達策略的延遲 |

單位是微秒，0 到 86,400,000,000。沒有預設值：延遲對結果的影響很大，必須由使用者決定
（[撮合規格](matching.md)第 3 節）。

## 6. `[matching]`

| Key | 預設 | 說明 |
| --- | --- | --- |
| `auction_marginal` | `"conservative"` | 集合競價中與成交價同價的委託如何分配：`"conservative"` 或 `"queue"`（[撮合規格](matching.md)第 7.2 節）|

## 7. `[account]`、`[fee]`、`[daytrade]`、`[disposition]`、`[credit]`

Key、預設值與意義見[帳務規格](accounting.md)第 15 節的表；這裡只列格式與檢查。

| Key | 格式 |
| --- | --- |
| `account.initial_cash` | 必填；整數元，不得為負 |
| `account.cash_check` | `"at_order"`、`"at_settlement"`、`"none"` |
| `account.daily_limit` | 整數元；0 或省略表示不限 |
| `account.daytrade_enabled`、`credit_offset_enabled`、`nhi_enabled` | 布林 |
| `fee.rate`、`fee.discount` | 比率 |
| `fee.rounding`、`credit.interest_rounding` | `"floor"`、`"half_up"`、`"ceil"` |
| `fee.min_regular`、`fee.min_odd` | 整數元 |
| `daytrade.uncovered` | `"error"`、`"force_cover"` |
| `daytrade.borrow_fee_rate` | 比率 |
| `disposition.prepay` | `"all"`、`"none"` |
| `credit.eligibility` | `"marketdata_approx"`、`"none"` |
| `credit.margin_loan_ratio` | 比率，必須小於 1 |
| `credit.short_margin_ratio` | 比率；省略時依帳務規格第 10.1 節的表 |
| `credit.margin_interest_rate`、`short_credit_rate`、`short_fee_rate` | 比率 |
| `credit.call_threshold`、`credit.cancel_threshold` | 比率；`cancel_threshold` 不得低於 `call_threshold` |
| `credit.notice_lag_days`、`max_extensions` | 非負整數 |
| `credit.grace_days`、`credit.term_months` | 正整數 |

比率是 0 以上的十進位數，小數最多 12 位。

## 8. `[corporate_actions]`

| Key | 預設 | 說明 |
| --- | --- | --- |
| `cash_pay_lag_days` | 30 | 帳務規格第 13.1 節 |
| `stock_credit_lag_days` | 30 | 同上 |
| `missing_terms` | `"error"` | `"error"` 或 `"approximate"` |

`[[corporate_actions.overrides]]` 可以出現多次，每一項補上 marketdata 沒有的事實：

| Key | 必填 | 格式 |
| --- | --- | --- |
| `symbol`、`ex_date`、`action` | 是 | 指定 `corp_actions` 的一列；`action` 是 marketdata 的值（`"dividend"`、`"rights"`、`"split"`、`"reverse_split"`、`"par_change"`、`"reduction_cash"`、`"reduction_loss"` 等）|
| `cash_date`、`stock_date` | 否 | 現金發放日、股票配發日；不得早於 `ex_date` |
| `share_ratio` | 否 | `[新股數, 舊股數]`，兩個正整數 |
| `refund_per_share`、`cash_dividend`、`stock_dividend`、`par_value` | 否 | 每股金額（元），小數最多 10 位；`par_value` 必須為正 |
| `fraction_price` | 否 | 畸零股折算現金的價格 |

同一個 `(symbol, ex_date, action)` 只能出現一次。各欄位何時需要見帳務規格第 13.1 節。

```toml
[[corporate_actions.overrides]]
symbol = "3432"
ex_date = "2024-03-20"
action = "reduction_cash"
share_ratio = [7, 10]
refund_per_share = 3
cash_date = "2024-04-02"
```

## 9. `[calendar]`

| Key | 預設 | 說明 |
| --- | --- | --- |
| `extra_settlement_days` | 空 | 額外的交割日（日期的列表）|

交割日由 `qc/calendar` 的交易日推算。run 的最後幾個交易日，其交割日可能在 `qc/calendar` 的最後一天
之後；此時必須在這裡列出那些日子，否則 run 以帳務錯誤結束（[帳務規格](accounting.md)第 7.1 節）。

## 10. `[strategy]`

| Key | 必填 | 說明 |
| --- | --- | --- |
| `name` | 是 | 程式登記的策略名稱（`osmium strategies` 列出）|

## 11. 完整範例

```toml
[run]
start = "2026-09-01"
end = "2026-09-30"

[data]
md_root = "/srv/md/v1"
feeds = ["TWSE_F6", "TWSE_F23"]
symbols = ["2330", "0050"]
missing_days = "error"

[latency]
market_data = 200
order = 1000
report = 1000

[matching]
auction_marginal = "conservative"

[account]
initial_cash = 5000000
cash_check = "at_order"

[fee]
discount = "0.28"
min_regular = 20

[daytrade]
uncovered = "force_cover"

[credit]
margin_interest_rate = "0.0645"

[corporate_actions]
missing_terms = "approximate"

[strategy]
name = "open_to_close"

[strategy.params]
symbol = "2330"
lots = 2
```

## 12. `[strategy.params]`

策略自己的參數，內容由策略定義（[策略介面規格](strategy.md)第 6 節）。值可以是整數、文字、布林，
或它們的列表；浮點數、日期時間與巢狀的 table 會被拒絕。策略不認得的參數是錯誤。
