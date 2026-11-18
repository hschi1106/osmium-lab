# osmium-lab 產品需求

## 1. 產品定位

`osmium-lab` 是以 Rust 建立的台灣市場回播與回測平台，唯一資料源是 marketdata（預設
`/srv/md/v1`，唯讀）。資料邊界由 `osmium-md` 擁有；planner、domain event、replay、strategy、
simulation 與通用驗收契約不得依賴 marketdata 的欄位或檔案格式。系統依交易所時間重播可觀察的
成交與行情快照，讓策略在沒有未來資料的前提下執行，並以明確、可版本化的模型估算成交與帳務結果。

產品原則依序為：

1. 正確性優先：資料與 manifest 不一致、或無法證明語義的組合一律拒絕，不猜測。
2. 相同資料 identity、版本與設定產生相同結果。
3. 只讀取 universe 需要的資料，不建立第二份 source 或 cache。
4. 模型不宣稱來源資料無法支持的撮合或排隊精度。
5. 市場、商品、board、session、五檔與成交的差異保留在明確的介面契約中。

## 2. 支援範圍

資料可回播至 MarketState 的範圍（見 [marketdata 介面](../../reference/marketdata.md)）：

- TWSE／TPEx 一般 board 股票與 ETF（Format 6）。
- TWSE／TPEx 權證（Format 17）。
- TWSE／TPEx 盤中零股（Format 23），數量單位為股。
- 處置證券以一般商品方式回播；不重算處置撮合規則。

TAIFEX 期貨與選擇權的 MarketState、execution 與 accounting model 已存在，但 marketdata 目前沒有
TAIFEX 逐筆資料，因此尚無資料源。

不支援的範圍：

- 盤後零股、盤後定價與鉅額交易。
- 即時交易或自動下單。
- 完整交易所撮合、逐筆委託簿、真實 queue position 或 hidden liquidity 推論。
- 瞬間價格穩定措施、處置撮合規則或其他交易所內部狀態的重建。
- 從低粒度資料合成不存在的高粒度資料。
- 回播或回測期間存取網路。

## 3. 資料能力與限制

marketdata 能證明的內容是回播能力的上限：

- books：每則訊息的最佳五檔（`n_bid`／`n_ask` 為 null 表示未揭示）、phase、試撮旗標、暫緩撮合
  狀態與方向、開收盤延後旗標。
- trades：正式成交與中間成交（僅連續交易），以 `seq` 與 books 對應。
- instruments：商品類別、每張股數與撮合間隔；權證目前不在 instruments 中。
- 每個 feed-day 都有 manifest row（path、rows、`schema_version`、`git_commit`、`code_hash`、
  `inputs_hash`、`partial`、`seq_gaps`、`seq_dup`）作為資料 identity。
- `exch_ts` 是交易所時間；`match_time` 取 `exch_ts`，replay clock 取 feed 內 `exch_ts` 的
  running max（`observed_time`）。
- 一般與零股 book 目前共用同一個 `InstrumentId`；漲跌停狀態尚未轉成中性欄位。
- 每個執行 universe 的 instrument class 必須由設定明確指定；TAIFEX futures 另須明確指定
  contract shape 與 session profile，不依 symbol 推定。contract identity 必須進入 effective
  config checksum。

marketdata 欄位與 domain event 必須分離。未知或未觀察過的語義組合需明確拒絕。來源欄位名稱與
part 名稱以外的格式細節不得洩漏到 strategy 或通用 replay contract。

## 4. 使用流程

```text
RunConfig
  -> execution plan
  -> osmium-md：manifest 驗證、parquet 讀取、mapping
  -> deterministic replay
  -> strategy callbacks
  -> execution simulation / accounting
  -> immutable run artifacts
```

回播與回測不需要網路或 credential。以 `osmium-md` 串接 planner 與 runner 的入口尚在重建中。

## 5. 資料需求

### DATA-01 資料根目錄

- `md_root` 可設定，預設 `/srv/md/v1`；osmium 只讀，不寫入、不搬移、不快取 marketdata。
- `md_root` 不進入 semantic checksum；資料 identity 由 manifest row 表達。

### DATA-02 Identity 與一致性

- 讀取前，檔案的 path、rows 與 footer `md.schema_version`、`md.git_commit` 必須與 manifest row
  一致。
