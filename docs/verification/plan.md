# M1／M2 Verification Plan

## 1. 文件目的

本文件定義 M1「TWSE 2330 regular-market deterministic replay vertical slice」及
M2「Teralion 真實資料準備與離線 backtest」的驗證方法、test ID、fixture policy、
golden artifacts 與執行順序。它是測試契約，不是測試已通過的證據；實際結果由
M1 [Acceptance](acceptance.md)、後續 M2 acceptance register 及 machine-readable
CI evidence 登錄。

```text
verification_plan_version = 2
scope                     = M1 + M2
```

依據：

- [產品需求](../product-requirements.md)
- [M1 TWSE replay](../increments/M1-twse-replay.md)
- [M2 offline backtest](../increments/M2-offline-backtest.md)
- [Market Types](../design/market-types.md)
- [Data Sync](../design/data-sync.md)
- [MarketState](../design/market-state.md)
- [Replay Engine](../design/replay-engine.md)
- [Strategy API](../design/strategy-api.md)
- [Execution Simulation](../design/execution-sim.md)
- [Local Data](../operations/local-data.md)
- [CLI](../operations/cli.md)
- [Fixture provenance](fixture-provenance.md)

## 2. 驗證範圍

### 2.1 M1 必須驗證

- 經核准的 Teralion `TWSE / 2330` regular fixture 可將 `STOCK_SNAPSHOT` 與
  `STOCK_REALTIME` 正規化為 `QuoteSnapshot`／`TradeBatch`。
- Teralion wire type 與 domain event 分離。
- exact value、完整五檔 snapshot、deal、cumulative volume、raw／typed flags
  的 mapping。
- realtime intermediate／final `1+1` grouping、source phase ordering，以及
  intermediate trade 不清除既有 book。
- deterministic event identity、canonical encoding、ordering 與 tie-break。
- replacement-style `MarketState` reducer 與 deterministic final-state checksum。
- post-event strategy callback、唯讀 state、no-lookahead 與 deterministic
  `ExampleStrategy` output。
- invalid time、unknown format、invalid numeric shape 與 unknown flag 的 strict
  handling。
- 相同 fixture、plan、strategy、binary 與 version 重跑得到相同結果。
- 測試不使用 Teralion API key，且可在無 network 的環境執行。

### 2.2 M1 不驗證

- Teralion download、cursor recovery 與 verified local source reuse。
- derived replay cache build／reuse。
- order intent、fill model、fees、positions、cash、P&L 或 accounting。
- CLI、result directory UX、TAIFEX、TPEx 或 multi-instrument performance。

上述內容留在 M2 以後，不得用 M1 test name 暗示已完成。

## 3. Entry gates

測試實作可先使用 local/private source 開發，但 M1 acceptance 執行前必須全部
符合：

| Gate | 條件 | 目前來源 |
| --- | --- | --- |
| `GATE-FIXTURE-01` | fixture public redistribution 或 repository commit 權限有明確 approval reference | [fixture provenance](fixture-provenance.md) |
| `GATE-FIXTURE-02` | fixture metadata、source selectors、checksum 與 extraction method 完整 | 同上 |
| `GATE-SECRET-01` | fixture 與 metadata 不含 API key、authorization header、cookie、request URL secret | secret scan |
| `GATE-SPEC-01` | market-types、market-state、replay-engine、strategy-api versions 已固定 | design docs |
| `GATE-BUILD-01` | Rust workspace 與 M1 owning crates 可離線建置 | implementation |

任何 gate 未通過時，相關 acceptance status 是 `Blocked`，不是 `Passed` 或
`Failed`。特別是 `GATE-FIXTURE-01` 不可由技術測試取代。

## 4. Test data policy

### 4.1 `approved_source`

唯一可作為 source mapping acceptance evidence 的資料類別。它必須：

- 來自 provenance 指定的 Teralion acquisition。
- 保留全部 `STOCK_SNAPSHOT` 與 `STOCK_REALTIME` regular records；排除已知但
  不支援的盤中零股 format。
- 精確複製 source values，不改造價格、數量、時間、flags 或 book levels。
- 移除 API transport metadata 時，在 metadata 明示移除欄位。
- 有每個 fixture shard 的 SHA-256、固定 concatenation order 的 fixture-set
  SHA-256、完整 source page checksums 與 deterministic selection policy。

