# 策略介面規格

策略與 run 之間的介面（`osmium-strategy`）：策略何時被呼叫、看得到什麼、能做什麼。怎麼寫一個策略的
逐步說明見[使用手冊](../guide/writing-a-strategy.md)；事件的全序見[回放規格](replay.md)。

狀態：已實作於 `osmium-strategy`，由 `osmium-runner` 驅動。

## 1. 原則

- 策略是編譯進程式的 Rust 型別，實作 `Strategy` trait；每個方法都有空的預設實作。
- 策略只在被呼叫時行動。它看到的行情與帳戶，是回放時鐘當下「已送達策略」的部分，不是交易所當下的
  狀態：行情晚 `latency.market_data`，回報晚 `latency.report`。
- 策略不得讀取 `Context` 以外的市場資料、系統時間或亂數。相同的輸入必須做出相同的決定，否則 run
  不具[決定性](replay.md)。需要亂數時用參數給定種子、自行實作產生器。
- 策略的狀態是它自己的欄位。run 只建立一個策略實例，跨交易日沿用。

## 2. 一個交易日的呼叫順序

| 順序 | 方法 | 時刻 | 說明 |
| --- | --- | --- | --- |
| 1 | `on_forced_order` | 08:30:00 | 帳務當天要代為送出的每一筆強制委託各呼叫一次（[帳務規格](accounting.md)第 12.4 節） |
| 2 | `on_day_start` | 08:30:00 | 當天的商品資料、帳戶的開盤前狀態已就緒；當天第一則行情之前 |
| 3 | `on_market` | `observed_time + latency.market_data` | 一則行情訊息送達；`ctx.market()` 已包含它 |
| 3 | `on_order_event` | 產生時刻 `+ latency.report` | 一筆回報送達；`ctx.account()` 已包含它 |
| 3 | `on_timer` | 設定的時刻 | 策略自己設的計時器 |
| 4 | `on_day_end` | 當天最後一個事件的時刻 | 日結完成之後；`ctx.account()` 是日結後的狀態 |

- 第 3 步的三種呼叫依[時間線](replay.md)的全序交錯。
- 策略會收到所選 feed、所選商品的每一則訊息，包括試算揭示與暫緩撮合；不想處理的自行略過。
- 當天沒有任何 feed 資料（`data.missing_days = "skip"`）的交易日仍會呼叫 `on_day_start` 與
  `on_day_end`，帳務的交割、公司行動與日結照常進行。
- 強制委託由帳務送出、不由策略決定；它的回報與策略自己的委託一樣經由 `on_order_event` 送達。

## 3. `Context`

| 方法 | 內容 |
| --- | --- |
| `now()` | 回放時鐘：這次呼叫所屬事件的時刻 |
| `date()` | 交易日 |
| `market()` | 行情視圖（第 4 節） |
| `account()` | 帳戶視圖（第 5 節） |
| `submit(order)` | 送出新委託，回傳 `OrderId`；`on_day_end` 中回傳 `None` |
| `cancel(id)`、`reduce(id, by)`、`reprice(id, price)` | 刪單、減量、改價（[撮合規格](matching.md)第 5.5 節） |
| `set_timer(time)` | 設定計時器，回傳 `TimerId` |

- 委託與改單在 `now() + latency.order` 到達交易所。`submit` 回傳的 `OrderId` 立刻可用，但委託在回報
  `Accepted` 送達之前還不存在於交易所；被拒絕時回報是 `Rejected`。
- `OrderId` 在整個 run 內唯一、依送出順序遞增；強制委託也佔用編號。
- 計時器的時刻不得早於 `now()`（`TimerError::Past`），必須在當天（`TimerError::AnotherDay`）。
  同一時刻的多個計時器依設定順序觸發。
- 委託與計時器不跨日。收盤後仍未成交的委託以 `Expired` 回報；`on_day_end` 中不能送單、不能設
  計時器（`TimerError::DayOver`）。
- 當天的行情結束、未成交委託失效之後才到達交易所的新委託，以 `Rejected`（`MarketClosed`）回報。
- 延遲為 0 時，策略在回報中立即重送被拒絕的委託會在同一時刻無限循環；策略必須自己避免。

### 3.1 委託

`NewOrder` 的欄位：

| 欄位 | 值 |
| --- | --- |
| `symbol` | 商品代號 |
| `board` | `Board::Regular`（整股）、`Board::OddLot`（盤中零股） |
| `side` | `Side::Buy`、`Side::Sell` |
| `kind` | `TradeKind::Cash`、`CashSellFirst`、`MarginBuy`、`MarginSell`、`ShortSell`、`ShortCover`（[帳務規格](accounting.md)第 4 節） |
| `price` | `OrderPrice::Limit(price)`、`OrderPrice::Market` |
| `time_in_force` | `TimeInForce::Rod`、`Ioc`、`Fok` |
| `qty` | 股數（`Shares`）。整股委託必須是交易單位的整數倍 |

數量的單位一律是股，不是張。價格是 `Price`（十進位定點數，可由文字 `"583.5"` 解析）。哪些組合被
接受見[撮合規格](matching.md)第 5 節。

