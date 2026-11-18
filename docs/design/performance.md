# 效能

本頁記錄一個 run 的時間與記憶體花在哪裡、為此做了什麼設計、量到的數字，以及刻意沒有做的事。
規則只有一條：**速度與記憶體的修改不得改變輸出**（[回放規格](../reference/replay.md)第 6 節）。
驗證方法在[驗證](../development/validation.md#效能修改)。

## 1. 量測

2026-11，Intel Core Ultra 5 225（10 核心）、30 GB 記憶體、資料在本機磁碟且已在 page cache；
`cargo build --release`；策略是 `passive_quote`（2330）。時間是 wall clock，記憶體是 process 的
peak RSS（`/usr/bin/time` 的 `maxrss`）。「之前」是最佳化前的 commit `e50a8b6`，兩邊的輸出位元組相同。

| Run | 訊息數 | 之前 | 現在 |
| --- | ---: | --- | --- |
| 2330，`TWSE_F6`，20 個交易日 | 925,272 | 11.0 秒／75 MB | 2.9 秒／136 MB |
| 全市場四個 feed（`TWSE_F6`、`TWSE_F23`、`TPEX_F6`、`TPEX_F23`），1 個交易日 | 22,865,582 | 17.5 秒／3.57 GB | 5.6 秒／0.72 GB |
| 同上，3 個交易日 | 69,992,802 | 53.7 秒／3.94 GB | 17.2 秒／0.82 GB |
| `TWSE_F17`（權證），1 個交易日 | 68,991,877 | 未量測 | 19.4 秒／1.44 GB |

全市場一天的 5.6 秒裡，載入四個 feed 約 1.2 秒，回放約 4.4 秒（每則訊息約 190 ns）。

指定商品的 run 記憶體比之前多（75 → 136 MB）：每條執行緒各有一份解碼中的 row group。

## 2. 載入（`osmium-md`）

marketdata 的 `books`／`trades` 依 `(symbol, seq)` 排序，回放要的是依 `seq`。原本的做法是把每一列
存成約 140 bytes 的紀錄，再把整天依 `seq` 排序；現在 `FeedDay` 照檔案的順序保存資料，回放時才
組出訊息。

- **Row group 平行解碼。** 每個 row group（marketdata 寫成 500,000 列一組）由一條執行緒獨立解碼與
  檢查，結果在呼叫端依檔案順序合併（`parallel::ordered`）。合併與解碼同時進行，所以一個 row group
  的結果只短暫地存在兩份。
- **`books` 列以差分保存。** 同一商品相鄰兩列的五檔通常只差一兩個欄位。每一列存成與該商品前一列
  的差異（`stream.rs`）：一個 tag byte、`exch_ts` 的差、每一邊有變動的價與量。2026-09-01 的
  `TWSE_F6` 是 1,709 萬列、1.40 億 bytes，平均每列 8.2 bytes（原本每則訊息約 140 bytes）；比磁碟上
  ZSTD 壓縮的 parquet（1.73 億 bytes）還小。
- **依 `seq` 的順序表。** 交易所把一個 feed 的訊息編號為 1、2、3…，所以整個 feed 的順序是一張以
  `seq` 為索引的表，每格 4 bytes：商品的索引，以及這則訊息有沒有 `books` 列、`trades` 列
  （`order.rs`）。建表同時完成兩張表的 join 與所有跨表檢查，不需要排序。只讀部分商品時表大部分是
  空的，改為排序（列數少，成本低）。
- **回放時解碼。** `FeedDay::messages` 依順序表逐一取出商品，從該商品的游標解出下一列。每個商品的
  游標持有目前的五檔，所以解一列只是套用差異。
- **`observed_time`。** 整個 feed 的 observed time 是訊息 `exch_ts` 的累計最大值，回放時順便算出，
  不儲存。只讀部分商品時它取決於沒有讀進來的商品，所以另外平行掃一次整個 feed 的 `seq` 與
  `exch_ts`。
- **中間成交的判斷跨 row group。** 一列是不是掃單的中間成交，要看同商品的下一列
  （[marketdata 介面](../reference/marketdata.md)第 6 節）。每個 row group 的最後一列留到合併時、
  看到下一個 row group 的第一列才決定。

一個 feed 一天留在記憶體的量：每個 `books` 列約 8 bytes、每個 `trades` 列 48 bytes、每個 `seq`
4 bytes。2026-09-01 的 `TWSE_F6`（1,711 萬則訊息）合計約 263 MB。

## 3. 回放（`osmium-replay`、`osmium-runner`）

- **訊息只複製一次。** 原本一則訊息從解碼到送達策略被複製約 16 次。現在 `MergedFeeds` 把訊息直接
  放進 `Timeline` 的佇列，`Timeline::next_event` 借出佇列裡的訊息（`Event<'_, E>` 持有
  `&Message`），撮合引擎、行情視圖與策略都讀同一份。
- **沒有委託時不做委託的工作。** 每則訊息之後只在有回報、有策略送出的委託或計時器時才處理它們。
- **`BookMap`。** 每則訊息要查兩次「這本 book 的狀態」（撮合引擎與策略的行情視圖）。`BookId` 是
  商品代碼加一個 board，標準函式庫預設的 SipHash 為抵抗攻擊而設計，在這裡占了約 6 % 的時間；
  `osmium-book` 的 `BookMap` 改用乘法雜湊。沒有任何輸出依賴 map 的走訪順序（需要順序的地方都先
  排序）。

## 4. 決定性

平行處理不影響結果，理由是結構上的，不是碰巧：

- 每個 row group 的解碼只讀自己的列，結果是它的純函數。
- 結果依 row group 在檔案裡的順序合併，與哪一條執行緒先做完無關。
- 錯誤也依檔案順序回報：永遠是檔案裡第一個有問題的列。
- observed time 的平行掃描只用取最大值這個可交換的運算合併。

`crates/osmium-md/tests/synthetic.rs` 把同一份資料切成每組 1 到 4 列的 row group，訊息與錯誤都必須和
不切時相同。實測 1、2、4、10 核心的輸出位元組相同。

## 5. 執行緒與記憶體

載入使用機器上所有可用的核心（`std::thread::available_parallelism`，會遵守 CPU affinity 與 cgroup
的限制）；回放是單一執行緒。沒有設定執行緒數的選項：結果與它無關，要限制時用作業系統的工具
（`taskset -c 0-3 osmium run …`）。全市場四個 feed、1 個交易日：

| 核心數 | 時間 | 記憶體峰值 |
| ---: | ---: | ---: |
| 1 | 9.4 秒 | 474 MB |
| 2 | 7.2 秒 | 547 MB |
| 4 | 6.2 秒 | 610 MB |
| 10 | 5.6 秒 | 727 MB |

核心數多時記憶體較高，原因有二：每條執行緒各有一份解碼中的 row group（每欄一頁解壓縮後的資料），
以及 glibc 的配置器把各執行緒釋放的記憶體留在該執行緒的 arena 裡而不還給作業系統。後者可以用環境
變數關掉，同一個 10 核心的 run 降到 519 MB，時間不變：

```sh
MALLOC_TRIM_THRESHOLD_=131072 osmium run run.toml --out …
```

這只影響 RSS，不影響結果；記憶體吃緊時才需要。

## 6. 沒有做的事

- **衍生快取**（[ADR-0008](decisions/0008-full-market-matching-simulation.md) 允許）：把 marketdata
  轉存成依 `seq` 排序的檔案。現在直接讀 parquet 的載入只占一個 run 的兩成（全市場一天 1.2 秒），
  快取最多省下其中一部分，代價是一份要管理失效的磁碟資料，以及第二條可能和直接讀取不一致的路徑。
  目前不值得。
- **串流回放**（不把整天留在記憶體）：`books` 依 `(symbol, seq)` 排序，要依 `seq` 回放就得同時保有
  每個商品還沒回放的列；這些列以差分保存後已經比壓縮的 parquet 小。全市場一天的峰值是 0.7 GB，
  權證 feed 一天 1.4 GB。
- **預先載入下一個交易日**：能把多日 run 的載入時間藏到回放背後，最多再省兩成時間，代價是記憶體
  加倍。
- **平行回放**：一個交易日是單一條時間線，策略的每個決定都可能依賴之前所有的事件；不同交易日
  之間則由帳務串接。要平行只能在不同的 run 之間（不同參數各跑一個 process）。
- **更換記憶體配置器**：workspace 禁止 `unsafe`，自訂配置器或新增相依套件的代價高於第 5 節的環境
  變數。

## 7. 剩下的時間在哪裡

以 callgrind 量 2026-09-01 的 `TPEX_F6`（441 萬則訊息），依執行的指令數：

| 部分 | 占比 |
| --- | ---: |
| **載入**（所有執行緒合計） | 62 % |
| 　parquet 讀取與 ZSTD 解壓縮（arrow-rs） | 22 % |
| 　把五檔的 list 欄位轉成列並檢查（`BookColumns::body`） | 28 % |
| 　差分編碼 | 4 % |
| **回放**（主執行緒） | 35 % |
| 　從差分還原訊息（`Messages::next`） | 16 % |
| 　跨 feed 合併與時間線 | 5 % |
| 　撮合引擎（`Engine::on_market`） | 7 % |
| 　行情視圖、策略 callback、其餘 | 6 % |

指令數不是時間：載入的 62 % 分散在所有核心上，所以在 10 核心的機器上載入只占 wall clock 的兩成，
回放占八成。也就是說，現在決定一個 run 多快的是主執行緒的回放，其中將近一半是還原訊息
（套用差異、把數量換成股數、建立 `Book`）。

要再加快，先重新量測，不要從這張表猜。量測方法：

```sh
CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release -p osmium-cli
valgrind --tool=callgrind target/release/osmium run run.toml --out /path/outside/the/repository/out
callgrind_annotate --inclusive=yes callgrind.out.<pid>
```
