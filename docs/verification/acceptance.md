# M1 Acceptance Contract

## 1. 文件目的

本文件把 M1 acceptance criteria 映射至穩定 test IDs、必要 evidence 與 pass/fail
規則。它同時記錄目前 paperwork 狀態，但不把尚未實作或尚未執行的測試標成
通過。

```text
acceptance_contract_version = 1
scope                       = M1
current_overall_status      = Passed
```

fixture approval gate 已關閉；詳見
[Fixture provenance](fixture-provenance.md)。M1 vertical slice implementation、固定
goldens、debug／release workspace tests、no-key 與 network-disabled replay 已完成。
正式 evidence 位於
[`evidence/m1/formal-2026-07-31/acceptance-report.yaml`](evidence/m1/formal-2026-07-31/acceptance-report.yaml)。

依據：

- [M1 TWSE replay](../increments/M1-twse-replay.md)
- [Verification plan](plan.md)
- [Fixture provenance](fixture-provenance.md)

## 2. Status model

每個 criterion 與 test 使用：

| Status | 定義 |
| --- | --- |
| `NotRun` | 尚未執行，且沒有外部 gate 阻止 |
| `Passed` | 指定 test 與 evidence 已實際成功 |
| `Failed` | test 已執行但不符合 expected result |
| `Blocked` | 明確 entry gate 未滿足，無法形成有效 acceptance evidence |
| `NotApplicable` | 由 spec 明確排除；必須附理由 |

文件存在、code review、人工推測或 local raw 資料存在都不能單獨形成
`Passed`。

## 3. Entry gate register

| Gate | Status | Closure evidence |
| --- | --- | --- |
| `GATE-FIXTURE-01` redistribution／commit approval | `Passed` | authorized private-repository approval record |
| `GATE-FIXTURE-02` fixture metadata 與 checksum | `Passed` | regular fixture metadata、source manifest 與 exact SHA-256 |
| `GATE-SECRET-01` secret scan | `Passed` | field allowlist 與 forbidden-pattern scan，0 findings |
| `GATE-SPEC-01` M1 design versions fixed | `Passed` | market-types 1、market-state 1、replay-engine 1、strategy-api 1 |
| `GATE-BUILD-01` offline Rust workspace build | `Passed` | 2026-07-31 debug／release 各 105 tests／43 suites，workspace Clippy 無 warning |

`GATE-SPEC-01` 只代表 paper contract 已固定，不代表 implementation 符合。

## 4. Acceptance mapping

### M1-AC-01：合法且可回溯的 real fixture

**要求：** 使用獲准 commit 的實際 Teralion TWSE 2330 regular fixture，涵蓋
`STOCK_SNAPSHOT` 與 `STOCK_REALTIME`，並能回溯 source acquisition。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T001`–`M1-T003`, `M1-T007`, `M1-T008` | approval reference、fixture metadata、SHA-256、normalizer report、secret scan | `Passed` |

Pass 必須同時滿足：

- provenance approval 欄位完整。
- 每個 fixture shard 與 fixture-set checksum 都和 metadata 一致。
- deterministic selection policy 與全部 source page checksums 可回溯。
- 無 secret。
- replay window 內的 final records 成功產生 `QuoteSnapshot`。
- 三筆 intermediate records 與其 final pair 成功產生
  `TradeBatch -> QuoteSnapshot`。
- replay window 外 record 明確分類，不以 `received_at` 塞入 timeline。

### M1-AC-02：quote／trade 的 atomic market observation

**要求：** final quote 的完整 book、deal、cumulative volume 與 flags 在同一
`QuoteSnapshot` 中處理；intermediate 的 trade、cumulative volume 與 flags 在
同一 `TradeBatch` 中處理，且不清除 book。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T004`–`M1-T009`, `M1-T012`, `M1-T016`, `M1-T036` | mapping assertions、group rejection、canonical event golden | `Passed` |

Pass 必須證明至少兩個 match times，且有 book 與 cumulative volume 變化；fixture
若提供 deal，deal mapping 也必須驗證。raw flags 必須保留。

### M1-AC-03：輸入擾動後仍 deterministic

