# Changelog

## Unreleased

全市場撮合模擬的改建（[ADR-0007](docs/design/decisions/0007-marketdata-native.md)、
[ADR-0008](docs/design/decisions/0008-full-market-matching-simulation.md)）。所有版本號重新從 1 起算。

- 移除 Teralion provider、`data-sync`（source sync／verify／replay cache）、synthetic Teralion
  fixtures、acceptance 與 release tooling，以及 AGPL／Strategy Linking Exception。舊版本歷史保留於
  Git。
- 移除舊架構的 crate：`market-types`、`market-state`、`replay-engine`、`strategy-api`、
  `execution-sim`、`run-planner`、`example-strategy`，以及它們的文件（`docs/design/legacy/`）。
- 新增 `osmium-types`：整數的 `Price`（1e-4 元）、`Shares`、`Money`、`PerShare`、`Symbol`、
  `Timestamp`／`Date`、`Feed`，行情訊息、商品基本資料、參考資料，以及委託與委託事件的型別
  （`NewOrder`、`OrderEvent`、`Fill`、`RejectReason` 等）。
- 新增 `osmium-md`：讀取 marketdata F6／F17／F23 parquet，檢查 schema 版本與欄位型別、驗證
  manifest identity，計算全 feed observed time，並解碼成依 `seq` 排序的訊息（揭示、中間成交、
  試撮、暫緩撮合），數量一律換算成股數；讀取參考表 `qc/calendar`、`corp_actions`、`events`、
  `daily`、`daily_ext`（[marketdata 介面](docs/reference/marketdata.md)）。
- 新增 `osmium-replay`：依 `observed_time`、feed rank、`seq` 合併多個 feed（`MergedFeeds`），以及把
  行情、委託到達、回報、行情送達策略、計時器排成單一時間線的 `Timeline`
  （[回放規格](docs/reference/replay.md)）。
- 新增 `osmium-book`：每本 book 的撮合方式（逐筆／分盤，含 `match_interval_s` 缺值時的推論）、
  交易模式、目前五檔、最近成交、試撮與暫緩撮合狀態（[撮合規格](docs/reference/matching.md)第 4 節）。
- 新增 `osmium-matching`：模擬委託對真實行情撮合。交易所端的委託檢查、逐筆交易（到達時吃五檔、
  排隊估計、被動成交、交叉成交）、集合競價（開盤、收盤、分盤、盤中零股、暫緩撮合結束）、瞬間
  價格穩定措施的 ±3.5 % 範圍、改單與刪單（[撮合規格](docs/reference/matching.md)第 5–8 節）。
- 新增 `osmium-ledger`：整數元的精確帳務。手續費與證交稅、T+2 交割與待交割款、資金檢查與掛單
  保留、現股當沖、融資融券（利息、期限、資券相抵、維持率、追繳與強制委託）、公司行動（現金與
  股票股利、二代健保補充保費、分割、面額變更、反分割、減資）、逐筆分錄與已實現損益
  （[帳務規格](docs/reference/accounting.md)）。
- 新增 `osmium-strategy`：策略 trait、`Context`（委託、改單、刪單、計時器）、行情與帳戶視圖、
  具型別的參數、策略註冊表（[策略介面規格](docs/reference/strategy.md)）。
- 新增 `osmium-runner`：交易日流程（前一日事實、開盤前帳務、強制委託、行情／委託／回報延遲、
  收盤失效、日結）、TOML 設定（[設定規格](docs/reference/config.md)）與輸出
  （`run.json`、`orders.csv`、`order_events.csv`、`entries.csv`、`realized.csv`、`daily.csv`、
  `positions.csv`、`diagnostics.csv`；[輸出規格](docs/reference/artifacts.md)）。相同的資料、設定與
  版本產生位元組相同的輸出。
- 新增 `osmium-cli`：`osmium run`、`check`、`strategies`、`version`；同時是 library，自己的策略
  以 `osmium_cli::main(&registry)` 建立自己的執行檔。
