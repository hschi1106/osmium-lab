# 寫策略

策略是一個實作 `Strategy` trait 的 Rust 型別，和 osmium 一起編譯成你自己的執行檔。這一頁用一個
完整的例子走一遍；精確的規則在[策略介面規格](../reference/strategy.md)。

例子的完整程式：[`crates/osmium-cli/examples/breakout.rs`](../../crates/osmium-cli/examples/breakout.rs)。
它會編譯、有測試，可以直接複製來改。

```sh
cargo run --release -p osmium-cli --example breakout -- strategies
```

## 1. 策略能知道什麼、能做什麼

回測的重點是不讓策略知道它在真實世界不可能知道的事。osmium 的保證：

- 策略只在被呼叫時行動，呼叫的原因只有三種：一則行情送達、一筆回報送達、自己設的計時器到期
  （加上每天開始與結束各一次）。
- 行情在交易所發出之後 `latency.market_data` 才送達策略；委託在送出之後 `latency.order` 才到達
  交易所；回報在產生之後 `latency.report` 才送回策略。
- 策略透過 `ctx`（`Context`）看行情與帳戶、送委託、設計時器。除此之外它不應該讀任何東西：不讀檔案、
  不讀系統時間、不用亂數。相同的輸入要做出相同的決定，回測才可重現。

委託怎麼成交由 osmium 依真實行情決定（[撮合規格](../reference/matching.md)），重點：

- 你的委託**不影響行情**：真實的五檔與成交照資料重播。所以它只適合相對於市場成交量很小的委託。
- 吃單（可立即成交的委託）：依到達交易所當下揭示的五檔逐檔成交，最多吃到看得見的量。
- 掛單：排在當時該價位揭示量的後面。之後該價位有真實成交時先扣掉排在前面的量，輪到你才成交；
  該價位的量減少而沒有成交（刪單）時，一律假設刪的是排在你後面的單。
- 集合競價（開盤、收盤、盤中零股、分盤）：你的委託價優於成交價才保證成交；與成交價同價時預設
  不成交（`matching.auction_marginal`）。
- 所有看不到的部分都取保守值：寧可少成交。

## 2. 骨架

```rust
use osmium_strategy::{Context, ParamError, Params, Registry, Strategy, StrategyEntry, TimerId};
use osmium_types::{Message, OrderEvent, OrderId, Price, Symbol};

struct Breakout {
    symbol: Symbol,
    trigger: Price,
    /// 今天的買單；一天只試一次。
    buy: Option<OrderId>,
}

impl Strategy for Breakout {
    fn on_day_start(&mut self, ctx: &mut Context<'_>) { /* 每天 08:30 */ }
    fn on_market(&mut self, ctx: &mut Context<'_>, message: &Message) { /* 一則行情送達 */ }
    fn on_order_event(&mut self, ctx: &mut Context<'_>, event: &OrderEvent) { /* 一筆回報送達 */ }
    fn on_timer(&mut self, ctx: &mut Context<'_>, timer: TimerId) { /* 計時器到期 */ }
    fn on_day_end(&mut self, ctx: &mut Context<'_>) { /* 日結之後 */ }
}
```

每個方法都有空的預設實作，只寫需要的。策略的狀態就是 struct 的欄位；整個 run 只建立一個實例，
跨交易日沿用。

**委託與計時器不跨日**：收盤時還沒成交的委託會失效（`Expired`），隔天要自己重下。所以和「今天的
委託」有關的狀態，要在 `on_day_start` 重設。

## 3. 每天開始：`on_day_start`

```rust
fn on_day_start(&mut self, ctx: &mut Context<'_>) {
    self.buy = None;
    // 收盤集合競價從 13:25 開始收單。
    ctx.set_timer(ctx.date().at_taipei(13, 25, 0))
        .expect("13:25 is later today");
}
```

08:30:00 呼叫，在當天第一則行情之前。此時：

- `ctx.market().instrument(symbol)` 是當天的商品資料：交易單位（`shares_per_lot`）、參考價、
  漲跌停價。**這些欄位都是 `Option`**，marketdata 沒有就是 `None`，要自己處理。