- 讀取後重新讀 manifest；任一使用中的 row 改變即失敗，不發布混合版本的結果。
- 當天沒有 manifest 或沒有該 table／part 的 row，代表沒有資料，不得視為空日。

### DATA-03 品質

- `partial`、`seq_gaps`、`seq_dup` 與 `qc` 表是品質證據；strict 執行的 qc gate 尚在重建中。
- row 層級違反不變條件（價格不是精確 1e-4 decimal、數量非正、揭示檔數不符、同 symbol `seq` 不遞增、
  `exch_ts` 倒退）一律拒絕。

### DATA-04 選擇性讀取

- 只解碼需要的欄位，並以 `symbol` row-group statistics 剪枝。
- observed time 需掃描整個 feed 的 `seq` 與 `exch_ts`，因此與 universe 選擇無關。

### DATA-05 商品資料

- 影響帳務的 quantity unit、multiplier 與 currency 必須由已驗證 metadata 或明確設定提供，缺值
  不得自行推定。
- TAIFEX 跨日資料依 exchange trading date 與 session plan 歸屬，不以本地日曆日期切割。

## 6. 回播需求

### REPLAY-01 標準事件

domain event 集合為：

- `QuoteSnapshot`：完整五檔，以及同一 source observation 的成交、累計量與 annotations。
- `BookSnapshot`：完整五檔與 annotations。
- `TradeBatch`：同一 source observation 的一筆或多筆成交與可用累計量。
- `MarketStatus`：只有累計量與 provider-neutral market signal 的狀態 observation；不宣稱
  提供正式成交或完整 firm book。
- `IndicativeAuction`：以 `AuctionPurpose::{Opening, Closing, Periodic,
  VolatilityInterruption}` 與 partial `AuctionObservation` 表達的試算資訊，不是實際成交。
  `purpose`、`delayed`、`disposal` 與 volatility direction 各自可為 `Known`、
  `NoObservation` 或 `Unknown`；缺少 evidence 不得補成 `Periodic` 或 `false`。

事件的 market signal 使用單一 provider-neutral taxonomy：`Continuous`、
`AuctionCollecting(AuctionObservation)`、`AuctionUncross(AuctionObservation)` 與 `Closed`。
marketdata 的 phase、試撮與暫緩撮合欄位只在 `osmium-md` 邊界轉換，不進入 strategy 或
execution simulation。

market matching、auction post-state、order-entry 與 disposal 不另依賴 instrument-day background
或今日處置名單：前四者由 event evidence、reducer、session phase 依序產生，缺少 evidence 時保留
`Unknown`／missing；來源未證實的處置欄位不由稀疏成交、時間間隔或 trial 頻率猜測。只有會影響
帳務或 fill 的既有 simulation、instrument economics、contract 與 session 設定進入 effective
config／execution identity。

每個 event 包含 instrument、trading date、source format、`match_time`、`observed_time`（不早於 `match_time`）、可選 source sequence 與 payload。同一 source observation 的不可分割內容以單一 event 原子處理。

### REPLAY-02 排序

`observed_time` 是第一排序鍵與 replay clock。相同時間依版本化內容鍵（ordering rule v5）排序：market rank、source format、source sequence、symbol、event kind、`match_time` 與 event fingerprint。同一 feed 內依 `seq` 前進；不同 feed 之間的順序只保證可重現，不代表交易所的全域封包順序。

### REPLAY-03 市場狀態

每個商品的 `MarketState` 分開保存 firm 完整五檔、最近成交、累計量，最新 indicative
auction observation、reducer-owned `MarketSignal`／`MarketPhase`、最後 `match_time` 與
state version。新的完整 firm snapshot 取代舊 firm snapshot；`MarketStatus` 只更新其實際
攜帶的 signal 與累計量，不清空既有 firm book／trade；indicative observation 不得覆寫
firm state。系統不重建逐筆委託或 queue。

### REPLAY-04 處理順序

```text
select event
  -> advance replay clock
  -> atomically reduce MarketState
  -> derive TradingContext
  -> invoke strategy with post-event read-only state
  -> process strategy output and feedback
```

策略不能取得下一事件、未完成狀態或日後才知道的統計值。

### REPLAY-05 Session 與多商品

- universe 使用明確 market／symbol 清單與 semantic session kinds。
- planner 以版本化 calendar／profile 產生 session plan 與前後五分鐘 window。
- replay 以 bounded streaming merge 合併多商品 streams。
- WarmUp、Active 與 CoolDown 控制策略可見性與交易資格，不合成市場事件。

