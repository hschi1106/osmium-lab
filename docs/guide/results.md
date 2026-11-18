# 讀結果

一個 run 的輸出目錄裡有什麼、怎麼回答常見的問題。每個檔案每一欄的精確定義在
[輸出規格](../reference/artifacts.md)。

## 1. 先確認 run 是完整的

目錄裡有 `run.json` 才是完整的結果：它最後才寫。沒有它表示 run 中途失敗，其他檔案只到失敗的那
一天。

所有表格都是 CSV（UTF-8、第一列是欄位名稱），可以直接用 pandas、polars、DuckDB 或試算表打開。
金額是整數元，數量是股，時刻是台灣時間（微秒）。

```python
import polars as pl
run = "/home/you/osmium-runs/first"
daily = pl.read_csv(f"{run}/daily.csv", try_parse_dates=True)
events = pl.read_csv(f"{run}/order_events.csv")
```

## 2. 賺了多少：`daily.csv`

每個交易日日結後一列。

```text
date,settles,cash,pending,receivable,equity,maintenance,margin_call
2026-09-01,2026-09-03,5000000,34451,0,5034451,,false
2026-09-02,2026-09-04,5000000,-5965,0,4994035,,false
2026-09-03,2026-09-07,5034451,-45804,0,4988647,,false
```

- `equity`（權益）是績效曲線：現金＋待交割款＋應收的股利等＋部位的市值（融資扣掉借款、融券加上
  擔保品）。總損益 = 最後一天的 `equity` − `account.initial_cash`。
- `cash` 只包含**已經交割**的現金。9 月 1 日的損益 34,451 元在 9 月 3 日（`settles`）才進 `cash`；
  在那之前它在 `pending`（待交割款）。所以看績效要看 `equity`，不是 `cash`。
- 部位以當天的收盤價評價（marketdata `daily`）；當天沒有收盤價時沿用上一次的價格。
- `maintenance` 是整戶維持率，只有持有融資融券部位時才有值；`margin_call` 是當天是否被追繳。

日報酬、最大回撤、Sharpe 等都由 `equity` 這一欄算出來；osmium 不替你算，因為定義因人而異。

## 3. 每一筆委託發生了什麼：`orders.csv`、`order_events.csv`

`orders.csv` 是送出的委託（策略的與帳務強制的），`order_events.csv` 是它們的回報，用 `order_id`
串起來。

```text
order_id,time,event,price,qty,remaining,basis,trigger_feed,trigger_seq,request,reason
1,2026-09-01T08:30:00.001000+08:00,Accepted,,,,,,,,
1,2026-09-01T09:00:09.701792+08:00,Filled,2395,1000,,AuctionBetter,TWSE_F6,124996,,
```

`time` 是事件在交易所發生的時刻（不是策略收到的時刻；策略收到是 `time + latency.report`）。

**成交依據 `basis`** 說明這筆成交是依哪一條規則認定的，是判斷回測可信度最重要的一欄：

| `basis` | 意義 | 可信度 |
| --- | --- | --- |
| `Aggressive` | 委託到達時吃掉揭示的對手方五檔 | 高：量與價都是當下揭示的 |
| `PassiveThrough` | 真實成交穿過你的委託價 | 高：價位上排在前面的都已成交 |
| `PassiveQueue` | 真實成交發生在你的委託價，而且排在你前面的量已消耗完 | 取決於排隊估計（刪單都假設在你後面，所以偏保守）|
| `CrossedPassive` | 掛單後對手方出現可成交的量 | 高 |
| `AuctionBetter` | 集合競價，你的委託價優於成交價 | 高：一定成交 |
| `AuctionOpposite`、`AuctionQueue`、`AuctionResidual` | 集合競價與成交價同價時的分配 | 只有 `auction_marginal = "queue"` 時出現；是估計 |

`trigger_feed`、`trigger_seq` 是促成成交的那一則 marketdata 訊息，可以回原始資料查證：

```python
books = pl.scan_parquet("/srv/md/v1/books/2026/2026-09-01/TWSE_F6.parquet")
books.filter(pl.col("seq") == 124996).collect()
```

**委託被拒絕**時 `event` 是 `Rejected`，`reason` 是原因。常見的：