### 4.2 `derived_negative`

從 approved source record 複製後，只改一個被測欄位，例如 invalid
`match_time`、unknown format 或 decimal overflow。每筆必須標示：

```text
data_class          = derived_negative
derived_from        = source selector
mutation            = exact field and operation
expected_error_code = stable error category
```

它不能作為 real-world field mapping 證據。

### 4.3 `synthetic_domain`

可直接建立 `DomainEvent` 測試 ordering、state reducer、clock 與 strategy API，
但不能替代 approved source 的 normalizer integration test。synthetic event
必須滿足 domain invariant；測 invalid invariant 時要明示 expected constructor
failure。

## 5. Test levels

### 5.1 Static／provenance checks

- fixture shard／set checksums 與 metadata selection policy 可回溯。
- fixture path 只包含核准檔案。
- secret pattern scan。
- docs links、YAML parse 與 traceability mapping。

### 5.2 Unit tests

- exact time、decimal、quantity、flags 與 canonical primitive encoding。
- event validation、fingerprint、canonical bytes。
- ordering key 與 final tie-break。
- MarketState replacement reducer、version 與 checksum。
- strategy output transaction、canonical output 與 error categories。

### 5.3 Integration tests

- approved wire fixture → TWSE normalizer → `QuoteSnapshot`／`TradeBatch`。
- normalized events → deterministic merge → state reducer → strategy callback。
- warning／error record 與 run outcome。
- shuffled input repeated runs。

### 5.4 End-to-end M1 test

在 network disabled 且未提供 Teralion secret 的 process：

```text
approved fixture
-> normalizer
-> ReplayPlan
-> ReplayEngine
-> MarketState
-> ExampleStrategy
-> golden evidence
```

end-to-end test 不得呼叫 data-sync、remote HTTP 或讀取 `raw/`。

## 6. Stable test catalog

Test ID 是 acceptance 與 CI report 的穩定 identity；Rust module／function 名稱可
調整，但 report 必須保留 Test ID。

### 6.1 Fixture 與 normalizer

| Test ID | 測試 |
| --- | --- |
| `M1-T001` | `fixture_metadata_matches_bytes`：fixture SHA-256、format counts 與 selection policy 相符 |
| `M1-T002` | `fixture_contains_no_secrets`：未發現 key、auth header、cookie 或 private request metadata |
| `M1-T003` | `approved_stock_snapshot_normalizes`：window 內 snapshot records 成功產生 `QuoteSnapshot` |
| `M1-T004` | `snapshot_mapping_is_atomic`：book、deal、cum volume、flags 同屬單一 tick event |
| `M1-T005` | `book_and_volume_changes_are_preserved`：fixture 中 book 與 cumulative volume 的變化未丟失 |
| `M1-T006` | `source_flags_are_preserved`：raw flags、typed view、opening／closing／trial 語意符合 TWSE mapping |
| `M1-T007` | `approved_stock_realtime_normalizes`：window 內 final realtime records 產生 `QuoteSnapshot` |
| `M1-T008` | `realtime_intermediate_final_group_normalizes`：三個 real `1+1` groups 各產生 `TradeBatch -> QuoteSnapshot` |
| `M1-T009` | `invalid_realtime_group_is_rejected`：missing／multiple／volume-mismatch group 整組拒絕 |

### 6.2 Types、canonical encoding 與 errors

| Test ID | 測試 |
| --- | --- |
| `M1-T010` | `match_time_is_exact_utc_microseconds` |
| `M1-T011` | `invalid_match_time_is_rejected` |
| `M1-T012` | `decimal_never_rounds_or_uses_float` |
| `M1-T013` | `unknown_format_is_rejected` |
| `M1-T014` | `invalid_quote_or_trade_shape_is_rejected` |
| `M1-T015` | `unknown_status_bits_are_preserved_and_warned` |
| `M1-T016` | `canonical_event_golden` |
| `M1-T017` | `event_fingerprint_changes_with_semantics` |

### 6.3 Ordering 與 replay

| Test ID | 測試 |
| --- | --- |
| `M1-T020` | `shuffled_input_has_same_order` |
| `M1-T021` | `same_match_time_uses_ordering_rule_v2` |
| `M1-T022` | `duplicate_occurrences_are_not_collapsed` |
| `M1-T023` | `clock_never_moves_backwards` |
| `M1-T024` | `repeated_replay_has_same_event_checksum` |
| `M1-T025` | `unsupported_version_fails_before_replay` |