### REPLAY-06 錯誤處理

缺少或無效時間、順序錯誤、未知語義、非法價量、資料與 manifest 不一致時，系統停止，不發布結果。degraded policy 只能由明確設定選擇，並留下警告。

## 7. 策略需求

### STRAT-01 策略能力與邊界

- strategy 以 Rust trait 實作，編譯進 binary 並加入 compiled registry；外部 crate 可提供自己的 registry，不需複製 runner source。
- strategy identity 包含 id、version、binary identity 與 canonical parameter checksum。
- strategy 宣告 explicit universe 與 session kinds。
- callback 只能讀取目前 event、更新後的 `MarketStateView`、`TradingContext`、session context 與 deterministic feedback。
- strategy 可產生 indicator、order intent、scheduled request、timer 與非負 `cash charge`；能力由 runner 明確授予。
- strategy 不得修改 market state、replay clock 或 historical event，也不得讀取網路、wall clock、未記錄 randomness 或 future data。
- callback error 或 panic 必須使 run 明確失敗，不能發布成功結果。

## 8. 模擬與帳務需求

### SIM-01 成交模型

預設 `subsequent_event_v1` 模型只在 origin event 之後的 eligible event 判定成交：

- market order 使用後續可觀察價格並套用 slippage。
- limit order 需有後續成交或行情穿越限價的證據。
- quantity 可受觀察成交量或顯示量限制。
- 不確定時採保守結果，不推定真實 queue position。

可選的 `scheduled_visible_depth_v1` 以 execution control time、market-data latency、order latency、snapshot staleness 與最多五檔可見量執行。control action 不得寫入 replay event stream，也不得修改 `MarketState` 或 event/state checksum。

### SIM-02 帳務

- 所有 fill、cash、position、fee、tax、cash charge、realized／unrealized P&L 與 marking 變化可追溯至 order intent 或 strategy callback output。
- 同一 callback 的 cash charges 先整批驗證再提交，並在同批新 orders 的資金判定前扣除；任一筆非法時不得留下部分更新。
- 每筆 fill 保存 `fee_delta` 與 `tax_delta`。當沖稅事後重算可使後續 `tax_delta` 為負，代表調整值而非負稅率。
- exact 金額與比率不經 binary floating-point。
- equity、futures 與 options 使用明確的 instrument economics 與 accounting model。
- 每次執行結束需 reconciliation；失敗不得發布 successful performance。

## 9. 操作與非功能需求

### OPS-01 操作

- CLI 尚在重建；目前以 library API 與 tests 執行。
- 錯誤需指出 category 與可辨識的 market、symbol、date、format 或 artifact context。

### OPS-02 執行結果

run artifacts 至少保存 effective config checksum、execution plan identity、marketdata manifest identity、版本集合、strategy identity 與 materialized parameters、warnings、orders、fills、per-fill costs、cash charges、positions、P&L、event checksum 與 final-state checksum。output directory 以 staging 建立並以 atomic publish 完成。

### NFR-01 可重現

相同 marketdata identity、版本、strategy 與 effective config 必須得到相同事件順序、策略輸出、orders、fills 與帳務結果。並行與效能最佳化不得改變 domain result。

### NFR-02 效能

系統只解碼需要的欄位與 row groups，避免 universe 外的 mapping，並以 bounded stream 處理多商品。benchmark 使用固定資料與版本化輸入。

### NFR-03 安全與版本

marketdata identity、mapping、event schema、ordering、strategy output、fill model、accounting 與 run manifest 均需有相容性 identity；不相容時拒絕。

## 10. 能力矩陣與代表性證據

本矩陣是目前產品宣稱的邊界。`production owner` 是實際擁有該行為的 crate 或
composition-root；`evidence` 是可直接定位的 regression test 或 implementation。標示 `ignored`
的 test 需要 `/srv/md/v1`，執行方式見[驗證文件](../../development/validation.md)。任何一列都不表示
交易所完整撮合、逐筆委託、queue position 或 hidden liquidity 已被重建。

### 10.1 Source 與 replay

