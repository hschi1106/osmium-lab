# osmium-lab 產品需求

## 1. 產品定位

`osmium-lab` 是以 Rust 建立、可接入不同歷史行情供應商的台灣市場回播與回測平台。目前內建 Teralion Feed Archive adapter，但 planner、domain event、cache codec、replay、strategy、simulation 與通用驗收契約不得依賴 Teralion wire schema。系統依 `match_time` 重播可觀察的成交與行情快照，讓策略在沒有未來資料的前提下執行，並以明確、可版本化的模型估算成交與帳務結果。

產品原則依序為：

1. 一份設定可以完成資料規劃、準備與執行。
2. 已驗證的來源資料可跨多次回測重用。
3. 相同資料、版本與設定產生相同結果。
4. 模型不宣稱來源資料無法支持的撮合或排隊精度。
5. 市場、商品、session、五檔、成交與來源 flags 的差異保留在明確的介面契約中。

## 2. 支援範圍

支援的市場與商品：

- TWSE 股票與權證。
- TPEx 股票與權證。
- TAIFEX 期貨與選擇權。
- 處置證券以一般商品方式回播；來源若提供特殊狀態，平台保存並呈現該資訊，但不重算處置撮合規則。

不支援的範圍：

- 盤中零股、盤後零股、盤後定價與鉅額交易。
- 即時交易或自動下單。
- 完整交易所撮合、逐筆委託簿、真實 queue position 或 hidden liquidity 推論。
- 瞬間價格穩定措施、處置撮合規則或其他交易所內部狀態的重建。
- 從低粒度資料合成不存在的高粒度資料。
- 回播或回測期間自動存取網路。

## 3. 資料能力與限制

每個 source adapter 能證明的原始資料是該次回播能力的上限。目前 Teralion adapter 的能力為：

- TWSE／TPEx quote 可提供完整最佳五檔、可選成交、累計量與來源 flags。
- TAIFEX tick 可提供成交批次、完整五檔與非時間軸的 close／stats 記錄。
- 同一商品可能有多種 `format`，每種格式由明確的 normalizer 處理。
- 商品 metadata 可能缺少 multiplier、underlying 或其他欄位；缺值不得自行推定。
- 每個執行 universe 的 instrument class 必須由設定明確指定；TAIFEX futures 另須明確指定 contract shape 與 session profile，不依 symbol 推定。contract identity 必須進入 effective config checksum。
- `received_at` 是擷取時間，只用於 archive query 與診斷；`match_time` 是唯一回播時間。
- 沒有有效 `match_time` 的記錄不得插入事件時間軸。

任何 provider wire payload 與 domain event 都必須分離。未知或不支援的格式需明確拒絕或以設定允許的 degraded 模式略過，並保留可追溯警告。Provider-specific flags、format 名稱與 credential 不得洩漏到 strategy 或通用 replay contract。

## 4. 使用流程

```text
RunConfig
  -> execution plan
  -> source sync / verify
  -> replay cache prepare
  -> deterministic replay
  -> strategy callbacks
  -> execution simulation / accounting
  -> immutable run artifacts
```

資料同步與回測可分開執行。source 與 cache 準備完成後，`replay`、`backtest`、`run` 與 `inspect` 不需要網路或 API credential。

## 5. 資料需求

### DATA-01 資料取得

- source 由設定中的 stable source identity 選擇 adapter；planner、partition identity 與本地目錄不得自行假設特定供應商。
- 目前內建 Teralion adapter 支援 coverage、symbol range、ticks、daily instrument 與 opaque cursor pagination；這些 endpoint 與 credential 行為只屬於該 adapter。
- 所有 cursor pages 必須完整取得；HTTP、schema、cursor 或儲存錯誤不得被視為合法空頁。
- 只有已結束交易日可發布為可重用 source partition。
- credential 只存在於 online sync context，不得寫入 source、cache、log 或 run artifacts。

### DATA-02 本地來源資料

- partition identity 至少包含 source、market、trading date、symbol、session 與 session plan identity。
- published source revision 是 immutable artifact，保存 frozen query、payload、metadata、頁數、筆數與 checksum。
- sync 使用 staging 與 atomic publish；失敗的 staging 不得被視為完整資料。
- 已發布 revision 不得被靜默覆寫。

### DATA-03 完整性

每個 partition 必須可辨識為 `Missing`、`Building`、`Complete`、`Incomplete` 或 `Corrupt`。strict 執行只接受通過 identity、manifest、cursor、payload 與 checksum 驗證的 `Complete` source；degraded 執行必須明確記錄品質與警告。