`M1-T021` 必須使用 approved fixture 的三個 realtime intermediate／final groups
證明 source phase ordering，並以其他 real same-time records 與必要的
`synthetic_domain` event 覆蓋完整 tie-break 分支。report 必須分開列出 real 與
synthetic evidence。

### 6.4 MarketState

| Test ID | 測試 |
| --- | --- |
| `M1-T030` | `quote_snapshot_replaces_complete_book` |
| `M1-T031` | `state_version_increments_once_per_event` |
| `M1-T032` | `no_observation_does_not_clear_state` |
| `M1-T033` | `explicit_clear_removes_value` |
| `M1-T034` | `repeated_replay_has_same_final_state_checksum` |
| `M1-T035` | `reducer_failure_does_not_publish_partial_state` |
| `M1-T036` | `intermediate_trade_updates_trade_without_clearing_book` |

### 6.5 Strategy

| Test ID | 測試 |
| --- | --- |
| `M1-T040` | `callback_observes_post_event_state` |
| `M1-T041` | `callback_count_equals_accepted_event_count` |
| `M1-T042` | `example_strategy_output_is_deterministic` |
| `M1-T043` | `failed_callback_discards_current_output_batch` |
| `M1-T044` | `strategy_error_marks_run_failed_after_core_commit` |
| `M1-T045` | `strategy_context_exposes_no_next_event` |
| `M1-T046` | `market_state_view_is_not_mutable` |
| `M1-T047` | `m1_order_intent_capability_is_unavailable` |
| `M1-T048` | `example_strategy_selects_only_declared_twse_2330_stream` |
| `M1-T049` | `callback_trading_context_matches_event_and_state` |

`M1-T045` 與 `M1-T046` 應以 compile-fail／public API surface test 驗證，而不只
靠 runtime assertion。

### 6.6 End-to-end 與 operations metadata

| Test ID | 測試 |
| --- | --- |
| `M1-T050` | `m1_vertical_slice_matches_goldens` |
| `M1-T051` | `m1_repeated_run_is_byte_identical` |
| `M1-T052` | `m1_runs_without_api_key` |
| `M1-T053` | `m1_runs_with_network_disabled` |
| `M1-T054` | `run_summary_records_versions_counts_warnings_checksums` |

## 7. Golden artifacts

M1 至少固定：

| Golden | 內容 |
| --- | --- |
| `fixture-set.sha256` | approved shards 依 metadata order 串接後的 exact bytes |
| `normalized-events.bin` | `canonical_event_version = 1` bytes |
| `event-stream.blake3` | `canonical_replay_event_stream_version = 1` checksum |
| `final-state.blake3` | `canonical_final_state_set_version = 1` checksum |
| `strategy-output.bin` | `strategy_output_version = 1` bytes |
| `strategy-output.blake3` | ExampleStrategy output checksum |
| `warnings.yaml` | ordered stable warning category 與 occurrence |
| `run-summary.yaml` | versions、counts、checksums、outcome；不含 wall-clock value |

golden 檔不得依賴 Rust `Debug`、hash map iteration、platform path、locale 或
serializer default。

### 7.1 Golden update protocol

golden 只能在下列情況更新：

1. 對應 spec／interface／design version 已先 review 並變更。
2. diff 能解釋每一筆 semantic change。
3. 重新執行全部 M1 tests。
4. acceptance report 記錄舊／新 checksum 與變更原因。

測試失敗時直接覆寫 expected golden，不是合法 update。

## 8. Repetition 與 perturbation

determinism suite 至少執行：

- 原始 fixture record order。
- 固定 seed 產生的 3 種 shuffled input；seed 寫入 test code／report。
- 同一 input 連續 replay 10 次。
- debug 與 release profile 各一次；若 release 尚無 CI job，acceptance 明示
  `NotRun`，不得推定結果。

所有 run 的 normalized event bytes、event stream checksum、final state
checksum 與 strategy output bytes 必須完全相同。performance duration 不參與
deterministic comparison。

## 9. Offline 與 secret validation

### 9.1 Secret scan

至少掃描：