| capability | production owner | evidence | 宣稱 |
| --- | --- | --- | --- |
| manifest identity 與一致性 | `osmium-md` | `crates/osmium-md/tests/synthetic.rs#files_must_match_their_manifest_rows`、`#a_day_without_manifest_has_no_data`、`crates/osmium-md/src/day.rs` | 支援；讀取前後比對 manifest |
| marketdata → `DomainEvent` mapping | `osmium-md` | `crates/osmium-md/src/mapper.rs`、`crates/osmium-md/src/rows.rs#prices_convert_only_when_exact` | 支援 F6／F17／F23 |
| feed observed time | `osmium-md` | `crates/osmium-md/tests/synthetic.rs#observed_time_spans_the_whole_feed_regardless_of_selection` | 支援；與 universe 選擇無關 |
| 真實資料回播至 reducer | `osmium-md`、`market-state` | `crates/osmium-md/tests/marketdata.rs`（`ignored`） | 支援；事件數與 marketdata `md.read.replay` 一致 |
| deterministic multi-stream replay | `replay-engine` | `crates/replay-engine/tests/replay.rs#multi_stream_replay_merges_bounded_heads_and_ignores_sentinel`、`#multi_stream_binding_order_does_not_change_checksum` | 支援 |
| multi-instrument universe | `replay-engine`、`osmium-runner` | `crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` | 支援；每個 instrument 有獨立 state、simulator 與 ledger |
| multi-day／trading-date | `replay-engine` | `crates/replay-engine/tests/replay.rs#context_schedule_resets_state_across_trading_dates` | 支援 |
| firm／indicative isolation | `market-state` | `crates/market-state/tests/neutral_signal.rs`、`crates/market-state/tests/reducer.rs#taifex_opening_indicative_is_timeline_without_trade_or_volume_state` | 支援 |
| no-look-ahead／visibility | `strategy-api`、`osmium-runner` | `crates/strategy-api/tests/strategy.rs#callback_observes_committed_post_event_state_and_context`、`#failed_callback_discards_current_batch_after_core_commit`、`crates/osmium-runner/src/visibility.rs` | 支援；strategy 只讀目前 post-event state |

### 10.2 市場與商品

| market／instrument | 資料路徑 | regression evidence | 實際 claim |
| --- | --- | --- | --- |
| TWSE／TPEx 股票與 ETF | `Feed::{TwseF6, TpexF6}` → `Feed::state_profile` → `EquityV1` | `crates/osmium-md/tests/marketdata.rs#twse_f6_replays_through_the_reducer`（`ignored`）、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` | 資料回播至 reducer 已驗證；quantity 為 `TradingUnit`；與 runner 的串接尚在重建 |
| TWSE／TPEx 權證 | `Feed::{TwseF17, TpexF17}` → `EquityV1` | `crates/osmium-md/tests/marketdata.rs#odd_lot_and_warrant_feeds_replay_through_the_reducer`（`ignored`） | 同上；權證 periodic purpose 因 instruments 缺資料為 `Unknown` |
| TWSE／TPEx 盤中零股 | `Feed::{TwseF23, TpexF23}` | `crates/osmium-md/tests/marketdata.rs#odd_lot_and_warrant_feeds_replay_through_the_reducer`（`ignored`） | 資料回播至 reducer 已驗證；quantity 為 `Share`；execution／accounting 尚未連接 |
| TAIFEX future | `FuturesV1` | `crates/osmium-runner/src/lib.rs#multi_backtest_isolates_instruments_and_respects_latency` | 只有 model；marketdata 無資料源 |
| TAIFEX option | `OptionsV1` | `crates/execution-sim/src/accounting.rs#options_v1_moves_premium_cash_with_contract_multiplier`、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` | 只有 model；premium cash 使用 `price × quantity × multiplier`；marketdata 無資料源 |

缺少 contract economics、currency、multiplier 或 class identity 時，config／plan 必須拒絕，不以
symbol 或 market 猜測。

### 10.3 Market state

| capability | production owner | evidence | 缺證據時的行為 |
| --- | --- | --- | --- |
| continuous | `market-types`、`market-state`、`strategy-api` | `crates/market-state/tests/neutral_signal.rs#volatility_interruption_result_returns_to_continuous_without_using_a_trial_book` | explicit `Continuous` 才放行；不由預設值補上 |
| opening／closing | `market-state` | `crates/market-state/tests/neutral_signal.rs#opening_and_delayed_opening_uncross_end_in_continuous_phase`、`#closing_uncross_closes_without_erasing_the_last_firm_book` | 依 `AuctionPurpose` reducer transition |
| delayed open／close | `market-types`、`market-state` | `crates/market-state/tests/neutral_signal.rs#opening_and_delayed_opening_uncross_end_in_continuous_phase`、`crates/strategy-api/tests/strategy.rs#strategy_callback_can_observe_explicit_delayed_open_signal` | 只保留 event evidence 的 `delayed` |
| periodic／disposal | `market-types`、`market-state`、`osmium-md` | `crates/market-state/tests/neutral_signal.rs#periodic_uncross_resets_delay_for_next_round_and_reasserts_after_explicit_trigger`、`crates/osmium-md/src/mapper.rs#odd_lot_delay_marks_only_the_current_periodic_round` | 沒有來源證據不載入處置背景、不猜頻率 |
| volatility interruption | `market-state`、`execution-sim`、`osmium-md` | `crates/market-state/tests/neutral_signal.rs#volatility_interruption_result_returns_to_continuous_without_using_a_trial_book`、`crates/osmium-md/src/mapper.rs#volatility_interruption_round_carries_the_delay_direction`、`crates/execution-sim/src/lib.rs#volatility_interruption_restricts_new_orders_and_cancels_pending_market_orders` | restricted entry；既有 market ROD 依 policy cancel |
| Unknown | `market-state`、`osmium-md` | `crates/market-state/tests/neutral_signal.rs#no_observation_keeps_phase_but_unknown_signal_is_not_continuous`、`crates/osmium-md/src/mapper.rs#opening_delay_flag_is_unknown_when_null_and_unobserved_on_odd_lots` | unknown 不推測 continuous |