### DATA-04 回播快取

- replay cache 由 verified source 建立，可刪除並離線重建。
- cache identity 綁定 source checksum、normalizer mapping、event schema、ordering rule 與 cache format。
- cache 失效或損壞不應觸發 source 重新下載。
- replayer 只開啟 execution plan 中需要的商品與日期 streams。

### DATA-05 商品資料

- identity 與可用的 kind、expiry、strike、option side、currency、multiplier 需保留來源。
- 影響帳務的 quantity unit、multiplier 與 currency 必須由已驗證 metadata 或明確設定提供。
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
TWSE／TPEx raw status bits 只在 provider boundary 解碼，不進入 strategy 或 execution
simulation。

market matching、auction post-state、order-entry 與 disposal 不另依賴 instrument-day background
或今日處置名單：前四者由 event evidence、reducer、session phase 依序產生，缺少 evidence 時保留
`Unknown`／missing；來源未證實的處置欄位不由稀疏成交、時間間隔或 trial 頻率猜測。只有會影響
帳務或 fill 的既有 simulation、instrument economics、contract 與 session 設定進入 effective
config／execution identity。

每個 event 包含 instrument、trading date、source format、`match_time`、可選 source sequence 與 payload。同一 source observation 的不可分割內容以單一 event 原子處理。

### REPLAY-02 排序

`match_time` 是第一排序鍵與唯一 replay clock。相同時間依版本化內容鍵排序：market rank、symbol、source format、source phase、event kind、source sequence 與 event fingerprint。此順序只保證可重現，不代表交易所的全域封包順序。

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
- 多商品以 bounded streaming merge 執行，不需將整段資料載入記憶體。
- WarmUp、Active 與 CoolDown 控制策略可見性與交易資格，不合成市場事件。

### REPLAY-06 錯誤處理

缺少或無效時間、順序錯誤、未知格式、非法價量、資料不完整或 checksum 不符時，系統依 strict policy 停止，或依明確選擇的 degraded policy 繼續並留下警告。

## 7. 策略需求

### STRAT-01 策略能力與邊界

- strategy 以 Rust trait 實作，編譯進 binary 並加入 registry；外部 binary 可透過公開的 CLI application API 注入自己的 compiled registry，不需複製 CLI 或 runner source。
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

- CLI 提供 `init`、`config check`、`plan`、`data sync`、`data verify`、`cache prepare`、`replay`、`backtest`、`run` 與 `inspect`。
- 錯誤需指出 category 與可辨識的 market、symbol、date、format 或 artifact context。

### OPS-02 執行結果

run artifacts 至少保存 effective config checksum、execution plan identity、source/cache checksum、版本集合、strategy identity 與 materialized parameters、warnings、orders、fills、per-fill costs、cash charges、positions、P&L、event checksum 與 final-state checksum。output directory 以 staging 建立並以 atomic publish 完成。

### NFR-01 可重現

相同 source、cache identity、版本、strategy 與 effective config 必須得到相同事件順序、策略輸出、orders、fills 與帳務結果。並行與效能最佳化不得改變 domain result。

### NFR-02 效能

系統優先避免重複下載、JSON parsing 與 universe 外 I/O，並以 bounded stream 處理多商品。benchmark 使用固定資料與版本化輸入。

### NFR-03 安全與版本

credential 不得進入設定、資料 artifact、log 或版本控制。source interface、normalizer mapping、event schema、ordering、cache、strategy output、fill model、accounting 與 run manifest 均需有相容性 identity；不相容時拒絕或由 verified source 重建。

## 10. 能力矩陣與代表性證據

本矩陣是目前產品宣稱的邊界。`production owner` 是實際擁有該行為的 crate 或
composition-root；`evidence` 是可直接定位的 regression test、fixture 或 implementation。
市場／商品列為「模型支援」時，表示已完成 provider normalization、MarketState、execution
與 accounting model 的連接；不表示交易所完整撮合、逐筆委託、queue position 或 hidden
liquidity 已被重建。

### 10.1 Source 與 replay