- `api[_-]?key`
- `authorization`
- `bearer`
- `cookie`
- 已知 Teralion secret value 的 exact hash／value（由 CI secret scanner 注入，
  不寫入 repository）
- URL query 中的 credential-like parameters

scanner finding 必須人工分類；不得因 fixture 價格數字碰巧匹配 pattern 就靜默
忽略。

### 9.2 Offline execution

`M1-T052` 必須移除 Teralion key 與相關 auth environment variables。
`M1-T053` 必須在 CI 使用 network-disabled container／sandbox；只 mock HTTP
client 不足以證明沒有旁路連線。

## 10. 執行命令

workspace 建立後，minimum acceptance commands：

```sh
cargo fmt --check
cargo test --workspace
cargo test --workspace --release
```

fixture／acceptance runner 可額外提供 focused command，但不得取代 workspace
tests。CI network-disabled job 必須執行標有 `M1-T050` 至 `M1-T054` 的 suite。

repository 提供的完整 M1 acceptance harness 會先執行上述 debug／release checks，
再以 OS sandbox 或 network-disabled container 連續執行兩次 release replay，
比對完整 artifact bytes，並以 atomic rename 發布 evidence：

```sh
tools/run_m1_acceptance.sh \
  --output target/m1-acceptance
```

macOS 的 `auto` runner 使用 `sandbox-exec`；Linux 使用 Docker
`--network none`。container image 可以在進入 network-disabled runtime 前準備，
但 replay process 本身不得具有網路能力或 `TERALION_API_KEY`。

文件階段使用：

```sh
git diff --check
```

以及 YAML parse、relative Markdown link existence 與 fixture metadata schema
check。命令只有實際成功執行後才能登錄為 evidence。

## 11. Evidence capture

每次 acceptance run 產生 machine-readable report：

```text
AcceptanceEvidence {
    acceptance_contract_version
    verification_plan_version
    git_commit
    rust_toolchain
    build_profile
    network_policy
    test_id
    outcome
    artifact_checksums
    error_or_blocker
}
```

report 可存於 CI artifact；若要 commit，使用
`docs/verification/evidence/m1/<approved-run-id>/`，且不得包含 source fixture
payload、secret 或 machine-specific absolute path。

## 12. Exit criteria

M1 verification 完成需要：

- 所有 entry gates closed。
- `M1-T001` 至 `M1-T054` 中適用的 tests 全部 `Passed`。
- debug 與 release commands 成功。
- offline／no-key tests 成功。
- golden artifacts 有 reviewable checksum。
- acceptance mapping 沒有 `Failed`、`Blocked` 或未解釋的 `NotRun`。
- `docs/traceability.yaml` 登錄實際 evidence path；不能只登錄本 plan。

## 13. M2 verification scope

### 13.1 M2 必須驗證

- versioned config、effective config、plan identity 與所有 defaults 可檢查。
- coverage、symbol range、ticks、daily instrument、opaque cursor 全頁下載。
- interrupted sync、retry/resume、atomic publish及 immutable complete revision。
- `Missing`／`Building`／`Complete`／`Incomplete`／`Corrupt` 分類。
- second-sync complete-source reuse 且零 HTTP requests。
- source-bound cache reuse、version invalidation、刪除後離線 deterministic rebuild。
- frozen TWSE `2330 / 2026-07-27 / regular` universe只開啟必要 stream。
- network-disabled/no-key replay、backtest及 inspect。
- intent validation、origin-event no-fill、TradingEligibility gate及 feedback順序。
- `TopOfBookV1`／`TradePrintV1` market/limit fill、slippage、quantity cap、
  deterministic allocation及 partial fill。
- exact instrument economics、fee、tax、cash、Average Cost position、marking與 P&L。
- ledger reconciliation、failed/degraded publication及 offline inspect。
- source/cache/event/state/strategy/order/fill/ledger/result identities的 determinism。
- live authorized API evidence、recorded contract tests及 offline CI的證據邊界。

### 13.2 M2 不驗證

- Teralion 當日增量或即時交易。
- TPEx、TAIFEX、權證、選擇權及多商品 portfolio。
- 盤中零股、盤後零股、盤後定價及鉅額交易。
- 真實委託排隊、hidden liquidity或 exchange matching engine。
- cancel/replace、IOC/FOK/stop/iceberg。
- borrow、margin、multi-account、multi-currency或 corporate action。
- 未定義公式與 period 的 Sharpe ratio等高階績效。

