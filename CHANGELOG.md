# Changelog

## Unreleased

- 改建為 marketdata-native 架構（[ADR-0007](docs/design/decisions/0007-marketdata-native.md)）。
  移除 Teralion provider、`data-sync`（source sync／verify／replay cache）、舊 CLI 與 config loader、
  synthetic Teralion fixtures、acceptance 與 release tooling，以及 AGPL／Strategy Linking Exception。
  `run-planner` 只保留 effective config、session 與 canonical encoding，資料根目錄改為 `md_root`。
  舊版本歷史保留於 Git。
- `Symbol`／`SourceFormatId` 改為最多 23 bytes 的 inline `Copy` 型別；`InstrumentId` 因此為 `Copy`。
- `DomainEvent` 新增 `observed_time`；ordering rule v5 依 `observed_time`、market、source format、
  source sequence 排序，replay clock 改為 observed time。移除 Teralion `STOCK_REALTIME` phase rank。
- 新增 `osmium-md`：讀取 marketdata F6／F17／F23 parquet，檢查 schema 版本與欄位型別、驗證
  manifest identity，計算全 feed observed time，並解碼成依 `seq` 排序的訊息（揭示、中間成交、
  試撮、暫緩撮合），數量一律換算成股數（見 [marketdata 介面](docs/reference/marketdata.md)）。
- 新增 `osmium-replay`：依 `observed_time`、feed rank、`seq` 合併多個 feed（`MergedFeeds`），以及把
  行情、委託到達、回報、行情送達策略、計時器排成單一時間線的 `Timeline`
  （[回放規格](docs/reference/replay.md)第 2、3 節）。
- 新增 `osmium-book`：每本 book 的撮合方式（逐筆／分盤，含 `match_interval_s` 缺值時的推論）、
  交易模式、目前五檔、最近成交、試撮與暫緩撮合狀態（[撮合規格](docs/reference/matching.md)第 4 節）。
- 新增 `osmium-types`：整數的 `Price`（1e-4 元）、`Shares`、`Symbol`、`Timestamp`／`Date`、
  `Feed`，以及行情訊息與商品基本資料的型別
  （[ADR-0008](docs/design/decisions/0008-full-market-matching-simulation.md)）。
- 移除 Teralion raw status／limit bit annotations（`TwseQuoteAnnotations` 等）；`MarketAnnotations`
  只保留 `None` 作為日後中性 annotation 的 canonical 欄位。
- 移除內建 Teralion market-state profiles（`twse_regular`、`taifex_futures` 等），改以
  `MarketStateProfile::accepting_all` 建立；marketdata feed profile 由 `Feed::state_profile` 提供。