- `ctx.account()` 是開盤前的帳戶：昨天的成交、今天生效的交割與除權息都已反映。
- 這時送出的委託會參加開盤集合競價。想用開盤價買進，就以漲停價下限價單（見 `buy_and_hold`）。

## 4. 行情：`on_market`

```rust
fn on_market(&mut self, ctx: &mut Context<'_>, message: &Message) {
    // 所有被回放的 book 的每一則訊息都會進來：不關心的盡早離開。
    if self.buy.is_some()
        || message.symbol != self.symbol
        || message.feed.board() != Board::Regular
    {
        return;
    }
    let Some(state) = ctx.market().book(self.symbol, Board::Regular) else {
        return;
    };
    if state.mode(ctx.now()) != Mode::Continuous {
        return;
    }
    let broke_out = state
        .last_trade()
        .is_some_and(|trade| trade.price >= self.trigger);
    let lot = ctx
        .market()
        .instrument(self.symbol)
        .and_then(|instrument| instrument.shares_per_lot);
    let (true, Some(ask), Some(lot)) = (broke_out, state.book().ask().best(), lot) else {
        return;
    };
    // 吃賣一，吃不到就算了：IOC 沒有立即成交的部分會被取消。
    let order = self.order(Side::Buy, ask.price, TimeInForce::Ioc, i64::from(lot));
    self.buy = ctx.submit(order);
}
```

- `message` 是剛送達的那一則訊息；`ctx.market()` 已經包含它。多數策略不需要解讀 `message` 的
  內容，只用它判斷「是不是我關心的 book」，然後讀 `ctx.market().book(...)` 的狀態。
- `state.book()` 是最新揭示的五檔（`bid()`、`ask()`，各有 `best()` 與 `levels()`）；
  `state.last_trade()` 是最近一筆成交；`state.indicative()` 是最新的試算結果。五檔不含你自己的
  委託。
- `state.mode(ctx.now())` 是交易模式：`Collecting`（集合競價收單中：開盤前、收盤前 5 分鐘、暫緩
  撮合、零股與分盤交易）、`Continuous`（逐筆交易）、`Closed`。IOC、FOK 與市價單只在 `Continuous`
  被接受。
- 一檔熱門股票一天有數萬到數十萬則訊息，全市場有數千萬則。`on_market` 開頭的過濾決定了回測的
  速度。

## 5. 送委託

```rust
NewOrder {
    symbol: self.symbol,
    board: Board::Regular,            // 整股；盤中零股是 Board::OddLot
    side: Side::Buy,
    kind: TradeKind::Cash,            // 現股
    price: OrderPrice::Limit(price),  // 或 OrderPrice::Market
    time_in_force: TimeInForce::Ioc,  // Rod、Ioc、Fok
    qty: Shares::new(1_000),          // 股，不是張
}
```

- **數量的單位是股**。整股委託必須是交易單位的整數倍，所以用 `instrument.shares_per_lot` 換算，
  不要寫死 1000。零股委託是 1 到 `shares_per_lot − 1` 股。
- 價格必須符合升降單位、在漲跌停之內，否則被拒絕。
- `kind` 決定帳務怎麼處理：

  | `TradeKind` | 用途 |
  | --- | --- |
  | `Cash` | 現股買賣。賣出今天買進的股票就是當沖，日結時自動配對 |
  | `CashSellFirst` | 沒有庫存時先賣（當沖先賣後買），當天要買回 |
  | `MarginBuy`／`MarginSell` | 融資買進／賣出償還 |
  | `ShortSell`／`ShortCover` | 融券賣出／買進回補 |

- `ctx.submit` 立刻回傳 `OrderId`，但委託要 `latency.order` 之後才到交易所，到那時才知道被接受
  還是被拒絕。在 `on_day_end` 中 `submit` 回傳 `None`。
- 改單：`ctx.cancel(id)`、`ctx.reduce(id, by)`（減量，保留排隊位置）、`ctx.reprice(id, price)`
  （改價，重新排隊；零股不能改價）。它們同樣在 `latency.order` 之後才到交易所，結果以回報通知。

