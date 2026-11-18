# 疑難排解

錯誤訊息都印在 stderr，以 `error:` 開頭，exit status 是 1（指令列看不懂是 2）。這一頁依訊息列出
原因與做法。

## 1. 設定檔

| 訊息 | 原因與做法 |
| --- | --- |
| `<file>: No such file or directory` | 設定檔的路徑不對 |
| `TOML parse error ... unknown field ...` | key 打錯，或放錯 section。訊息會列出那個 section 認得的 key。osmium 不會忽略不認得的 key |
| `invalid type: floating point ... expected a whole number, or a decimal number written as text` | 帶小數的值要寫成文字：`discount = "0.28"`，不是 `discount = 0.28` |
| `strategy.name: ... is not a strategy of this program (it has: ...)` | 策略名稱打錯，或你執行的是 `osmium` 而不是登記了那個策略的執行檔 |
| `strategy parameter ... is missing` | `[strategy.params]` 少了必要的參數 |
| `strategy parameter ... is not a parameter of this strategy` | `[strategy.params]` 寫了策略不讀取的參數（多半是打錯字）|
| `strategy parameter ... must be ...` | 參數的型別不對；價格與代號要寫成文字 |
| `no trading day from ... to ...` | 日期範圍內沒有交易日 |

改過設定就先跑 `osmium check <config>`：它做完所有不需要回放的檢查。

## 2. 資料

| 訊息 | 原因與做法 |
| --- | --- |
| `osmium check` 的 `missing` 列出日期與 feed | 那些日子沒有那個 feed 的資料。縮小日期範圍、拿掉那個 feed，或設 `data.missing_days = "skip"` 跳過 |
| `marketdata: ... instruments has no shares_per_lot` | 那一天的商品資料沒有交易單位（2024-03-29 以前、2026-10-13 起）。設 `data.assume_lot_size = 1000` |
| `<date>: marketdata daily/... has no row for this day` | 那一天（或 run 第一天的前一個交易日）沒有盤後資料，無法評價。調整日期範圍 |
| `marketdata: ... schema ...`、`... manifest ...` | marketdata 的檔案與 osmium 支援的 schema 版本不符，或檔案與 manifest 不一致。確認 `md_root` 指向 schema 版本 1 的資料（`osmium version`），並回報給 marketdata 的維護者 |
| `the output directory ... is not empty` | 輸出目錄已有東西。osmium 不覆寫結果；換一個目錄 |

osmium 只讀取 `md_root`，不會修改它。

## 3. 帳務

| 訊息 | 原因與做法 |
| --- | --- |
| `accounting on <date>: the calendar has no settlement day far enough after ...` | run 跑到資料的最後兩個交易日，交割日不在日曆裡。在 `calendar.extra_settlement_days` 補上之後的交易日（[執行](running.md#5-公司行動與日曆)）|
| `accounting on <date>: <symbol> <action> on <ex_date>: marketdata does not have ...` | 持有的商品在那一天除權息或減資，而 marketdata 沒有需要的事實（發放日、換股比例、退還股款、股利金額等）。以 `[[corporate_actions.overrides]]` 補上，或設 `corporate_actions.missing_terms = "approximate"` |
| `... shares of <symbol> sold without inventory were not bought back` | 策略以 `CashSellFirst` 賣出、當天沒有買回。修正策略，或設 `daytrade.uncovered = "force_cover"` 讓帳務隔天強制買回 |
| `cash is ... after settlement on <date>` | `account.cash_check = "at_settlement"` 而交割日的現金不夠。增加期初現金或改回 `"at_order"` |
| `a forced order for ... shares of <symbol> could not be filled` | 被追繳、到期或未回補的部位需要賣出／回補，但那檔商品沒有被回放（不在 `data.symbols`、所選 feed）或連續數日沒有成交。把持有的商品都列進 `data.symbols` |
| `a short position in <symbol> is still open on the ex-date ...` | 融券部位跨過股票股利、分割或減資的除權日；不支援。策略要在那之前回補 |
| `no price to value <symbol> with on <date>` | 持有的商品從來沒有過收盤價。確認商品代號與日期 |

## 4. run 成功了，但結果不如預期

**完全沒有委託（`orders=0`）。** 策略沒有呼叫 `submit`。最常見的原因：

- 商品不在 `data.symbols` 或所選的 feed 裡，`ctx.market().book(...)` 是 `None`。
- `instrument.shares_per_lot`、`limit_up`、`limit_down` 是 `None`，策略提早 return。
- 策略等待的條件沒有發生（例如模式不是 `Continuous`）。

在策略裡暫時加 `eprintln!` 是最快的查法；stderr 不屬於輸出，不影響結果。

**有委託但沒有成交（`fills=0`）。** 看 `order_events.csv`：

- 都是 `Rejected`：看 `reason`（[讀結果](results.md#3-每一筆委託發生了什麼orderscsvorder_eventscsv)）。
- 都是 `Accepted` 然後 `Expired`：限價單沒有被觸及。掛單要等排在前面的量被真實成交消耗完；
  集合競價中與成交價同價的委託預設不成交。
- `Cancelled`（`IocRemainder`、`FokUnfilled`）：委託到達時（`latency.order` 之後）對手方已經沒有
  那個價格的量。

**`InsufficientCash`。** 預設的 `cash_check = "at_order"` 要求每一筆買單在它的交割日有足夠的現金，
而賣出的款項要兩個交易日後才交割。每天買賣的策略因此需要多於一天交易額的現金；當沖策略改用
`"at_settlement"`。

**賣不掉今天買的（`InsufficientPosition`）。** 兩種可能：買進的成交回報還沒送達就送出賣單（等
`on_order_event`）；或這檔商品、這個帳戶當天不能當沖（[帳務規格](../reference/accounting.md)第 8
節），今天買的要隔天才能賣。

**權益和自己算的不一樣。** `cash` 只含已交割的金額，損益在交割日（T+2）才進 `cash`；績效看
`equity`。逐筆的金額在 `entries.csv`，可以加總驗算。

**同一個設定跑兩次結果不同。** osmium 保證相同輸入產生相同輸出，所以先檢查策略：有沒有用
`HashMap`／`HashSet` 的走訪順序決定下單順序、有沒有讀系統時間或用亂數、有沒有讀外部檔案。比較兩個
`run.json`（[讀結果](results.md#7-比較兩個-run)）確認輸入真的相同。策略沒有問題的話，這是 osmium
的 bug。

**run 很慢或記憶體不夠。** 見[執行](running.md#6-時間與記憶體)：縮小 `data.symbols`、不要選不需要
的 feed（尤其 `F17`）、確認是 `--release` 編譯、檢查 `on_market` 有沒有盡早略過不關心的訊息。

## 5. 還是不對

結果與交易所的規則或真實對帳單不符時，先讀對應的規格，確認是不是已知的限制：

- 成交：[撮合規格](../reference/matching.md)第 10 節
- 金額：[帳務規格](../reference/accounting.md)第 16 節
- 資料：[marketdata 介面](../reference/marketdata.md)第 10、11 節

回報問題時附上整個輸出目錄（含 `run.json` 與 `config.toml`）與 `osmium version` 的輸出：它們足以
重現那個 run。