| capability | production owner | evidence | 宣稱 |
| --- | --- | --- | --- |
| provider-neutral verified source | `data-sync`、`market-types` | `crates/data-sync/src/verify.rs`、`crates/data-sync/src/partition.rs`、`crates/run-planner/tests/partition_plan.rs#source_and_cache_states_classify_into_explicit_actions` | 支援；Teralion 只在 composition root 接入 |
| cache prepare／reuse／rebuild | `data-sync`、`run-planner` | `crates/data-sync/src/cache.rs`、`crates/run-planner/tests/partition_plan.rs#source_and_cache_states_classify_into_explicit_actions` | 支援；cache 是可重建 artifact |
| deterministic multi-stream replay | `replay-engine` | `crates/replay-engine/tests/replay.rs#multi_stream_replay_merges_bounded_heads_and_ignores_sentinel`、`#multi_stream_binding_order_does_not_change_checksum` | 支援 |
| multi-instrument universe | `replay-engine`、`osmium-runner` | `crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` | 支援；每個 instrument 有獨立 state、simulator 與 ledger |
| multi-day／trading-date | `run-planner`、`replay-engine` | `crates/run-planner/tests/partition_plan.rs#execution_plan_requires_every_universe_date_partition`、`crates/replay-engine/tests/replay.rs#context_schedule_resets_state_across_trading_dates` | 支援；repository fixtures 仍明確標示 `complete_day: false` |
| firm／indicative isolation | `market-state` | `crates/market-state/tests/neutral_signal.rs`、`crates/market-state/tests/reducer.rs#taifex_opening_indicative_is_timeline_without_trade_or_volume_state` | 支援 |
| no-look-ahead／visibility | `strategy-api`、`osmium-runner` | `crates/strategy-api/tests/strategy.rs#callback_observes_committed_post_event_state_and_context`、`#failed_callback_discards_current_batch_after_core_commit`、`crates/osmium-runner/src/visibility.rs` | 支援；strategy 只讀目前 post-event state |

### 10.2 市場與商品