**要求：** fixture order 打亂與 repeated replay 不改變 event stream 或 final
state。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T017`, `M1-T020`, `M1-T023`, `M1-T024`, `M1-T034`, `M1-T051` | seeds、10-run checksums、event／state goldens | `Passed` |

Pass 是 normalized event bytes、ordered event checksum、final-state checksum 與
strategy output bytes 全部相同；只有 summary 文字相同不算。

### M1-AC-04：相同 `match_time` 的 deterministic tie-break

**要求：** 相同 `match_time` 的 occurrence 依 `ordering_rule_version = 3`
排列；realtime intermediate 必須在 final 前，duplicate 不被 collapse。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T021`, `M1-T022` | real same-time selectors、synthetic branch matrix、ordered occurrence list | `Passed` |

Pass report 必須分開標示 real fixture 與 synthetic coverage。

### M1-AC-05：MarketState replacement semantics

**要求：** 完整 snapshot 取代舊 book；`TradeBatch` 保留既有 book；state
version 每 accepted event 恰增一次；reducer error 不發布 partial state。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T030`–`M1-T036` | reducer assertions、final-state golden、failure test | `Passed` |

### M1-AC-06：Strategy 看到 post-event state 且無前視

**要求：** 每個 accepted event 對應一次 callback；callback 只讀 event 後 state，
不能取得 next event 或 mutable state。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T040`–`M1-T049` | integration report、compile-fail tests、strategy output golden | `Passed` |

Pass 需要 runtime ordering evidence 與 compile-time API evidence。M1 發出 order
intent 必須得到 `CapabilityUnavailable`，不能靜默接受。

### M1-AC-07：invalid `match_time` strict failure

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T010`, `M1-T011` | exact-time cases、stable error category | `Passed` |

invalid offset、invalid date、precision loss 或 overflow 必須拒絕，不可修正、
round 或改用 `received_at`。

### M1-AC-08：unknown format strict failure

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T013`, `M1-T014`, `M1-T025` | derived-negative cases、unsupported version result | `Passed` |

unknown format／version 不得以 generic event 進入 timeline。

### M1-AC-09：unknown flag 保留且可診斷

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T006`, `M1-T015`, `M1-T054` | raw bits assertion、ordered warning、run summary | `Passed` |

unknown bit 不得被丟棄或自行命名；若 interface policy 允許 accepted event，必須
保留 raw bits 並產生 stable warning。

### M1-AC-10：完全離線且不需 API key

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T050`, `M1-T052`, `M1-T053`, `M1-T054` | network-disabled sandbox report、sanitized environment、goldens、run summary | `Passed` |

Pass 必須在 process 無 Teralion credential 且 network policy 為 disabled 時
完成。讀取 developer machine 的 `raw/` 不算離線 fixture acceptance。

## 5. Requirement coverage

| Requirement | M1 scope | Acceptance criteria |
| --- | --- | --- |
| `REPLAY-01` | `QuoteSnapshot`／`TradeBatch` 與 source mapping | M1-AC-01、M1-AC-02、M1-AC-08、M1-AC-09 |
| `REPLAY-02` | deterministic ordering | M1-AC-03、M1-AC-04 |
| `REPLAY-03` | snapshot MarketState | M1-AC-02、M1-AC-05 |
| `REPLAY-04` | event → state → strategy 順序 | M1-AC-06 |
| `REPLAY-06` | strict errors | M1-AC-07、M1-AC-08、M1-AC-09 |
| `STRAT-01` | 唯讀 ExampleStrategy | M1-AC-06 |
| `OPS-02`（部分） | versions、counts、warnings、checksums | M1-AC-09、M1-AC-10 |
| `NFR-01` | repeated-run equality | M1-AC-03、M1-AC-06 |
| `NFR-03`（部分） | versioning、secret safety | M1-AC-01、M1-AC-08、M1-AC-10 |

M1 不宣稱完成 `DATA-01` 至 `DATA-04`、`SIM-01`、`SIM-02` 或完整
`OPS-01`／`OPS-02`。

## 6. Required acceptance artifact set

成功的 M1 run 必須提供：

```text
acceptance-report.yaml
fixture-metadata.yaml
fixture-set.sha256
normalized-events.bin
event-stream.blake3
final-state.blake3
strategy-output.bin
strategy-output.blake3
warnings.yaml
run-summary.yaml
test-results/
```

