# Changelog

## Unreleased

- CLI 透過 `SourceAdapterRuntime` 依 SourceId 同步資料；provider queries、credential 與 staging
  orchestration 移至 adapter boundary。TAIFEX normalizer 保留各 session 的離散時間窗。
- `data verify`／`cache prepare` 共同檢查 partition descriptor，錯誤帶出商品、日期與 reference mismatch。

- cache identity 同時 hash normalizer mapping name 與 version，避免不同 adapter 使用同版號時碰撞；
  cache format 升為 v3，使舊 identity 的 cache 明確失效並由 verified source 重建。

- scheduled execution 以每筆可見 observation 更新交易資格，與 firm depth timestamp／已消耗量分離；
  status-only pause 不再沿用暫緩前資格。Slippage 後依商品 price policy 整批驗證成交價。
- volatility interruption 僅取消同商品的未完成 market ROD；`MarketStatus` 不當成 firm resume
  evidence，不清除既有 indicative state。
- run manifest 升為 v4，三種 publisher 統一保存 runtime version set，包含 execution／fill model
  與 domain／replay／strategy API 版本。

- 將 opening、closing 與盤中 stability 試算統一為 typed `IndicativeAuction`；
  所有 trial observation 與 firm book／trade／volume 分流，舊 event/cache API 不相容。
- MarketState 於確認 matching 恢復的 firm event 或 carry session boundary 清除過期 indicative state；
  清除保留 event provenance、不改動 firm observations，market-state/reducer versions 升為 5/4。
- `MatchTime` formatting 與 canonical event timestamp、`TradingDate` epoch constructor及 canonical vector decoding新增邊界驗證；
  `market-types` 先升為 v5，使舊 cache identity 失效；`Decimal` 提供不經浮點數的 exact text formatting。
- TWSE 真實 stability-day partition 揭露第一檔零價的市價委託聚合量；`BookSide` 新增獨立
  `market_order_quantity`，不再將它誤當零價 `BookLevel` 或 execution price evidence。event schema、
  canonical event、market-types 現升為 v6/v6/v7，TWSE equity/warrant mapping 升為 v9/v6、
  TPEx 升為 v6/v6；舊 cache 必須由 verified source 重建。
- 新增 provider-neutral `MarketStatus` event，保存只有 cumulative volume 與 typed annotations 的狀態
  observation；TWSE／TPEx 暫緩撮合的空簿 sentinel 不再以空 `QuoteSnapshot` 清除最後 firm book／trade。
- 移除 `InstrumentClass::Unknown` 與依 market／symbol 推定商品類型或 TAIFEX futures session 的 fallback；
  RunConfig 升為 v3，明確要求 `instrument_class`，TAIFEX future 明確要求 `contract_shape` 與
  `session_profile`；option/warrant reference 欄位納入 EffectiveRunConfig v6 canonical identity；source identity
  隨後納入 v7 identity，partition/cache 路徑與 cache factory 不再自行假設 Teralion；
  signed／zero spread price policy 傳入撮合與帳務驗證。
- `TradePrint`／`TradePrintKind`／`TradeOrder` 改名為 `ObservedTrade`／`TradeObservationKind`／
  `TradeBatchOrdering`；canonical discriminants 保持不變，舊 Rust API 不保留相容別名。
- Replay merge 重用 ordering key 的 canonical frame 計算 event checksum，並快取 TAIFEX sort key；
  加入固定 synthetic event／ReplayCore microbenchmark，以及 verified-source/cache/replay pipeline benchmark；
  驗證 event 與 final-state checksum 不變。Pipeline 基準使用合成行情，不代表完整策略回測速度或真實行情表現。
- `ReplayStateViews` 改為借用 replay core 的 deterministic state map，移除一般與 scheduled strategy callback
  每筆 event 配置 market-state view 向量；此為 strategy API v3 的破壞性 API migration，不保留 slice adapter。
  新增 full multi-backtest benchmark，確認 strategy output、fills 與 replay checksums 可重現。
- TWSE／TPEx stability 暫緩撮合期間限制新委託為 limit ROD，並取消既有已送出的未完成 market ROD；一般與 scheduled execution 均驗證此規則。新增 `VolatilityInterruption` cancellation reason，strategy API 升為 v4、execution simulator 升為 v3。此修改只落實已觀察到的 order-entry 規則，不推算 stability lifecycle。
- TWSE／TPEx 盤中 `trial` 統一輸出 `IntradayUnclassified`，保留方向 annotations 供 Alpha 關聯明確 trigger。真實 `/ticks` stability samples 確認兩市場 zero-quantity／empty-book trigger 形狀；normalizer 將這類 status-only observation 映射為 `MarketStatus`，其他 zero-quantity shape 拒絕。TWSE 同一 `match_time` 多筆 intermediate trades + final trigger 會合併成保序 `TradeBatch` 加 final `MarketStatus`。原始 source rows 未納入 fixtures；Teralion adapter certification 已以外部 immutable partitions 完成，Alpha differential 仍待完成。
- Derived cache 升為 format v2，單筆 canonical event record 限制 16 MiB；reader 在配置 record buffer 前拒絕超限長度，writer 同步拒絕產生無法讀回的 cache。
- cache codec 不再列舉內建 provider normalizer；source adapter 提供預期 mapping identity，catalog 將舊 schema／mapping cache 標為 stale 並由 verified source 重建。通用 partition integrity／stability lifecycle 驗證與 Teralion wire conformance 已拆開。

## 0.1.0 — public release preparation

- 建立 neutral `osmium-config` 與 `osmium-runner` production boundary。
- 移除 M1/M2/M3 milestone crates 與 legacy config v1 parser。
- 收斂 CLI 至 `config check`、`data sync/verify`、cache、replay、backtest、display、run
  與 inspect workflow。
- 將 fixture builder、歷史 runner、acquisition helpers 與 formal scripts 移到 tools。
- 加入 binary archive packaging、neutral example config、quickstart、config reference、
  local data layout 與 fixture manifest。
- 完成 RLS-06 JSON output/quiet/no-color 與 stable exit categories。
- 加入 synthetic smoke fixture、fixture bundle package/fetch/verify flow，以及
  deterministic archive、offline installer、clean-machine/reproducibility checks。
- archive 現在包含 CycloneDX SBOM 與 transitive third-party license inventory；新增
  [support policy](docs/operations/support.md)。
- 移除 historical increment／verification／milestone config 與舊 acceptance tooling；完整
  historical evidence 改由 Git history 或 external archive 保存。
- 保留 `examples/config.yaml` 原樣，並以 repository-owned synthetic scenarios 取代
  committed Teralion market data；新增可重建 fixture 的 deterministic generator。