### 10.4 Strategy

| capability | production owner | evidence |
| --- | --- | --- |
| initialize／event／feedback／finalize lifecycle | `strategy-api`、`osmium-runner` | `crates/strategy-api/tests/strategy.rs#boxed_strategy_runs_through_the_generic_lifecycle`、`crates/osmium-runner/src/lib.rs#empty_stream_still_finalizes_and_reconciles` |
| explicit universe／sessions | `strategy-api`、`run-planner` | `crates/strategy-api/tests/strategy.rs#session_phase_and_twse_indicative_rules_are_explicit`、`crates/run-planner/tests/config.rs#strategy_declaration_must_match_effective_universe` |
| timer | `strategy-api`、`osmium-runner` | `crates/osmium-runner/src/scheduled.rs` 的 timer／control tests |
| scheduled request | `strategy-api`、`osmium-runner`、`execution-sim` | `crates/osmium-runner/src/scheduled.rs#delayed_observation_can_fill_at_control_time_without_market_event`、`crates/execution-sim/src/scheduled.rs` |
| deterministic outputs | `strategy-api`、`replay-engine` | `crates/strategy-api/tests/strategy.rs#deterministic_output_is_independent_of_input_order`、`crates/strategy-api/tests/registry.rs#defaults_and_key_order_have_one_canonical_identity` |
| callback transaction／failure | `strategy-api`、`osmium-runner` | `crates/strategy-api/tests/strategy.rs#failed_callback_discards_current_batch_after_core_commit`、`#panic_and_unavailable_capability_have_stable_categories` |
| no network／wall clock／future data | `strategy-api`、`replay-engine` | `crates/strategy-api/tests/strategy.rs#callback_observes_committed_post_event_state_and_context`、`crates/replay-engine/src/engine.rs` replay callback boundary |

### 10.5 Execution simulation