| market／instrument | production path | regression evidence | 實際 execution／accounting claim |
| --- | --- | --- | --- |
| TWSE equity | `TwseNormalizer` → `twse_regular` → `EquityV1` | `crates/providers/teralion/tests/twse_full_fixture.rs`、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` | 模型支援；quantity 為 `TradingUnit` |
| TWSE warrant | `TwseNormalizer::new_warrant` → `twse_warrant` → `EquityV1` | `crates/providers/teralion/tests/twse_m5_warrant_fixture.rs`、`crates/providers/teralion/tests/twse_normalizer.rs` | 模型支援；沿用 equity accounting，不宣稱 warrant-specific exchange matching |
| TPEx equity | `TpexNormalizer` → `tpex_regular` → `EquityV1` | `crates/providers/teralion/tests/tpex_full_fixture.rs`、`crates/providers/teralion/tests/tpex_normalizer.rs` | 模型支援；quantity 為 `TradingUnit` |
| TPEx warrant | `TpexNormalizer::new_warrant` → `tpex_warrant` → `EquityV1` | `crates/providers/teralion/tests/tpex_m5_warrant_fixture.rs`、`crates/providers/teralion/tests/tpex_normalizer.rs` | 模型支援；沿用 equity accounting，不宣稱 warrant-specific exchange matching |
| TAIFEX future | `TaifexNormalizer::new_futures` → `taifex_futures` → `FuturesV1` | `crates/providers/teralion/tests/taifex_fixtures.rs`、`crates/osmium-runner/src/lib.rs#multi_backtest_isolates_instruments_and_respects_latency` | 模型支援；multiplier、contract shape 與 session profile 必須明確 |
| TAIFEX option | `TaifexNormalizer::new_options` → `taifex_options` → `OptionsV1` | `crates/providers/teralion/tests/taifex_m5_option_fixture.rs`、`crates/execution-sim/src/accounting.rs#options_v1_moves_premium_cash_with_contract_multiplier`、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` | 模型支援；premium cash 使用 `price × quantity × multiplier` |

以上六類是「可被規劃、正規化、回播並以明確模型執行／記帳」的支援宣稱，不是 full exchange
matching。缺少 contract economics、currency、multiplier 或 class identity 時，config／plan
必須拒絕，不以 symbol 或 market 猜測。

### 10.3 Market state

| capability | production owner | evidence | 缺證據時的行為 |
| --- | --- | --- | --- |
| continuous | `market-types`、`market-state`、`strategy-api` | `crates/market-state/tests/neutral_signal.rs#volatility_interruption_result_returns_to_continuous_without_using_a_trial_book`、`crates/providers/teralion/tests/production_market_path.rs#provider_events_reach_production_reducer_and_context_without_background_lookup` | explicit `Continuous` 才放行；不由預設值補上 |
| opening／closing | `market-state` | `crates/market-state/tests/neutral_signal.rs#opening_and_delayed_opening_uncross_end_in_continuous_phase`、`#closing_uncross_closes_without_erasing_the_last_firm_book` | 依 `AuctionPurpose` reducer transition |
| delayed open／close | `market-types`、`market-state` | `crates/market-state/tests/neutral_signal.rs#opening_and_delayed_opening_uncross_end_in_continuous_phase`、`crates/strategy-api/tests/strategy.rs#strategy_callback_can_observe_explicit_delayed_open_signal` | 只保留 event evidence 的 `delayed` |
| periodic／disposal | `market-types`、`market-state`、provider boundary | `crates/market-state/tests/neutral_signal.rs#periodic_uncross_keeps_the_next_periodic_auction_and_repeated_delay_has_no_counter`、`crates/providers/teralion/tests/production_market_path.rs#provider_events_reach_production_reducer_and_context_without_background_lookup` | 沒有來源證據不載入處置背景、不猜頻率 |
| volatility interruption | `market-state`、`execution-sim`、`osmium-runner` | `crates/market-state/tests/neutral_signal.rs#volatility_interruption_result_returns_to_continuous_without_using_a_trial_book`、`crates/execution-sim/src/lib.rs#volatility_interruption_restricts_new_orders_and_cancels_pending_market_orders` | restricted entry；既有 market ROD 依 policy cancel |
| Unknown／degraded | `market-state`、`run-planner`、`replay-engine` | `crates/market-state/tests/neutral_signal.rs#no_observation_keeps_phase_but_unknown_signal_is_not_continuous`、`crates/providers/teralion/tests/production_market_path.rs#missing_market_signal_replays_unrelated_event_as_unknown`、`crates/run-planner/tests/partition_plan.rs#strict_plan_rejects_degraded_scope` | unknown 不推測 continuous；strict 失敗，degraded 留 warning |

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
| quantity unit／multiplier／currency | `osmium-config`、`run-planner`、`execution-sim` | `crates/run-planner/tests/config.rs#option_reference_is_required_consistent_and_bound_into_effective_identity`、`#every_universe_instrument_requires_valid_economics` |
| equity／futures／options model | `osmium-cli`、`execution-sim` | `crates/osmium-cli/src/command.rs` model selection、`crates/execution-sim/src/accounting.rs#futures_cash_moves_only_on_realized_pnl_and_costs`、`#options_v1_moves_premium_cash_with_contract_multiplier` |
| reconciliation | `execution-sim`、`osmium-runner` | `crates/execution-sim/src/accounting.rs#multi_ledger_reconciles_equity_and_futures_cash_separately`、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` |
| per-fill costs | `execution-sim`、`osmium-runner` | `crates/execution-sim/src/accounting.rs#fixed_per_unit_fee_is_charged_for_each_filled_contract`、`crates/osmium-runner/src/artifacts.rs` |

### 10.7 Artifacts

| capability | production owner | evidence |
| --- | --- | --- |
| immutable publish | `osmium-runner` | `crates/osmium-runner/src/artifacts.rs` output-exists、staging 與 atomic publish paths；`crates/osmium-runner/src/lib.rs#empty_stream_still_finalizes_and_reconciles` |
| effective config checksum／plan identity | `osmium-config`、`run-planner`、`osmium-runner` | `crates/run-planner/tests/config.rs#semantic_checksum_excludes_data_root`、`crates/osmium-runner/src/artifacts.rs` |
| source／cache lineage | `data-sync`、`osmium-cli`、`osmium-runner` | `crates/data-sync/src/cache.rs`、`crates/osmium-cli/src/command.rs`、`crates/osmium-runner/src/artifacts.rs` |
| orders／fills／cash | `osmium-runner`、`execution-sim` | `crates/osmium-runner/src/artifacts.rs`、`crates/execution-sim/src/accounting.rs` |
| positions／performance | `osmium-runner`、`execution-sim` | `crates/osmium-runner/src/artifacts.rs`、`crates/execution-sim/src/accounting.rs` |
| event／final-state checksum | `replay-engine`、`market-state`、`osmium-runner` | `crates/replay-engine/tests/replay.rs#shuffled_inputs_produce_identical_checksums_and_final_state`、`crates/osmium-runner/src/artifacts.rs` |
| inspect | `osmium-runner` | `crates/osmium-runner/src/lib.rs#empty_stream_still_finalizes_and_reconciles`、`crates/osmium-runner/src/artifacts.rs` |

