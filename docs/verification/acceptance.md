# M1 Acceptance Contract

## 1. 文件目的

本文件把 M1 acceptance criteria 映射至穩定 test IDs、必要 evidence 與 pass/fail
規則。它同時記錄目前 paperwork 狀態，但不把尚未實作或尚未執行的測試標成
通過。

```text
acceptance_contract_version = 1
scope                       = M1
current_overall_status      = Blocked
```

目前 blocker 是 `TERALION_FIXTURE_REDISTRIBUTION_APPROVAL`；詳見
[Fixture provenance](fixture-provenance.md)。其餘 design 與 verification
contract 已可作為最小實作的依據。

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
| `GATE-FIXTURE-01` redistribution／commit approval | `Blocked` | authorized approver、approval date、contract／permission reference |
| `GATE-FIXTURE-02` fixture metadata 與 checksum | `Blocked` | 需在 approval 後產生 exact fixture bytes |
| `GATE-SECRET-01` secret scan | `NotRun` | `M1-T002` report |
| `GATE-SPEC-01` M1 design versions fixed | `Passed` | market-types 1、market-state 1、replay-engine 1、strategy-api 1 |
| `GATE-BUILD-01` offline Rust workspace build | `NotRun` | successful workspace test report |

`GATE-SPEC-01` 只代表 paper contract 已固定，不代表 implementation 符合。

## 4. Acceptance mapping

### M1-AC-01：合法且可回溯的 real fixture

**要求：** 使用獲准 commit／再散布的實際 Teralion TWSE 2330
`STOCK_SNAPSHOT` fixture，並能回溯 source acquisition。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T001`, `M1-T002`, `M1-T003` | approval reference、fixture metadata、SHA-256、normalizer report、secret scan | `Blocked` |

Pass 必須同時滿足：

- provenance approval 欄位完整。
- fixture bytes checksum 與 metadata 一致。
- 每筆 record 有 source selector。
- 無 secret。
- 每筆成功產生一個 accepted `QuoteSnapshot`。

### M1-AC-02：單一 tick 的 atomic market observation

**要求：** 一個 `STOCK_SNAPSHOT` 的完整 book、deal、cumulative volume 與 flags
在同一 `QuoteSnapshot` 中處理，不拆成不同 replay time。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T004`, `M1-T005`, `M1-T006`, `M1-T012`, `M1-T016` | mapping assertions、canonical event golden | `NotRun` |

Pass 必須證明至少兩個 match times，且有 book 與 cumulative volume 變化；fixture
若提供 deal，deal mapping 也必須驗證。raw flags 必須保留。

### M1-AC-03：輸入擾動後仍 deterministic

**要求：** fixture order 打亂與 repeated replay 不改變 event stream 或 final
state。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T017`, `M1-T020`, `M1-T023`, `M1-T024`, `M1-T034`, `M1-T051` | seeds、10-run checksums、event／state goldens | `NotRun` |

Pass 是 normalized event bytes、ordered event checksum、final-state checksum 與
strategy output bytes 全部相同；只有 summary 文字相同不算。

### M1-AC-04：相同 `match_time` 的 deterministic tie-break

**要求：** 相同 `match_time` 的 occurrence 依 `ordering_rule_version = 2`
排列，duplicate 不被 collapse。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T021`, `M1-T022` | real same-time selectors、synthetic branch matrix、ordered occurrence list | `NotRun` |

Pass report 必須分開標示 real fixture 與 synthetic coverage。

### M1-AC-05：MarketState replacement semantics

**要求：** 完整 snapshot 取代舊 book，state version 每 accepted event 恰增一次；
reducer error 不發布 partial state。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T030`–`M1-T035` | reducer assertions、final-state golden、failure test | `NotRun` |

### M1-AC-06：Strategy 看到 post-event state 且無前視

**要求：** 每個 accepted event 對應一次 callback；callback 只讀 event 後 state，
不能取得 next event 或 mutable state。

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T040`–`M1-T049` | integration report、compile-fail tests、strategy output golden | `NotRun` |

Pass 需要 runtime ordering evidence 與 compile-time API evidence。M1 發出 order
intent 必須得到 `CapabilityUnavailable`，不能靜默接受。

### M1-AC-07：invalid `match_time` strict failure

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T010`, `M1-T011` | exact-time cases、stable error category | `NotRun` |

invalid offset、invalid date、precision loss 或 overflow 必須拒絕，不可修正、
round 或改用 `received_at`。

### M1-AC-08：unknown format strict failure

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T013`, `M1-T014`, `M1-T025` | derived-negative cases、unsupported version result | `NotRun` |

unknown format／version 不得以 generic event 進入 timeline。

### M1-AC-09：unknown flag 保留且可診斷

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T006`, `M1-T015`, `M1-T054` | raw bits assertion、ordered warning、run summary | `NotRun` |

unknown bit 不得被丟棄或自行命名；若 interface policy 允許 accepted event，必須
保留 raw bits 並產生 stable warning。

### M1-AC-10：完全離線且不需 API key

| Test IDs | Required evidence | Status |
| --- | --- | --- |
| `M1-T050`, `M1-T052`, `M1-T053`, `M1-T054` | network-disabled CI report、sanitized environment、goldens、run summary | `NotRun` |

Pass 必須在 process 無 Teralion credential 且 network policy 為 disabled 時
完成。讀取 developer machine 的 `raw/` 不算離線 fixture acceptance。

## 5. Requirement coverage

| Requirement | M1 scope | Acceptance criteria |
| --- | --- | --- |
| `REPLAY-01` | `QuoteSnapshot` 與 source mapping | M1-AC-01、M1-AC-02、M1-AC-08、M1-AC-09 |
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
fixture.sha256
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

目前沒有任何 golden checksum 可填。第一次 approved fixture implementation
完成後才建立 expected values；本文件不預先捏造。

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
3. 執行 fixture checksum 與 secret scan。
4. 在 debug profile 跑全部 workspace tests。
5. 在 release profile 重跑。
6. 在 network-disabled、no-key environment 跑 end-to-end suite。
7. 比對全部 goldens 與 repeated-run checksums。
8. 產生 machine-readable acceptance report。
9. review M1-AC-01 至 M1-AC-10 evidence。
10. 只有全部通過後，更新 traceability 的 `verification_evidence`。

## 9. Current paperwork result

截至 2026-07-30：

- M1-AC-01：`Blocked`，等待明確的 Teralion fixture commit／redistribution
  approval。
- M1-AC-02 至 M1-AC-10：`NotRun`，因 implementation 與 tests 尚未開始。
- M1 overall：`Blocked`。

這個 status 只描述 acceptance readiness；不阻止先實作不含受限 payload 的
market types、normalizer interfaces、reducer、replayer 與 synthetic tests。