synthetic order scenarios可覆蓋 simulation branch，但不能取代 real Teralion source
對 format、session、price、quantity及 flags mapping的證據。

## 14. M2 entry gates

| Gate | 條件 | 未滿足時 |
| --- | --- | --- |
| `M2-GATE-M1-01` | M1 event/state/strategy golden及 network-disabled gate完成 | M2 implementation可進行，formal acceptance不得 Passed |
| `M2-GATE-AUTH-01` | live reference acquisition具有合法 authorization/provenance | live sync evidence Blocked |
| `M2-GATE-SPEC-01` | data-sync、execution-sim、local-data及 CLI V1/V2 contracts已 review | 對應 implementation test Blocked |
| `M2-GATE-CONFIG-01` | checked-in acceptance config無 secret且 schema固定 | end-to-end Blocked |
| `M2-GATE-LIVE-01` | Teralion endpoint/cursor/auth/time contract與 interface一致 | live API suite Blocked |
| `M2-GATE-OFFLINE-01` | CI可強制 network disabled並移除 auth env | offline claim Blocked |
| `M2-GATE-TOOLCHAIN-01` | debug/release workspace可建置 | profile comparison Blocked |

gate狀態與 test outcome分開。外部 authorization 或 CI capability缺少是 `Blocked`，
不是用 mock test標成 `Passed`。

## 15. M2 test data與 execution profiles

### 15.1 `live_authorized_source`

只用於證明實際 coverage/range/ticks/daily-instrument endpoint、cursor與 published
source revision。evidence保存 sanitized query identity、counts及 checksums，不提交
credential/full cursor。live outage不影響 recorded/offline unit tests的可重跑性。

### 15.2 `recorded_transport_contract`

保存經核准、redacted的 request/response contract或等價 deterministic fake transport，
覆蓋：

- multi-page terminal cursor。
- retryable/non-retryable error。
- cursor stall/loop/query drift。
- partial response、中斷及 resume。
- zero records及 malformed envelope。

它驗證 client state machine，不可冒充 live endpoint或 network-disabled evidence。

### 15.3 `published_reference_source`

M2 reference identity：

```text
market       = TWSE
symbol       = 2330
trading_date = 2026-07-27
session      = regular
download     = [08:55, 13:35) by received_at
replay       = [08:55, 13:35) by match_time
```

它必須是由 live authorized sync發布的 complete immutable revision，含 manifest、
per-page `ZstdPerPageV1` ticks、compressed daily instrument及雙 checksums。若 source payload不適合提交 repository，
CI以受控 artifact/cache提供，仍須保存 immutable identity與 authorization reference。

### 15.4 `synthetic_simulation`

直接建立合法 `DomainEvent`／MarketState／TradingContext sequence，專門覆蓋難以從
單日 2330保證出現的 market/limit、partial fill、reversal、missing mark及 failure
branches。每個 vector明示 model/economics config與 expected canonical records。

### 15.5 Profiles

| Profile | Network | Credential | 目的 |
| --- | --- | --- | --- |
| `live` | enabled | runtime secret | 真實 endpoint、cursor及 publish |
| `recorded` | disabled/transport-injected | none | 每次 CI的 client branches |
| `offline-debug` | 強制 disabled | none | source/cache/backtest correctness |
| `offline-release` | 強制 disabled | none | release determinism及 baseline |
| `perturbed` | disabled | none | page/discovery/worker/buffer invariance |

## 16. M2 stable test catalog

### 16.1 Config、plan與 CLI preflight

| Test ID | 測試 |
| --- | --- |
| `M2-T100` | `acceptance_config_materializes_effective_values` |
| `M2-T101` | `config_rejects_unknown_duplicate_secret_and_invalid_exact_numeric_fields` |
| `M2-T102` | `plan_identity_ignores_paths_wall_clock_and_yaml_formatting` |
| `M2-T103` | `plan_classifies_source_verify_cache_and_network_actions` |
| `M2-T104` | `plan_is_read_only_and_does_not_execute_strategy` |
| `M2-T105` | `session_plan_materializes_twse_0855_1335_half_open_windows` |
| `M2-T106` | `plan_freezes_strategy_universe_models_economics_and_versions` |
| `M2-T107` | `cli_exit_status_distinguishes_all_m2_categories` |