| capability | production owner | evidence |
| --- | --- | --- |
| market ROD／limit ROD | `execution-sim` | `crates/execution-sim/src/lib.rs#fill_trigger_distinguishes_market_and_control_origins`、`crates/execution-sim/src/depth.rs#limit_order_stops_before_non_marketable_level` |
| partial fill／quantity evidence | `execution-sim` | `crates/execution-sim/src/depth.rs#sell_sweeps_bids_and_reports_partial_depth`、`#market_order_quantity_is_not_used_as_price_or_fill_depth` |
| displayed depth／subsequent-event causality | `execution-sim`、`osmium-runner` | `crates/execution-sim/src/depth.rs#buy_sweeps_asks_from_best_price`、`crates/execution-sim/src/lib.rs#latency_is_added_in_milliseconds_and_requires_a_later_match_time` |
| adverse slippage | `execution-sim` | `crates/execution-sim/src/lib.rs#adverse_slippage_never_clamps_to_limit`、`crates/execution-sim/src/depth.rs#slippage_is_applied_before_limit_check` |
| market-data latency／order latency | `execution-sim`、`osmium-runner` | `crates/execution-sim/src/lib.rs#latency_is_added_in_milliseconds_and_requires_a_later_match_time`、`crates/osmium-runner/src/lib.rs#multi_backtest_isolates_instruments_and_respects_latency` |
| scheduled activation／expiry | `osmium-runner`、`execution-sim` | `crates/osmium-runner/src/scheduled.rs#passive_limit_fills_only_after_a_matching_book_becomes_visible`、`crates/osmium-runner/src/scheduled.rs#auction_fill_at_match_time_wins_over_expiry_before_feedback_visibility` |
| visible depth／auction cross | `execution-sim` | `crates/execution-sim/src/scheduled.rs#activation_sweeps_visible_levels_and_traces_control_fills`、`#auction_strict_cross_fills_the_entire_order_at_the_clearing_price` |
| cancel／session end／run end | `execution-sim`、`osmium-runner` | `crates/execution-sim/src/scheduled.rs#status_only_stability_pause_cancels_active_market_rods_and_blocks_activation`、`crates/osmium-runner/src/lib.rs#empty_stream_still_finalizes_and_reconciles` |
| stability restrictions | `market-state`、`execution-sim`、`osmium-runner` | `crates/market-state/tests/reducer.rs#intraday_stability_updates_only_indicative_state`、`crates/execution-sim/src/lib.rs#volatility_interruption_restricts_new_orders_and_cancels_pending_market_orders` |
| IOC／FOK | — | 明確 out of scope；不得由 ROD 或 scheduled policy 暗中模擬 |

### 10.6 Accounting 與 economics

| capability | production owner | evidence |
| --- | --- | --- |
| cash／position／realized／unrealized／marking | `execution-sim` | `crates/execution-sim/src/accounting.rs#average_cost_realized_pnl_and_reconciliation_are_deterministic`、`#mark_to_market_adjustments_reconcile_shared_cash_to_current_equity` |
| fee／tax／day-trade adjustment | `execution-sim` | `crates/execution-sim/src/accounting.rs#buy_then_sell_same_day_uses_reduced_tax_for_matched_quantity`、`#sell_then_buy_same_day_reprices_prior_sell_tax`、`#unmatched_or_ineligible_quantity_keeps_ordinary_tax` |
| cash charge | `execution-sim`、`osmium-runner` | `crates/execution-sim/src/accounting.rs#cash_charge_batch_is_atomic_reconciled_and_generic`、`crates/osmium-runner/src/scheduled.rs` |
| quantity unit／multiplier／currency | `run-planner`、`execution-sim` | `crates/run-planner/tests/config.rs#option_reference_is_required_consistent_and_bound_into_effective_identity`、`#every_universe_instrument_requires_valid_economics` |
| equity／futures／options model | `execution-sim` | `crates/execution-sim/src/accounting.rs#futures_cash_moves_only_on_realized_pnl_and_costs`、`#options_v1_moves_premium_cash_with_contract_multiplier` |
| reconciliation | `execution-sim`、`osmium-runner` | `crates/execution-sim/src/accounting.rs#multi_ledger_reconciles_equity_and_futures_cash_separately`、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` |
| per-fill costs | `execution-sim`、`osmium-runner` | `crates/execution-sim/src/accounting.rs#fixed_per_unit_fee_is_charged_for_each_filled_contract`、`crates/osmium-runner/src/artifacts.rs` |

### 10.7 Artifacts