哪些委託會被拒絕、為什麼：[撮合規格](../reference/matching.md)第 5.2 節（交易所的檢查）與
[帳務規格](../reference/accounting.md)第 4 節（帳戶的檢查：資金、庫存、當沖與信用資格）。

## 6. 回報：`on_order_event`

```rust
fn on_order_event(&mut self, _ctx: &mut Context<'_>, event: &OrderEvent) {
    if let OrderEventKind::Rejected { request, reason } = event.kind {
        eprintln!("order {} {} rejected: {}", event.order.0, request.as_str(), reason.as_str());
    }
}
```

每一筆委託的一生：`Accepted` → 零次以上的 `Filled` → 以 `Filled`（全部成交）、`Cancelled`、
`Expired` 之一結束；或一開始就 `Rejected`。

| 回報 | 你該做什麼 |
| --- | --- |
| `Accepted` | 委託已在交易所 |
| `Filled(fill)` | 成交 `fill.qty` 股於 `fill.price`；一筆委託可以分多次成交 |
| `Cancelled { qty, .. }` | 刪單成功，或 IOC／FOK／市價單沒成交的部分 |
| `Expired { qty }` | 收盤了，剩下的失效 |
| `Rejected { request, reason }` | 新單被拒絕：這筆委託不存在。改單被拒絕：原委託不變 |
| `Reduced`、`Repriced` | 改單成功 |

收到回報時，`ctx.account()` 已經反映它：`position(symbol)` 的股數與 `available()` 的可用金額都是
這筆回報產生當下的值。所以多數策略不需要自己累計成交量，讀帳戶就好。

**注意延遲造成的時間差**：你決定下單時看到的五檔，到委託抵達交易所時可能已經變了；你收到成交
回報時，市場又往前走了 `latency.report`。兩筆回報之間，交易所可能已經發生你還不知道的成交。

**不要在回報裡無條件重送被拒絕的委託**：延遲設為 0 時會在同一時刻無限循環。

## 7. 計時器：`on_timer`

```rust
fn on_timer(&mut self, ctx: &mut Context<'_>, _timer: TimerId) {
    let Some(instrument) = ctx.market().instrument(self.symbol) else { return };
    let (Some(lot), Some(limit_down)) = (instrument.shares_per_lot, instrument.limit_down) else {
        return;
    };
    let lot = i64::from(lot);
    let shares = ctx.account().position(self.symbol).cash / lot * lot;
    if shares > 0 {
        // 以跌停價賣出：在收盤集合競價以收盤價成交。
        ctx.submit(self.order(Side::Sell, limit_down, TimeInForce::Rod, shares));
    }
}
```

`ctx.set_timer(time)` 設定當天稍後的一個時刻，回傳 `TimerId`；到期時呼叫 `on_timer`。時刻不能
早於 `ctx.now()`、不能在別的日子。需要「每 N 秒做一次」就在 `on_timer` 裡再設下一個。

常用時刻（台灣時間）：08:30 開始收單、09:00 開盤、13:25 起收盤集合競價收單、13:30 收盤。
`ctx.date().at_taipei(13, 25, 0)` 產生當天的時刻。

## 8. 參數

```rust
fn from_params(params: &mut Params) -> Result<Self, ParamError> {
    Ok(Self {
        symbol: params.symbol("symbol")?,
        trigger: params.price("trigger")?,
        buy: None,
    })
}
```

對應設定檔的：

```toml
[strategy]
name = "breakout"

[strategy.params]
symbol = "2330"
trigger = "2400"      # 價格與帶小數的數值寫成文字，例如 "583.5"
```

`Params` 有 `integer`、`integer_or`、`boolean_or`、`text`、`price`、`symbol`、`symbols`。缺少必要
的參數、型別不對、或設定檔寫了策略沒有讀取的參數，run 都會在開始前以錯誤結束——打錯字不會被
默默忽略。參數值的檢查（例如必須為正）自己做，回傳 `ParamError::new("lots", "must be positive")`。

**不要用浮點數**做價格與金額的運算。`Price` 是十進位定點數，`Shares` 與 `Money` 是整數。