### 16.2 Teralion client與 cursor

| Test ID | 測試 |
| --- | --- |
| `M2-T110` | `live_coverage_and_symbol_range_include_reference_partition` |
| `M2-T111` | `live_ticks_cursor_reaches_terminal_without_truncation` |
| `M2-T112` | `live_daily_instrument_is_bound_to_reference_partition` |
| `M2-T113` | `opaque_cursor_is_returned_exactly_and_query_identity_is_frozen` |
| `M2-T114` | `cursor_stall_loop_duplicate_page_and_query_drift_fail` |
| `M2-T115` | `retry_resume_matches_uninterrupted_uncompressed_page_bytes_and_revision` |
| `M2-T116` | `terminal_zero_records_differs_from_coverage_and_incomplete` |
| `M2-T117` | `credential_and_full_cursor_are_redacted_from_published_outputs` |
| `M2-T118` | `source_pages_and_daily_instrument_use_zstd_level3_checksum_without_dictionary` |
| `M2-T119` | `compression_output_changes_do_not_change_uncompressed_source_revision_identity` |

### 16.3 Source staging、publish與 verify

| Test ID | 測試 |
| --- | --- |
| `M2-T120` | `page_body_and_checkpoint_are_durable_before_next_request` |
| `M2-T121` | `partial_sync_never_publishes_complete_revision` |
| `M2-T122` | `atomic_publish_exposes_only_verified_revision` |
| `M2-T123` | `complete_revision_is_immutable_and_content_change_creates_new_revision` |
| `M2-T124` | `verify_classifies_missing_building_complete_incomplete_corrupt` |
| `M2-T125` | `verify_recomputes_manifest_counts_payload_and_instrument_checksums` |
| `M2-T126` | `verify_rejects_unknown_format_wrong_session_and_trading_date_ownership` |
| `M2-T127` | `second_sync_reuses_complete_source_with_zero_http_requests` |
| `M2-T128` | `strict_rejects_non_complete_and_degraded_never_accepts_corrupt` |
| `M2-T129` | `disk_full_and_rename_failure_leave_no_complete_reference` |

### 16.4 Replay cache與 selective stream

| Test ID | 測試 |
| --- | --- |
| `M2-T130` | `cache_build_binds_source_and_all_semantic_versions` |
| `M2-T131` | `valid_cache_is_reused_without_source_json_parse` |
| `M2-T132` | `cache_checksum_or_version_mismatch_is_rejected_before_replay` |
| `M2-T133` | `deleted_cache_rebuilds_offline_with_same_payload_checksum` |
| `M2-T134` | `cache_reader_validates_count_bounds_ordering_checksum_and_eof` |
| `M2-T135` | `duplicate_occurrences_survive_cache_build_and_read` |
| `M2-T136` | `only_reference_stream_opens_and_outside_universe_sentinel_stays_closed` |
| `M2-T137` | `cache_reader_uses_bounded_buffer_larger_than_test_event_count` |
| `M2-T138` | `cache_builder_streams_zstd_without_writing_uncompressed_json` |
| `M2-T139` | `verify_detects_compressed_frame_and_both_checksum_corruption` |

### 16.5 Strategy、intent與 feedback

| Test ID | 測試 |
| --- | --- |
| `M2-T140` | `intent_validation_uses_stable_first_failure_order` |
| `M2-T141` | `valid_intent_identity_is_deterministic_and_collision_checked` |
| `M2-T142` | `origin_occurrence_never_fills_its_new_order` |
| `M2-T143` | `same_match_time_later_ordering_key_is_subsequent` |
| `M2-T144` | `trial_indicative_unknown_and_cooldown_never_fill` |
| `M2-T145` | `warmup_limit_intent_can_be_accepted_but_trial_cannot_fill` |
| `M2-T146` | `closing_result_evaluates_only_older_pending_orders` |
| `M2-T147` | `feedback_order_and_feedback_generated_intent_preserve_no_lookahead` |
| `M2-T148` | `callback_failure_discards_current_intents_and_marks_run_failed` |

### 16.6 Fill、allocation與 slippage