- 新增 `osmium-examples`：範例策略 `buy_and_hold`、`open_to_close`、`passive_quote`。
- 新增 `osmium-fixture`（只供測試）：寫出合成的 marketdata 樹。
- 結果依賴的版本與更新規則見[版本](docs/reference/versions.md)。
- `DECODER_VERSION` 2：marketdata 在來源端修正了 2022-01-03 → 2024-03-29 `TWSE_F6` 的市價委託與中間
  成交形狀，`osmium-md` 移除對舊形狀的辨識。揭示五檔內沒有價格的檔位一律是錯誤，中間成交只來自
  沒有 `books` 列的 `trades` 列；`seq` 缺口旁有 4 列的分類與之前不同
  （[marketdata 介面](docs/reference/marketdata.md)第 6 節）。
- `DECODER_VERSION` 3、`osmium_ledger::RULES_VERSION` 2：當沖資格改讀 marketdata 的 `daytrade`
  （`BOTH`、`BUY_FIRST`、`NONE`），沒有 `F1` 的日子也採用它，不再近似；只有 null 時才用近似
  （[帳務規格](docs/reference/accounting.md) 8.1）。
- `DECODER_VERSION` 4、`osmium_ledger::RULES_VERSION` 3：公司行動改讀 marketdata 的 `after_price`、
  `pay_date`、`stock_per_1000`、`shares_old`／`shares_new`、`refund_per_share`，不再讀 FinMind 的
  `stock_dividend`。現金股利在 `pay_date` 入帳；配股股數 = `floor(股數 × stock_per_1000 ÷ 1000)`，
  面額只影響畸零；減資、分割、反分割、面額變更的比例與退款取自資料，不再需要補充資料或近似；
  除權息日與現金增資日的評價價格改用 `after_price`（[帳務規格](docs/reference/accounting.md)第 13 節）。
- `DECODER_VERSION` 5、`osmium_ledger::RULES_VERSION` 4：讀取 `events.disp_nth`。融資停止與融券
  保證金加成只適用於第二次以上的處置；TWSE 的第一次處置不再受限，TPEx 沒有次數，仍取較嚴者
  （[帳務規格](docs/reference/accounting.md) 9.1）。
- `DECODER_VERSION` 6：交易日與交割日改讀 marketdata 的 `ref/calendar`（`open`、`settle`），不再讀
  `qc/calendar`，`osmium-ledger` 也不再內建只辦理交割的日子。日曆涵蓋資料之後的日子，因此移除
  `calendar.extra_settlement_days`；日期範圍超過資料的最後一天時，之後的交易日是缺少資料的日子
  （[帳務規格](docs/reference/accounting.md) 7.1）。
- `OUTPUT_VERSION` 2：參考表以 marketdata `fm/_build` 的列識別。讀取前核對列數與 `md.git_commit`、
  讀取後確認那一列沒有改變，run 第一次讀到一個檔案時比對 `file_sha256`；`run.json` 的 `reference`
  多了 `content_hash`、`file_sha256`（[marketdata 介面](docs/reference/marketdata.md)第 9 節）。
- `osmium_ledger::RULES_VERSION` 5：減資列的 `cash_dividend`（減資期間同時發放的現金股利）以減資前的
  股數入帳，並計入二代健保補充保費的給付。2323（2022-10-24、2026-10-12）、3356（2024-11-11）的部位
  價值因此與前收一致（[帳務規格](docs/reference/accounting.md)第 13 節）。
- 新增使用手冊（`docs/guide/`）：安裝與第一個回測、寫策略、執行、讀結果、疑難排解；以及完整的
  自訂策略範例程式 `crates/osmium-cli/examples/breakout.rs`。
- 新增 agent skill `osmium-backtest`：使用系統跑回測的流程。
- 效能（輸出位元組相同，不更新任何版本）：`osmium-md` 平行解碼 row group、`books` 列以差分保存、
  以 `seq` 為索引的順序表取代排序；`Timeline::next_event` 借出訊息（`Event<'_, E>` 持有
  `&Message`），訊息只複製一次；`osmium-book` 新增 `BookMap`。全市場四個 feed 的一個交易日由
  17.5 秒／3.6 GB 降到 5.6 秒／0.7 GB（[效能](docs/design/performance.md)）。