`acceptance-report.yaml` 至少包含：

```text
acceptance_contract_version
verification_plan_version
status
git_commit
fixture_identity
fixture_checksum
strategy_identity
all design versions
rust_toolchain
build profiles
network policy
test ID -> status/evidence mapping
artifact checksums
approver
approved_at
```

approved fixture implementation 已在
`fixtures/teralion/twse/2330/2026-07-27/golden/` 建立首版 expected values：

| Artifact | Checksum |
| --- | --- |
| fixture set（SHA-256） | `5292ef24885c95c9402988423679e6b6381348cd09bb774d8489f08e9aa11ed1` |
| normalized events（BLAKE3-256） | `7e37ff0ad4a8b15b4c569b295c0f03f26bb6c0f32db1493edac71620e85a28df` |
| event stream（BLAKE3-256） | `0cecf1f6c3c6a8422955183fe383e787612efee3a4c4a7961d7faa6ee9e1de56` |
| final state（BLAKE3-256） | `02f256fc1007ce41e56200a4f82fc0f0cb504ee29afdf7262307a232862e7ea0` |
| strategy output（BLAKE3-256） | `763a5cb305a7ebbe86ea463e4091e90346421273e61b2f40f0c8ba4247690917` |

大型 `normalized-events.bin` 與 `strategy-output.bin` 由 `osmium replay` 在
acceptance run 期間從 committed fixture 產生，不提交為第二份 source data；
測試直接比較其完整 bytes，並以表列 checksum 固定 expected result。執行方式與
完整輸出集合見 [CLI 操作契約](../operations/cli.md)。

## 7. Failure policy

- 任一 required test `Failed`，M1 overall 為 `Failed`。
- 任一 required gate `Blocked`，M1 overall 為 `Blocked`。
- 任一 required test `NotRun`，M1 不可為 `Passed`。
- unexpected warning 使相關 criterion `Failed`。
- test panic、timeout、process abort 或缺少 evidence 視為 `Failed`，不是 skip。
- 只有 M1 明確排除項目可用 `NotApplicable`。

失敗後保留已產生的 diagnostics，但 artifact manifest 必須標示
`run_outcome = Failed`，不能放進 success golden directory。

## 8. Acceptance procedure

1. Authorized approver 關閉 fixture redistribution gate。
2. 依 provenance selectors 產生 minimal fixture 與 metadata。
3. 執行 fixture shard／set checksums 與 secret scan。
4. 在 debug profile 跑全部 workspace tests。
5. 在 release profile 重跑。
6. 在 network-disabled、no-key environment 跑 end-to-end suite。
7. 比對全部 goldens 與 repeated-run checksums。
8. 產生 machine-readable acceptance report。
9. review M1-AC-01 至 M1-AC-10 evidence。
10. 只有全部通過後，更新 traceability 的 `verification_evidence`。

## 9. Current paperwork result

截至 2026-07-31：

- fixture approval、implementation 與 compact golden gates 已完成。
- `cargo fmt --check`、workspace Clippy、debug 及 release workspace tests 已
  成功；debug／release 各為 105 tests／43 suites。
- vertical slice 已驗證 73,795 events、73,795 callbacks、147,590 strategy
  output records、0 warnings；同一 input 共 10 runs 及 3 個固定 shuffle seeds
  的 normalized event bytes、event／state checksums 與 strategy output bytes
  完全相同。
- 移除 `TERALION_API_KEY` 後，以 `sandbox-exec` 的 `deny network*` policy 連續執行
  兩次 release replay；完整 artifact bytes 相同。
- `osmium replay` 已能以 staging + atomic rename 產生完整 replay artifact set，
  並拒絕覆寫既有輸出目錄。
- fixture forbidden-field scan findings 為零。
- M1-T001 至 M1-T054 中 catalog 定義的 tests 全部 `Passed`；M1-AC-01 至
  M1-AC-10 及 M1 overall 均為 `Passed`。

正式可檢查證據記錄於
[`formal-2026-07-31/acceptance-report.yaml`](evidence/m1/formal-2026-07-31/acceptance-report.yaml)；
既有 `local-2026-07-31.yaml` 與 `cli-local-2026-07-31.yaml` 保留為較早的
readiness evidence，不回寫其歷史結果。
