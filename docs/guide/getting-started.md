# 安裝與第一個回測

這一頁從零開始：編譯、跑一個內建的範例策略、看懂它印出的東西。約十分鐘。

## 1. 需要什麼

- 可以讀取 marketdata 的機器（預設路徑 `/srv/md/v1`）。osmium 只讀取，不會寫入這個目錄。
- Rust toolchain。版本固定在 `rust-toolchain.toml`，`rustup` 會自動安裝。
- 記憶體：只回放幾檔商品時不到 1 GB；回放全市場時見[執行](running.md#6-時間與記憶體)。

## 2. 編譯

```sh
cargo build --release
```

產生 `target/release/osmium`。以下把它簡寫成 `osmium`。務必用 `--release`：debug 版慢很多。

```sh
osmium version      # 結果依賴的各個版本
osmium strategies   # 這個執行檔內建的策略
```

`osmium` 內建三個範例策略（[`crates/osmium-examples`](../../crates/osmium-examples/src/)）：

| 名稱 | 做什麼 |
| --- | --- |
| `buy_and_hold` | 開盤集合競價買進 `lots` 張 `symbol`，之後不動 |
| `open_to_close` | 開盤買進、收盤集合競價賣出（現股當沖） |
| `passive_quote` | 掛在買一排隊買一張，成交後掛在賣一賣出 |

它們示範介面怎麼用，不是賺錢的方法。

## 3. 寫設定檔

一個回測（run）由一個 TOML 設定檔描述。存成 `first.toml`：

```toml
[run]
start = "2026-09-01"
end = "2026-09-04"

[data]
feeds = ["TWSE_F6"]      # 上市整股的行情
symbols = ["2330"]       # 只回放這檔；省略這一行就是整個 feed

[latency]                # 微秒；沒有預設值，必須自己決定
market_data = 200        # 行情送到策略
order = 1000             # 委託送到交易所
report = 1000            # 回報送回策略

[account]
initial_cash = 5000000   # 元

[strategy]
name = "open_to_close"

[strategy.params]
symbol = "2330"
```

每個 key 的意義與預設值見[設定規格](../reference/config.md)；怎麼選見[執行](running.md)。

## 4. 先檢查，再執行

```sh
osmium check first.toml
```

```text
strategy      open_to_close (version 1)
marketdata    /srv/md/v1
feeds         TWSE_F6
instruments   1
trading days  4 (2026-09-01 to 2026-09-04)
missing       none
```

`check` 不回放行情，只確認設定正確、策略與參數存在、每個交易日的資料都在。`missing` 不是 `none`
時，它會列出缺少哪一天的哪個 feed。

```sh
osmium run first.toml --out ~/osmium-runs/first
```

```text
2026-09-01 messages=43898 orders=2 fills=2 equity=5034451
2026-09-02 messages=49367 orders=2 fills=2 equity=4994035
2026-09-03 messages=44120 orders=2 fills=2 equity=4988647
2026-09-04 messages=46821 orders=2 fills=2 equity=4973157
trading days  4
messages      184206
orders        8
fills         8
initial cash  5000000
final cash    4994035
final equity  4973157
results       /home/you/osmium-runs/first
```

每完成一個交易日印一行進度（stderr），最後印摘要（stdout）。輸出目錄必須不存在或是空的：osmium
不會覆寫既有的結果。

## 5. 看結果

輸出目錄裡是幾個 CSV 與一個 `run.json`：

```text
config.toml       設定檔的副本
orders.csv        送出的每一筆委託
order_events.csv  每一筆回報：接受、成交、取消、拒絕、失效
entries.csv       每一筆現金分錄：價金、手續費、證交稅、利息、股利
realized.csv      每一次平倉的已實現損益
daily.csv         每天日結：現金、待交割款、權益
positions.csv     每天日結後的部位
diagnostics.csv   run 照常進行、但你應該知道的事
run.json          版本、讀了哪些資料檔、每個輸出檔的雜湊；最後才寫
```

第一天發生了什麼：

```text
$ head -5 ~/osmium-runs/first/order_events.csv
order_id,time,event,price,qty,remaining,basis,trigger_feed,trigger_seq,request,reason
1,2026-09-01T08:30:00.001000+08:00,Accepted,,,,,,,,
1,2026-09-01T09:00:09.701792+08:00,Filled,2395,1000,,AuctionBetter,TWSE_F6,124996,,
2,2026-09-01T13:25:00.001000+08:00,Accepted,,,,,,,,
2,2026-09-01T13:30:10.592090+08:00,Filled,2440,1000,,AuctionBetter,TWSE_F6,17111537,,
```

- 委託 1 在 08:30:00 送出，1,000 微秒後（`latency.order`）到達交易所，在開盤集合競價以開盤價
  2,395 成交 1,000 股。`trigger_feed`、`trigger_seq` 指出是哪一則行情訊息促成這筆成交，可以回
  marketdata 查證。
- 委託 2 在 13:25 送出，在收盤集合競價以收盤價 2,440 成交。

```text
$ head -3 ~/osmium-runs/first/realized.csv
date,symbol,kind,shares,opened,proceeds,cost,interest,pnl,approx
2026-09-01,2330,DayTrade,1000,2026-09-01,2432863,2398412,0,34451,false
```

這一天的損益 34,451 元是這樣來的：

```text
賣出價金  2,440,000
買進價金 −2,395,000
買進手續費   −3,412   （2,395,000 × 0.1425 %，無條件捨去）
賣出手續費   −3,477
證交稅       −3,660   （當沖 0.15 %）
          ─────────
             34,451
```

每一筆金額都在 `entries.csv` 有一列。完整的說明見[讀結果](results.md)。

## 6. 接下來

- [寫自己的策略](writing-a-strategy.md)
- [執行](running.md)：選 feed、選商品、延遲、帳戶與費率、時間與記憶體
- [讀結果](results.md)
- [疑難排解](troubleshooting.md)