| `reason` | 意義 |
| --- | --- |
| `InsufficientCash` | 可用資金不足（[執行](running.md#4-帳戶與費用)的 `cash_check`）|
| `InsufficientPosition` | 可賣的股數不足 |
| `UnknownInstrument` | 這檔商品當天沒有被回放（不在 `data.symbols` 或所選 feed）|
| `UnsupportedInstrument` | 商品類別不支援，或缺少交易單位 |
| `MarketClosed` | 不在收單時間內 |
| `InvalidQuantity`、`InvalidTick`、`PriceOutOfLimit` | 數量、升降單位、漲跌停 |
| `OrderTypeNotAllowed` | 集合競價期間送了 IOC／FOK／市價單 |
| `DayTradeNotAllowed`、`ShortNotAllowed`、`KindNotAllowed` | 這檔商品或帳戶不能做這種交易 |
| `TooLate`、`UnknownOrder` | 改單、刪單時委託已經結束或不存在 |

完整的清單：[撮合規格](../reference/matching.md)第 5.2 節、[帳務規格](../reference/accounting.md)
第 4 節。

常用的統計：

```python
fills = events.filter(pl.col("event") == "Filled")
fills.group_by("basis").agg(pl.len(), pl.col("qty").sum())      # 成交依據的分布
events.filter(pl.col("event") == "Rejected").group_by("reason").len()   # 拒絕原因
```

## 4. 錢花到哪裡：`entries.csv`

每一筆會改變現金的金額一列：成交價金、手續費、證交稅、融資利息、股利、補充保費等。收入為正、
支出為負。

```text
date,settles,symbol,order_id,item,amount,approx
2026-09-01,2026-09-03,2330,1,TradeValue,-2395000,false
2026-09-01,2026-09-03,2330,1,Fee,-3412,false
2026-09-01,2026-09-03,2330,2,TradeValue,2440000,false
2026-09-01,2026-09-03,2330,2,Fee,-3477,false
2026-09-01,2026-09-03,2330,2,Tax,-3660,false
```

```python
entries = pl.read_csv(f"{run}/entries.csv")
entries.group_by("item").agg(pl.col("amount").sum())     # 交易成本的組成
```

這是帳務的完整明細：期末 `cash` 等於期初現金加上所有已交割分錄的總和，可以自己加總驗算。

## 5. 每一次平倉的損益：`realized.csv`

每次賣出庫存、回補、當沖配對一列，`pnl` 是已實現損益（已扣手續費、稅、利息）。

```text
date,symbol,kind,shares,opened,proceeds,cost,interest,pnl,approx
2026-09-01,2330,DayTrade,1000,2026-09-01,2432863,2398412,0,34451,false
```

- `kind`：`Cash`（賣出庫存）、`DayTrade`（當沖）、`Margin`（融資賣出）、`Short`（融券回補）、
  `CreditOffset`（資券相抵）、`SellFirstCover`（先賣後買隔日回補）、`CorporateAction`。
- 現股成本採先進先出。`opened` 是開倉日，`date − opened` 就是持有期間。
- 權益的變動 = 已實現損益 + 未實現損益的變動 + 股利等收入。還沒賣出的部位不在這個表，在
  `positions.csv`。

勝率、平均損益、持有期間都從這個表算。

## 6. 部位與診斷：`positions.csv`、`diagnostics.csv`

`positions.csv`：每天日結後每個有部位的商品一列（現股、融資、融券、先賣未回補的股數與評價價格）。

`diagnostics.csv`：run 沒有失敗、但你應該知道的事。**每個 run 都要看一眼。**

| `kind` | 要不要緊 |
| --- | --- |
| `feed_skipped` | 那一天那個 feed 沒有資料，策略沒有收到它的行情 |
| `lot_size_assumed` | 那一天的交易單位是假設的（`assume_lot_size`）；每張不是 1,000 股的商品會算錯 |
| `data_quality` | marketdata 標示那個檔案有封包遺失或不完整；那一天的成交可能偏少 |
| `untradable_books` | 有些 book 被回放但不能下單（例如外幣計價、不支援的商品類別）；策略沒有交易它們就不要緊 |
| `no_credit_quotas` | 前一交易日沒有信用額度資料，當天不能融資買進、融券賣出 |
| `stale_position` | 持有的商品當天不在商品資料中（可能下市或暫停交易），沿用舊的評價價格 |
| `aggressor_unknown` | 無法判斷主動方的真實成交數；它們不會讓你的掛單成交。通常佔極小比例 |
| `limit_unknown` | 缺少漲跌停價，相關檢查略過 |

另外，`entries.csv` 與 `realized.csv` 的 `approx = true` 表示那一列的金額是依近似規則算的（缺少
公司行動的條件、沒有當沖資格旗標的日子等），不是精確值。

## 7. 比較兩個 run

`run.json` 記錄了結果依賴的一切：

```text
versions       osmium、marketdata schema、decoder、ordering、matching rules、ledger rules
strategy       名稱與版本
config_blake3  設定檔的雜湊
reference      讀取的參考表檔案
days[].files   每個交易日讀取的每個 marketdata 檔案的 identity
files          每個輸出檔案的列數與 BLAKE3
summary        交易日數、訊息數、委託數、成交數、期初現金、期末現金、期末權益
```

- 兩個 run 的 `versions`、`strategy`、`config_blake3`、`reference`、`days[].files` 都相同時，
  `files` 裡每個檔案的 `blake3` 也必須相同。不同就是 bug，請回報。
- 結果不同時，從這幾項找出是什麼變了：設定、策略版本、osmium 的規則版本，還是 marketdata 重新
  產生過（`git_commit`、`inputs_hash` 不同）。
- 改了策略的邏輯卻沒有改它的 `version`，兩份結果就無從分辨。

```sh
diff <(jq -S 'del(.files, .days, .summary)' a/run.json) <(jq -S 'del(.files, .days, .summary)' b/run.json)
```

## 8. 回測結果的限制

結果是「在不影響市場的假設下，這個策略最多能拿到什麼」的保守估計，不是預言。解讀時記得：

- 委託的量相對於市場成交量不小時，「不影響市場」的假設不成立，結果會偏樂觀。
- 掛單的成交依賴排隊估計；`PassiveQueue` 佔成交的比例很高時，結果對這個估計敏感。
- 延遲是你給的參數；真實的延遲會變動。
- 手續費與稅的捨入採業界常見做法，尚未以實際對帳單校驗；信用交易資格、部分公司行動是近似。

完整的清單：[撮合規格](../reference/matching.md)第 10 節、[帳務規格](../reference/accounting.md)
第 16 節。