### 3.2 回報

`OrderEvent { order, time, kind }`，`time` 是事件在交易所發生的時刻（不是送達策略的時刻）。

| `kind` | 意義 |
| --- | --- |
| `Accepted` | 新委託通過檢查，進入交易所 |
| `Rejected { request, reason }` | 新單、刪單、減量或改價被拒絕；既有的委託不變 |
| `Filled(fill)` | 成交 `fill.qty` 股於 `fill.price`；`fill.basis` 是成交依據（[撮合規格](matching.md)第 8 節） |
| `Cancelled { qty, reason }` | `qty` 股被取消，委託結束（刪單、IOC／FOK／市價的剩餘） |
| `Reduced { by, remaining }` | 減量完成 |
| `Repriced { price }` | 改價完成 |
| `Expired { qty }` | 收盤時仍未成交的 `qty` 股失效 |
| `PriceBandLimited` | 價格穩定措施使這筆委託少成交；每筆委託最多回報一次 |

## 4. 行情視圖

`ctx.market()` 由已送達策略的訊息建立，每個交易日重新開始。

| 方法 | 內容 |
| --- | --- |
| `book(symbol, board)` | 這個 book 的狀態；run 當天沒有回放它時是 `None` |
| `books()` | 當天回放的所有 book，依商品、盤別排序 |
| `instrument(symbol)` | 當天的商品資料：交易單位、參考價、漲跌停、撮合間隔等（[marketdata](marketdata.md)）|

`BookState` 提供：最新的五檔（`book()`）、最近一筆成交（`last_trade()`）、最新的試算結果
（`indicative()`）、交易模式（`mode(now)`：`Collecting`、`Continuous`、`Closed`，
[撮合規格](matching.md)第 4.2 節）、是否已開盤（`opened()`）與收盤（`closed()`）。

- 五檔是交易所揭示的五檔，不含策略自己的模擬委託。
- 沒有被回放的商品（不在 `data.symbols` 或不在所選 feed）沒有 book；對它送出的委託被拒絕。
- `instrument` 的欄位可能缺值（`Option`）；缺值的原因見 [marketdata](marketdata.md)。

## 5. 帳戶視圖

`ctx.account()` 是開盤前的帳戶狀態，加上已送達策略的回報。

| 方法 | 內容 |
| --- | --- |
| `cash()` | 已交割的現金。只在開盤前變動 |
| `available()` | 新委託還能動用的金額（[帳務規格](accounting.md)第 7.3 節） |
| `equity()` | 上一個交易日日結的權益；第一個交易日是期初現金 |
| `position(symbol)` | `Position { cash, margin, short }`：現股、融資、融券的股數，含當天已回報的成交 |
| `positions()` | 所有有部位的商品，依代號排序 |

- 每一筆回報送達時，`available()` 與該商品的 `position` 更新為「回報產生當下」的帳務狀態。兩筆
  回報之間，策略看不到交易所已發生但尚未回報的成交。
- `position.cash` 可以是負的：先賣後買尚未回補的股數。
- 開盤前的狀態已包含當天生效的公司行動與交割。
- `on_day_end` 時的帳戶視圖是日結後的狀態（當沖配對、資券相抵、利息、權益都已結算）。

## 6. 參數與註冊

策略的參數寫在設定檔的 `[strategy.params]`（[設定規格](config.md)第 12 節），以 `Params` 交給策略的
建構函式：

| 方法 | 型別 |
| --- | --- |
| `integer(name)`、`integer_or(name, default)` | 整數 |
| `boolean_or(name, default)` | 布林 |
| `text(name)` | 文字 |
| `price(name)` | 價格：文字 `"583.5"` 或整數元 |
| `symbol(name)`、`symbols(name)` | 商品代號、代號的列表 |
| `take(name)` | 任意型別的原始值 |

- 讀取一個參數會把它從 `Params` 移除。建構完成後還留著的參數就是策略不認得的參數，run 以錯誤
  結束；打錯的參數名稱不會被默默忽略。
- 帶小數的數值必須寫成文字，避免二進位浮點數。

程式以 `Registry` 登記它能執行的策略；每一項是 `StrategyEntry`：

| 欄位 | 內容 |
| --- | --- |
| `name` | 設定檔 `strategy.name` 用的名稱，程式內唯一 |
| `version` | 記錄在輸出中。策略的決策邏輯改變時必須更新 |
| `about` | 一行說明 |
| `build` | `fn(&mut Params) -> Result<Box<dyn Strategy>, ParamError>` |

`osmium` 執行檔登記的是 `osmium-examples` 的範例策略。自己的策略放在自己的 crate，建立 `Registry`、
登記後呼叫 `osmium_cli::main`，就得到同一組指令（[使用手冊](../guide/writing-a-strategy.md)）。

## 7. 已知限制

- 策略只能以 Rust 撰寫並與 runner 一起編譯。
- 一個 run 只有一個策略、一個帳戶。
- 策略不能在盤中入金、出金。
- 每一則訊息都呼叫 `on_market`；全市場回放時策略應盡早略過不關心的商品。