### 10.8 Required representative E2E

| scenario | required claim | representative evidence |
| --- | --- | --- |
| equity continuous market／limit + accounting | continuous state、ROD market／limit、cash／position 可重現 | `crates/execution-sim/src/lib.rs`、`crates/execution-sim/src/depth.rs`、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` |
| auction／delayed／periodic + causality | indicative 不作 fill evidence；正式 uncross trade 才可成交 | `crates/providers/teralion/tests/production_market_path.rs`、`crates/osmium-runner/src/scheduled.rs#auction_price_requires_an_explicit_trade_in_the_released_event`、`#auction_fill_at_match_time_wins_over_expiry_before_feedback_visibility` |
| scheduled visible depth + latency | visible time、activation、staleness、market/order latency 固定 | `crates/execution-sim/src/scheduled.rs#activation_sweeps_visible_levels_and_traces_control_fills`、`crates/osmium-runner/src/scheduled.rs#delayed_observation_can_fill_at_control_time_without_market_event`、`crates/osmium-runner/src/lib.rs#multi_backtest_isolates_instruments_and_respects_latency` |
| futures economics | multiplier、realized P&L、reconciliation | `crates/osmium-runner/src/lib.rs#multi_backtest_isolates_instruments_and_respects_latency`、`crates/execution-sim/src/accounting.rs#futures_cash_moves_only_on_realized_pnl_and_costs` |
| options economics | option reducer、OptionsV1 premium cash、multiplier、reconciliation | `crates/providers/teralion/tests/taifex_m5_option_fixture.rs`、`crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` |
| multi-instrument／multi-market | TWSE equity 與 TAIFEX option 在同一 bounded merge 中保持 state／ledger isolation | `crates/osmium-runner/src/lib.rs#multi_market_backtest_reconciles_equity_and_option_economics` |
| failure／reconciliation | callback／input／checksum failure 不發布 successful run；正常 run 必須 reconcile | `crates/strategy-api/tests/strategy.rs#failed_callback_discards_current_batch_after_core_commit`、`crates/osmium-runner/src/lib.rs#empty_stream_still_finalizes_and_reconciles`、`crates/osmium-runner/src/artifacts.rs` |

本 repository 的 synthetic fixture 只驗證介面、normalization、replay 與離線 smoke；完整交易日
與外部 provider authorization 仍屬 repository 外的 release gate，不將 `complete_day: false`
的 fixture 升格為 full-day coverage。

## 11. 驗證原則

| 需求面 | 主要證據 |
| --- | --- |
| 資料同步 | cursor、resume、atomic publish、checksum 與 second-run reuse tests |
| 通用來源契約 | provider-neutral partition integrity、cache lineage／schema 與 lifecycle contract tests |
| Adapter conformance | 各 provider 自有 wire fixtures／真實外部 partitions、mapping 與 unknown-format negative tests；不得取代通用 gate |
| 回播 | shuffled-input ordering、multi-stream merge、state reducer 與 checksum tests |
| 策略 | read-only compile tests、no-look-ahead、callback transaction 與 registry tests |
| 模擬帳務 | market／limit、scheduled depth、latency、fee／tax、P&L 與 reconciliation tests |
| 操作 | CLI contract、offline flow 與 release smoke tests |

需求與程式入口的對照見 [追溯矩陣](traceability.yaml)，操作驗證見 [驗證文件](operations/validation.md)。

## 12. 參考資料

- [Teralion Feed Archive API](https://docs.teraliontech.com/feed-archive/)
- [TWSE TCP/IP 證券交易資訊網路文件](https://dsp.twse.com.tw/tcpipTradingFiles/list)
- [TPEx 上櫃股票 IP 行情網路規格書](https://dsp.tpex.org.tw/storage/regular_system/%E4%B8%8A%E6%AB%83%E8%82%A1%E7%A5%A8IP%E8%A1%8C%E6%83%85%E7%B6%B2%E8%B7%AF%E8%A6%8F%E6%A0%BC%E6%9B%B8%28V.12.18_TCPIP%29.pdf)
- [TAIFEX 逐筆行情資訊傳輸作業手冊](https://www.taifex.com.tw/cht/8/techDocsDetails?idx=67)

介面行為以 [Teralion](interfaces/teralion.md)、[TWSE](interfaces/twse.md)、[TPEx](interfaces/tpex.md) 與 [TAIFEX](interfaces/taifex.md) 文件中由 fixture 與測試固定的範圍為準。