| capability | production owner | evidence |
| --- | --- | --- |
| immutable publish | `osmium-runner` | `crates/osmium-runner/src/artifacts.rs` output-exists、staging 與 atomic publish paths；`crates/osmium-runner/src/lib.rs#empty_stream_still_finalizes_and_reconciles` |
| effective config checksum／plan identity | `run-planner`、`osmium-runner` | `crates/run-planner/tests/config.rs#semantic_checksum_excludes_data_root`、`crates/osmium-runner/src/artifacts.rs` |
| marketdata lineage | `osmium-md`、`osmium-runner` | `crates/osmium-md/src/day.rs`（`FeedDayIdentity`）；寫入 run manifest 尚在重建 |
| orders／fills／cash | `osmium-runner`、`execution-sim` | `crates/osmium-runner/src/artifacts.rs`、`crates/execution-sim/src/accounting.rs` |
| positions／performance | `osmium-runner`、`execution-sim` | `crates/osmium-runner/src/artifacts.rs`、`crates/execution-sim/src/accounting.rs` |
| event／final-state checksum | `replay-engine`、`market-state`、`osmium-runner` | `crates/replay-engine/tests/replay.rs#shuffled_inputs_produce_identical_checksums_and_final_state`、`crates/osmium-runner/src/artifacts.rs` |
| inspect | `osmium-runner` | `crates/osmium-runner/src/lib.rs#empty_stream_still_finalizes_and_reconciles`、`crates/osmium-runner/src/artifacts.rs` |

### 10.8 Required representative E2E

| scenario | required claim | representative evidence |
| --- | --- | --- |
| equity continuous market／limit + accounting | continuous state、ROD market／limit、cash／position 可重現 | `crates/execution-sim/src/lib.rs`、`crates/execution-sim/src/depth.rs`、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` |
| auction／delayed／periodic + causality | indicative 不作 fill evidence；正式 uncross trade 才可成交 | `crates/osmium-md/src/mapper.rs`、`crates/osmium-runner/src/scheduled.rs#auction_price_requires_an_explicit_trade_in_the_released_event`、`#auction_fill_at_match_time_wins_over_expiry_before_feedback_visibility` |
| scheduled visible depth + latency | visible time、activation、staleness、market/order latency 固定 | `crates/execution-sim/src/scheduled.rs#activation_sweeps_visible_levels_and_traces_control_fills`、`crates/osmium-runner/src/scheduled.rs#delayed_observation_can_fill_at_control_time_without_market_event`、`crates/osmium-runner/src/lib.rs#multi_backtest_isolates_instruments_and_respects_latency` |
| futures economics | multiplier、realized P&L、reconciliation | `crates/osmium-runner/src/lib.rs#multi_backtest_isolates_instruments_and_respects_latency`、`crates/execution-sim/src/accounting.rs#futures_cash_moves_only_on_realized_pnl_and_costs` |
| options economics | OptionsV1 premium cash、multiplier、reconciliation | `crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` |
| multi-instrument／multi-market | TWSE equity 與 TAIFEX option 在同一 bounded merge 中保持 state／ledger isolation | `crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` |
| failure／reconciliation | callback／input／checksum failure 不發布 successful run；正常 run 必須 reconcile | `crates/strategy-api/tests/strategy.rs#failed_callback_discards_current_batch_after_core_commit`、`crates/osmium-runner/src/lib.rs#empty_stream_still_finalizes_and_reconciles`、`crates/osmium-runner/src/artifacts.rs` |

Repository 內的 synthetic tests 只驗證介面、mapping 與 replay；真實交易日的覆蓋由 `ignored`
real-data tests 對 `/srv/md/v1` 執行。

## 11. 驗證原則

| 需求面 | 主要證據 |
| --- | --- |
| 資料邊界 | manifest 一致性、row 驗證、mapping unit tests 與 real-data sweep |
| 回播 | shuffled-input ordering、multi-stream merge、state reducer 與 checksum tests |
| 策略 | read-only compile tests、no-look-ahead、callback transaction 與 registry tests |
| 模擬帳務 | market／limit、scheduled depth、latency、fee／tax、P&L 與 reconciliation tests |

操作驗證見 [驗證文件](../../development/validation.md)。

## 12. 參考資料

- [TWSE TCP/IP 證券交易資訊網路文件](https://dsp.twse.com.tw/tcpipTradingFiles/list)
- [TPEx 上櫃股票 IP 行情網路規格書](https://dsp.tpex.org.tw/storage/regular_system/%E4%B8%8A%E6%AB%83%E8%82%A1%E7%A5%A8IP%E8%A1%8C%E6%83%85%E7%B6%B2%E8%B7%AF%E8%A6%8F%E6%A0%BC%E6%9B%B8%28V.12.18_TCPIP%29.pdf)
- [TAIFEX 逐筆行情資訊傳輸作業手冊](https://www.taifex.com.tw/cht/8/techDocsDetails?idx=67)

資料來源與範圍以 [ADR-0007](../decisions/0007-marketdata-native.md) 為準。
