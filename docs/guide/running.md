# 執行

指令、設定檔怎麼選、一個 run 需要多少時間與記憶體。每個 key 的精確定義在
[設定規格](../reference/config.md)。

## 1. 指令

```text
osmium run <config.toml> --out <dir>   執行回測，把結果寫到 <dir>
osmium check <config.toml>             檢查設定，列出 run 會讀取的資料，不執行
osmium strategies                      列出這個執行檔的策略
osmium version                         印出結果依賴的各個版本
osmium help
```

- Exit status：0 成功、1 指令失敗（訊息在 stderr，以 `error:` 開頭）、2 指令列看不懂。
- `run` 每完成一個交易日在 stderr 印一行進度，結束時在 stdout 印摘要。
- `<dir>` 必須不存在或是空的。run 失敗時目錄裡留下部分的檔案，但沒有 `run.json`；**有 `run.json`
  才是完整的結果**。
- 你自己的執行檔（[寫策略](writing-a-strategy.md#9-註冊變成執行檔)）有完全相同的指令。

建議的習慣：改過設定就先 `check`；每個 run 一個新的輸出目錄，放在 repository 之外。

## 2. 選資料：`[data]`

```toml
[data]
md_root = "/srv/md/v1"            # 預設值，可省略
feeds = ["TWSE_F6", "TPEX_F6"]
symbols = ["2330", "6488"]        # 省略 = 所選 feed 的全部商品
```

| Feed | 內容 | 有資料的期間 |
| --- | --- | --- |
| `TWSE_F6` | 上市整股：股票、ETF 等 | 2022-01-03 起 |
| `TPEX_F6` | 上櫃整股 | 2024-04-01 起 |
| `TWSE_F17`、`TPEX_F17` | 權證 | 2025-07-31 起 |
| `TWSE_F23`、`TPEX_F23` | 盤中零股 | 2025-08-22 起 |

- **策略只看得到、也只能交易被回放的 book**。要交易零股就要選 `F23`；要交易上櫃股票就要選
  `TPEX_F6`。對沒有被回放的商品送出的委託會被拒絕（`UnknownInstrument`）。
- `symbols` 只影響回放哪些商品，不影響帳務：持有的商品即使不在 `symbols`，除權息與評價照常進行。
  但帳務的強制委託（追繳、到期、未回補的先賣）需要那檔商品被回放才能成交，所以**持有或可能持有
  的商品都要列在 `symbols`**。
- 只列需要的商品與 feed：時間與記憶體大致和回放的訊息數成正比（第 6 節）。
- 2026-10-13 起權證與 TPEx 可轉換公司債沒有商品資料，回放它們必須設定 `assume_lot_size = 1000`，
  否則 run 會以 `instruments has no shares_per_lot` 結束。
- 某些日子缺少某個 feed 時 run 預設會失敗；`missing_days = "skip"` 改成記錄診斷後跳過。
  `osmium check` 會先列出缺少的日子。

日期範圍 `[run]` 的 `start`、`end` 都包含在內，run 處理其中的每個交易日。第一個交易日的前一個
交易日也必須有盤後資料（`daily`），用來取得前一日收盤價。

## 3. 延遲：`[latency]`

三個值都是微秒，沒有預設值。

| Key | 意義 | 影響 |
| --- | --- | --- |
| `market_data` | 交易所發出行情到策略收到 | 策略看到的市場有多舊 |
| `order` | 策略送出委託到交易所收到 | 吃單時五檔可能已經變了；掛單排得更後面 |
| `report` | 交易所產生回報到策略收到 | 策略多晚才知道成交 |

延遲對吃單與短線策略的結果影響很大，對一天交易一兩次的策略影響很小。不確定時，用你實際環境
量到的數字；沒有的話先用保守（較大）的值，再做敏感度測試：同一個策略用幾組延遲各跑一次，結果
差很多就表示策略依賴速度。

全部設為 0 是合法的，代表「零延遲」的理想情況，只適合當作上界參考。

## 4. 帳戶與費用

```toml
[account]
initial_cash = 5000000
cash_check = "at_order"       # 預設

[fee]
discount = "0.28"             # 手續費 2.8 折；帶小數的值要寫成文字
min_regular = 20              # 整股每筆最低手續費
```

最常需要調整的：

| Key | 預設 | 什麼時候改 |
| --- | --- | --- |
| `account.initial_cash` | 必填 | |
| `account.cash_check` | `"at_order"` | 預設要求下單時就有足夠支付交割的現金。當沖策略只需要支付差價，改成 `"at_settlement"`（交割時現金不足才失敗）|
| `fee.discount` | `1` | 你的券商折扣 |
| `fee.min_regular`、`fee.min_odd` | 20、1 | 你的券商的最低手續費 |
| `account.daytrade_enabled` | `true` | 帳戶沒有開當沖時設為 `false` |
| `daytrade.uncovered` | `"error"` | 先賣後買當天沒買回時：預設 run 失敗；`"force_cover"` 改成隔天強制買回並收借券費 |
| `credit.margin_interest_rate` | `0.0645` | 你的券商的融資利率 |
| `matching.auction_marginal` | `"conservative"` | 集合競價與成交價同價的委託：預設不成交；`"queue"` 依排隊估計成交 |

全部的參數與預設值：[帳務規格](../reference/accounting.md)第 15 節。證交稅率、融資融券成數等
法規數字是內建的，依日期自動套用。

## 5. 公司行動與日曆

除權息、分割、減資由 osmium 依 marketdata 的 `corp_actions` 自動處理。兩種情況需要你介入：

**資料不完整或有誤。** 2022 年以來的列都有股利金額、換股比例與退還股款，發放日也大多有；股票
股利的交付日與減資退款日沒有資料，以除權日後 30 天近似（標記 `approx`）。marketdata 缺某個必要
事實時（例如非 10 元面額股票的配股畸零需要面額），預設 run 以錯誤結束，訊息會指出是哪一檔、
哪一天、缺什麼。兩個選擇：

```toml
[corporate_actions]
missing_terms = "approximate"     # 以參考價推算，結果標記 approx

[[corporate_actions.overrides]]   # 或補上正確的事實（可以有很多項）
symbol = "3432"
ex_date = "2024-03-20"
action = "reduction_cash"
share_ratio = [7, 10]
refund_per_share = 3
cash_date = "2024-04-02"
```

要精確就用 `overrides`（事實來自公開資訊觀測站的公告）；只是想讓 run 跑完就用 `"approximate"`，
並檢查 `entries.csv`、`realized.csv` 中 `approx = true` 的列。

**run 跑到資料的最後幾天。** 成交在兩個交易日後交割（T+2）。marketdata 的日曆只到資料的最後
一天，所以最後兩個交易日的交割日不在日曆裡，run 會以
`the calendar has no settlement day far enough after ...` 結束。補上之後的交易日：

```toml
[calendar]
extra_settlement_days = ["2026-10-19", "2026-10-20"]
```

## 6. 時間與記憶體

實測（2026-11，10 核心的工作站，資料在本機磁碟）：

| Run | 訊息數 | 時間 | 記憶體峰值 |
| --- | --- | --- | --- |
| 一檔商品（2330），`TWSE_F6`，20 個交易日 | 93 萬 | 3 秒 | 136 MB |
| 全市場 `TWSE_F6`＋`TWSE_F23`＋`TPEX_F6`＋`TPEX_F23`，一個交易日 | 2,287 萬 | 6 秒 | 0.7 GB |
| `TWSE_F17`（權證），一個交易日 | 6,899 萬 | 19 秒 | 1.4 GB |

- 一次載入一個交易日，所以記憶體取決於最大的那一天，不取決於 run 的長度；時間與天數成正比。
- 指定 `symbols` 會大幅減少時間與記憶體，但每個檔案仍要讀過一遍，所以不會等比例縮短。
- `F17`（權證）的訊息數是 `F6` 的數倍。只有需要交易權證時才選它。
- 策略本身的成本：每一則被回放的訊息呼叫一次 `on_market`。上表的策略幾乎不做事；回放全市場時
  每則訊息約 0.2 微秒，`on_market` 裡多做的每一件事都直接加在這上面。
- 載入資料會用上所有的核心，回放是單一執行緒；結果與核心數無關。要限制核心數用
  `taskset -c 0-3 osmium run …`。記憶體吃緊時加上環境變數 `MALLOC_TRIM_THRESHOLD_=131072`
  可以再省約三成。細節與量測在[效能](../design/performance.md)。

## 7. 可重現

相同的 marketdata 檔案、設定檔、策略與 osmium 版本，產生位元組完全相同的輸出。`run.json` 記錄了
判斷這件事需要的一切（[讀結果](results.md#7-比較兩個-run)）。要保留一個結果，保留整個輸出目錄
即可：它包含設定檔的副本。
