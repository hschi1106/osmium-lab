# 007：收斂一般與 scheduled execution 的重複邏輯

Status: complete
Depends on: 006-backtest-capability-closure.md

## 目標

在 Goal-006 的 capability firewall 保護下，檢查並移除一般 execution、scheduled execution、runner orchestration 間可證明的重複 market interpretation / order lifecycle / fill logic。

最終責任應接近：

```text
runner
  = replay + strategy callbacks + control-time orchestration

execution-sim
  = order lifecycle + execution eligibility + fill allocation

accounting
  = fills/cash/position/economics
```

不要為了少 LOC 把 control time 與 market event time 混成一套。

## 主要 audit 熱點

實作前重新 cloc / inspect：

```text
crates/execution-sim/src/lib.rs
crates/execution-sim/src/scheduled.rs
crates/execution-sim/src/depth.rs
crates/osmium-runner/src/lib.rs
crates/osmium-runner/src/scheduled.rs
crates/osmium-runner/src/control.rs
crates/osmium-runner/src/visibility.rs
strategy-api order / feedback types
```

找出重複：

- matching / order-entry 判斷
- market state interpretation
- order status transitions
- cancellation
- visible depth allocation
- auction match eligibility
- fill feedback construction
- instrument/session universe checks
- repeated event-to-context conversion

只合併**語義相同**部分。

## 原則

- Goal-004 的 TradingContext / market semantics 是唯一來源。
- runner 不再自行重解 auction / provider annotations。
- simulator 不得管理 strategy lifecycle。
- scheduled control action 不寫 replay event stream。
- market-data latency / control time / expiry 仍可有專用 coordinator。
- depth sweep primitive 可共用，但不要把 market-event evidence 與 control evidence 假裝同一來源。

## 可接受結果

不強迫把 `Simulator` 與 `ScheduledDepthSimulator` 合成一個 struct。

如果兩者保留能讓語義更清楚，可以保留；但相同 order state / restriction / fill allocation 不應兩份。

同理，不強迫 `osmium-runner/src/scheduled.rs` 消失；它若仍是必要 control coordinator，可以留，但應停止擁有 execution-sim 已負責的規則。

## 刪除優先

- duplicate helper
- duplicate state enum
- duplicate eligibility branch
- pass-through wrapper
- 同結果的兩份 feedback conversion
- runner 中可直接委派 simulator 的 policy match

禁止用新 trait hierarchy 取代普通 function / enum。

## 驗收

以 Goal-006 matrix 為 regression gate，至少：

- normal market/limit
- partial fill
- subsequent-event causality
- scheduled activation / expiry
- visible depth until expiry
- auction cross
- volatility interruption restriction / cancellation
- latency
- multi-instrument allocation
- deterministic feedback order

## LOC

除 total Rust LOC，再記：

```text
execution-sim LOC before/after
osmium-runner LOC before/after
```

若 LOC 未下降，必須具體說明刪掉的 duplication 被何種必要 correctness code 抵銷。

## 驗收

- [ ] runner / simulator ownership 更清楚。
- [ ] provider/market raw interpretation 不在 execution paths 重複。
- [ ] 可證明的 order lifecycle / fill duplication 已移除。
- [ ] control time 與 replay time 仍正確分離。
- [ ] Goal-006 capability regression 全部通過。
- [ ] 沒有新增 execution framework / trait forest。
- [ ] fmt / test / clippy 通過。
- [ ] total + execution-sim + runner cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：`390b6d8`；baseline working tree clean。
- Rust LOC before：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,656 blank / 273 comment / 42,468 code。
- execution-sim LOC before：4 files / 410 blank / 21 comment / 5,750 code。
- runner LOC before：6 files / 237 blank / 0 comment / 4,778 code。
- Duplication found：一般 `Simulator::submit` 與 `ScheduledDepthSimulator::activate` 以相同規則解讀 `NewOrderEntry`；`ScheduledCoordinator::process_control` 的 Activate、visible-depth allocation、auction allocation 三個 branch 重複將新 fills 寫入 ledger、更新 open position 並排入 feedback。兩者都沒有混合 market-event evidence 與 control-time evidence，但 lifecycle sink code 確實重複。
- Consolidation：在 `crates/execution-sim/src/lib.rs` 建立單一 private `entry_allowed` policy helper，scheduled module 直接委派；在 `crates/osmium-runner/src/scheduled.rs` 建立 `settle_activations`，保留每個 branch 的 simulator operation、feedback time（auction 仍使用 `visible_at`）與 `FillTrigger` 差異，只集中 shared ledger／position／feedback plumbing。沒有加入 trait hierarchy，也沒有把一般與 scheduled simulator 合成同一 struct。
- Validation：`rtk cargo fmt --check` 通過；`rtk cargo test --workspace` = 328 passed（55 suites）；`rtk cargo clippy --workspace --all-targets --all-features -- -D warnings` 無問題；focused `rtk cargo test -p execution-sim -p osmium-runner` = 66 passed（4 suites）；`rtk git diff --check` 通過。
- Rust LOC after / delta：114 files / 3,657 blank / 273 comment / 42,450 code；相對 baseline code `-18`。
- execution-sim / runner LOC after：execution-sim = 4 files / 410 blank / 21 comment / 5,743 code（`-7`）；runner = 6 files / 238 blank / 0 comment / 4,767 code（`-11`）。
- 剩餘風險：scheduled coordinator 仍需在 visibility release 時讀取 provider-neutral `TradingContext` 的 volatility-interruption signal，才能以 control time cancel active market orders；這不是 raw provider annotation interpretation，且刻意保留以維持 market visibility 與 control scheduling 邊界。一般／scheduled order status 仍是不同模型的不同 state enum，未強行合併。
- 下一步：008-simplify-accounting-economics.md