## 9. 註冊，變成執行檔

```rust
const ENTRY: StrategyEntry = StrategyEntry {
    name: "breakout",
    version: "1",
    about: "Buys one board lot of `symbol` when it trades at `trigger` or higher, \
            and sells it at the close.",
    build: |params| Ok(Box::new(Breakout::from_params(params)?)),
};

fn main() -> std::process::ExitCode {
    let mut registry = Registry::new();
    registry.add(ENTRY);
    osmium_cli::main(&registry)
}
```

`osmium_cli::main` 提供和 `osmium` 一樣的指令（`run`、`check`、`strategies`、`version`）。
`version` 會記錄在每個 run 的 `run.json`：**策略的決策邏輯改了就要改它**，否則無法分辨兩份結果
是不是同一個策略跑出來的。

放在哪裡，兩種做法：

1. **自己的 crate**（建議）。在任何地方建立一個 binary crate，以路徑相依 osmium：

   ```toml
   [package]
   name = "my-strategies"
   version = "0.1.0"
   edition = "2024"

   [dependencies]
   osmium-book = { path = "/path/to/osmium-lab/crates/osmium-book" }
   osmium-cli = { path = "/path/to/osmium-lab/crates/osmium-cli" }
   osmium-strategy = { path = "/path/to/osmium-lab/crates/osmium-strategy" }
   osmium-types = { path = "/path/to/osmium-lab/crates/osmium-types" }
   ```

   ```sh
   cargo build --release
   ./target/release/my-strategies run run.toml --out ~/osmium-runs/breakout-1
   ```

   一個執行檔可以登記很多個策略（`registry.add` 多次），設定檔的 `strategy.name` 選擇要跑哪一個。

2. **放在這個 repository 的 `examples/`**，像 `breakout.rs` 一樣，以
   `cargo run --release -p osmium-cli --example <name> -- run ...` 執行。適合快速試驗。

## 10. 測試策略

用 `osmium-fixture` 的樣本市場（兩個交易日、一檔商品、每一則訊息都寫在程式裡）跑整個 run，再檢查
輸出。`breakout.rs` 最後的測試就是這樣做的：

```rust
let tree = sample::tree("cli-breakout");
// 寫設定檔，然後：
let status = osmium_cli::execute(&registry(), &arguments, &mut out, &mut err);
assert_eq!(status, 0);
let events = fs::read_to_string(results.join("order_events.csv")).unwrap();
```

樣本市場的內容在 [`crates/osmium-fixture/src/sample.rs`](../../crates/osmium-fixture/src/sample.rs)。
需要別的情境時，用 `osmium_fixture::Tree` 寫自己的資料
（[`crates/osmium-runner/tests/run.rs`](../../crates/osmium-runner/tests/run.rs) 有例子）。

然後才用真實資料：先選一兩檔商品、幾天，讀 `order_events.csv` 確認每一筆成交都是你預期的，再放大
範圍。

## 11. 常見的錯誤

| 現象 | 原因 |
| --- | --- |
| 策略完全沒下單 | `instrument.shares_per_lot` 或漲跌停是 `None` 而提早 return；商品不在 `data.symbols` 或所選 feed，`book()` 是 `None` |
| 委託都被 `InvalidQuantity` 拒絕 | 數量用了張而不是股，或不是交易單位的整數倍 |
| `OrderTypeNotAllowed` | 在集合競價期間（開盤前、13:25 之後、零股）送了 IOC／FOK／市價單 |
| `InsufficientPosition` | 賣出時回報還沒送達，帳戶還沒有庫存；或賣的是今天買的、但商品或帳戶不能當沖 |
| 隔天的委託不見了 | 委託不跨日；在 `on_day_start` 重下 |
| 掛單很少成交 | 這是排隊估計的結果：前面的量要先被真實成交消耗。見[撮合規格](../reference/matching.md)第 6.3、6.4 節 |
| 兩次 run 結果不同 | 策略用了 `HashMap` 的走訪順序、系統時間或亂數。`ctx.market().books()` 與 `ctx.account().positions()` 的順序是固定的 |

更多見[疑難排解](troubleshooting.md)。