| Test ID | 測試 |
| --- | --- |
| `M2-T150` | `top_of_book_market_buy_and_sell_use_subsequent_current_side` |
| `M2-T151` | `trade_print_market_buy_and_sell_use_ordered_subsequent_print` |
| `M2-T152` | `limit_buy_sell_require_touch_and_preserve_price_improvement` |
| `M2-T153` | `adverse_slippage_never_crosses_limit_or_becomes_nonpositive` |
| `M2-T154` | `missing_or_stale_evidence_keeps_order_pending` |
| `M2-T155` | `observed_capacity_produces_partial_fill_without_double_use` |
| `M2-T156` | `multiple_orders_allocate_by_acceptance_sequence_deterministically` |
| `M2-T157` | `unlimited_quantity_has_distinct_model_identity` |
| `M2-T158` | `remaining_day_orders_cancel_at_normal_end_of_run` |

### 16.7 Economics、accounting與 reconciliation

| Test ID | 測試 |
| --- | --- |
| `M2-T160` | `missing_unit_multiplier_currency_or_rounding_fails_before_first_event` |
| `M2-T161` | `economic_quantity_notional_fee_tax_and_cash_effect_recompute_exactly` |
| `M2-T162` | `fee_and_tax_side_minimum_precision_and_rounding_are_independent` |
| `M2-T163` | `average_cost_handles_add_reduce_close_and_signed_reversal` |
| `M2-T164` | `fill_transaction_failure_publishes_no_partial_accounting_state` |
| `M2-T165` | `last_observable_mark_never_uses_future_close_or_stats` |
| `M2-T166` | `missing_mark_is_unavailable_not_zero` |
| `M2-T167` | `reconciliation_rebuilds_orders_cash_positions_fees_tax_and_pnl` |
| `M2-T168` | `tampered_ledger_fails_run_and_suppresses_success_performance` |
| `M2-T169` | `order_fill_ledger_checksums_match_canonical_goldens` |

### 16.8 Workflow、artifacts、offline與 determinism

| Test ID | 測試 |
| --- | --- |
| `M2-T170` | `successful_backtest_atomically_publishes_required_artifact_set` |
| `M2-T171` | `failed_run_never_publishes_complete_checksums_or_performance` |
| `M2-T172` | `degraded_run_has_distinct_status_identity_and_scopes` |
| `M2-T173` | `inspect_reads_success_failed_and_degraded_without_reexecution` |
| `M2-T174` | `replay_marks_simulation_artifacts_not_applicable` |
| `M2-T175` | `network_disabled_no_key_backtest_and_inspect_make_zero_http_requests` |
| `M2-T176` | `existing_output_directory_is_rejected_without_overwrite` |
| `M2-T177` | `secret_scan_covers_source_cache_runs_logs_and_repository` |
| `M2-T178` | `ten_runs_and_three_input_perturbations_are_byte_identical` |
| `M2-T179` | `cache_hit_rebuild_debug_release_and_worker_settings_are_byte_identical` |
| `M2-T180` | `performance_report_records_dataset_environment_io_memory_and_checksums` |
| `M2-T181` | `run_short_circuits_after_failed_stage_and_preserves_stage_results` |
| `M2-T182` | `m1_fixture_replay_contract_remains_compatible` |

## 17. M2 acceptance mapping

| Acceptance | Required tests |
| --- | --- |
| `M2-AC-01` plan | `M2-T100`–`M2-T106` |
| `M2-AC-02` cursor | `M2-T110`–`M2-T116` |
| `M2-AC-03` interrupted sync | `M2-T115`、`M2-T118`、`M2-T120`–`M2-T122`、`M2-T129` |
| `M2-AC-04` second sync | `M2-T119`、`M2-T123`、`M2-T127` |
| `M2-AC-05` verify states | `M2-T124`–`M2-T128` |
| `M2-AC-06` cache | `M2-T130`–`M2-T135`、`M2-T138`、`M2-T139` |
| `M2-AC-07` universe | `M2-T136`、`M2-T137` |
| `M2-AC-08` offline | `M2-T133`、`M2-T175`、`M2-T177` |
| `M2-AC-09` origin/feedback | `M2-T140`–`M2-T143`、`M2-T147` |
| `M2-AC-10` market states | `M2-T144`–`M2-T146` |
| `M2-AC-11` fills | `M2-T150`–`M2-T158` |
| `M2-AC-12` economics/P&L | `M2-T160`–`M2-T167` |
| `M2-AC-13` reconciliation | `M2-T167`–`M2-T169` |
| `M2-AC-14` determinism | `M2-T169`、`M2-T178`–`M2-T180` |
| `M2-AC-15` inspect | `M2-T170`–`M2-T176`、`M2-T181` |

