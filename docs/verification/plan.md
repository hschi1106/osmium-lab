# M1 Verification Plan

## 1. 文件目的

本文件定義 M1「TWSE 2330 最小 deterministic replay vertical slice」的驗證
方法、test ID、fixture policy、golden artifacts 與執行順序。它是測試契約，
不是測試已通過的證據；實際結果由
[M1 Acceptance](acceptance.md) 登錄。

```text
verification_plan_version = 1
scope                     = M1
```

依據：

- [產品需求](../product-requirements.md)
- [M1 TWSE replay](../increments/M1-twse-replay.md)
- [Market Types](../design/market-types.md)
- [MarketState](../design/market-state.md)
- [Replay Engine](../design/replay-engine.md)
- [Strategy API](../design/strategy-api.md)
- [Fixture provenance](fixture-provenance.md)

## 2. 驗證範圍

### 2.1 M1 必須驗證

- 經核准的 Teralion `TWSE / 2330 / STOCK_SNAPSHOT` fixture 可正規化為
  `QuoteSnapshot`。
- Teralion wire type 與 domain event 分離。
- exact value、完整五檔 snapshot、deal、cumulative volume、raw／typed flags
  的 mapping。
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
- 只保留 `STOCK_SNAPSHOT` 的最小必要 records。
- 精確複製 source values，不改造價格、數量、時間、flags 或 book levels。
- 移除 API transport metadata 時，在 metadata 明示移除欄位。
- 有 fixture bytes SHA-256 與每筆 source selector。

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

- fixture checksum 與 metadata selector 可回溯。
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

- approved wire fixture → TWSE normalizer → `QuoteSnapshot`。
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
| `M1-T001` | `fixture_metadata_matches_bytes`：fixture SHA-256、record count 與 selectors 相符 |
| `M1-T002` | `fixture_contains_no_secrets`：未發現 key、auth header、cookie 或 private request metadata |
| `M1-T003` | `approved_stock_snapshot_normalizes`：每筆 approved record 成功產生 `QuoteSnapshot` |
| `M1-T004` | `snapshot_mapping_is_atomic`：book、deal、cum volume、flags 同屬單一 tick event |
| `M1-T005` | `book_and_volume_changes_are_preserved`：fixture 中 book 與 cumulative volume 的變化未丟失 |
| `M1-T006` | `source_flags_are_preserved`：raw flags、typed view、opening／closing／trial 語意符合 TWSE mapping |

### 6.2 Types、canonical encoding 與 errors

| Test ID | 測試 |
| --- | --- |
| `M1-T010` | `match_time_is_exact_utc_microseconds` |
| `M1-T011` | `invalid_match_time_is_rejected` |
| `M1-T012` | `decimal_never_rounds_or_uses_float` |
| `M1-T013` | `unknown_format_is_rejected` |
| `M1-T014` | `invalid_snapshot_shape_is_rejected` |
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

`M1-T021` 使用 approved fixture 中相同 `match_time` records，加上必要的
`synthetic_domain` event 覆蓋完整 tie-break 分支。若 real fixture 只能證明部分
分支，report 必須分開列出，不得把 synthetic evidence 說成 source observation。

### 6.4 MarketState

| Test ID | 測試 |
| --- | --- |
| `M1-T030` | `quote_snapshot_replaces_complete_book` |
| `M1-T031` | `state_version_increments_once_per_event` |
| `M1-T032` | `no_observation_does_not_clear_state` |
| `M1-T033` | `explicit_clear_removes_value` |
| `M1-T034` | `repeated_replay_has_same_final_state_checksum` |
| `M1-T035` | `reducer_failure_does_not_publish_partial_state` |

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
| `fixture.sha256` | approved fixture exact bytes |
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