每個 acceptance evidence必須列出實際 test IDs；只引用本 table或 M2 increment不能
標為 `Passed`。

## 18. M2 golden artifacts

至少固定：

| Golden | 內容 |
| --- | --- |
| `effective-config.blake3` | canonical effective acceptance config |
| `execution-plan.blake3` | canonical frozen plan |
| `source-manifest-semantic.blake3` | sanitized source semantic manifest |
| `source-storage-inventory.blake3` | compressed object names、sizes、policies及 checksums |
| `cache-payload.blake3` | cache header與 canonical events |
| `event-stream.blake3` | replay event sequence |
| `final-state.blake3` | final MarketState set |
| `strategy-output.bin/.blake3` | strategy canonical outputs |
| `orders.bin/.blake3` | accepted/rejected/pending/final order lifecycle |
| `fills.bin/.blake3` | canonical fills/evidence/economics |
| `ledger.bin/.blake3` | accounting entries |
| `positions.yaml` | exact final positions/cost basis |
| `performance.yaml` | cash、fee、tax、realized/unrealized P&L |
| `result.blake3` | versioned complete domain result identity |

golden update沿用第 7.1 節 protocol，且必須先更新對應 config/source/model/schema
version與說明 P&L差異。測試失敗時直接接受新 fill/P&L不是合法更新。

## 19. M2 failure injection

至少注入：

- HTTP timeout、429/5xx、auth failure。
- cursor stall/loop、query drift、different retry page。
- crash after page body/before checkpoint、after revision/before current reference。
- disk-full、short write、fsync/rename failure。
- missing page、modified payload、wrong checksum/count/manifest identity。
- truncated/modified zstd frame、compressed checksum或 uncompressed checksum mismatch。
- cache descriptor mismatch、truncated payload、wrong EOF/order。
- strategy callback error/panic。
- arithmetic overflow、illegal slippage、fee/tax rounding error。
- ledger entry deletion/modification、fill over remaining、capacity over-allocation。
- output artifact write/publish failure。

每個 injection驗證 failure stage、atomicity、exit category、diagnostics、secret
redaction及下一次 safe recovery action。

## 20. M2 commands與 evidence

minimum local commands：

```sh
cargo fmt --check
cargo test --workspace
cargo test --workspace --release
```

另外必須有可 machine-select的 suites：

```text
m2-recorded-contract
m2-offline-debug
m2-offline-release
m2-determinism
m2-failure-injection
m2-performance-baseline
m2-live-authorized
```

實際 cargo nextest/test command由 implementation落地後補入，不得在尚不存在時以
假 command作 evidence。

M2 evidence schema至少為：

```text
M2AcceptanceEvidence {
    acceptance_contract_version
    verification_plan_version
    test_id
    profile
    git_commit
    rust_toolchain
    build_profile
    network_policy
    credential_policy
    effective_config_checksum
    execution_plan_identity
    source_revision_identity
    cache_identity
    strategy_binary_identity
    model_version_set
    outcome
    artifact_checksums
    metrics?
    error_or_blocker?
}
```

live evidence不得包含 credential/full cursor。offline evidence必須由 execution
environment證明 network policy，而不是只相信 application counter。

## 21. M2 exit criteria

M2 verification完成需要：

- 第 14 節 entry gates全部 closed。
- 第 16 節列出的 required `M2-T*` tests全部 `Passed`。
- live authorized、recorded、offline debug/release、determinism、failure injection
  profiles成功。
- reference source complete且 second-sync HTTP request count為零。
- cache可 reuse、刪除後可離線重建。
- network-disabled/no-key backtest及 inspect成功。
- order/fill/ledger/result goldens有 reviewable checksum。
- reconciliation successful，tampered negative test確實 failed。
- performance baseline包含 correctness checksums且未偽造 threshold。
- M1 fixture replay regression仍通過。
- acceptance/traceability登錄實際 code/evidence paths。
- 沒有 secret、`Failed`、`Blocked`或未解釋的 `NotRun`。
