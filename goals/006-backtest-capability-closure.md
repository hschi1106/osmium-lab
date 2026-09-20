# 006：建立並關閉完整回測 capability matrix

Status: done
Depends on: 005-close-market-background-gap.md

## 目標

在後續大量精簡 execution / accounting 前，建立 functional firewall：

> 修訂後產品需求聲稱支援的每一項 backtesting capability，都必須有 production path、明確 contract 與 regression evidence。

本 goal 允許修正**產品範圍內**真正缺失的能力；不把 cleanup 變成新交易所功能專案。

## 先建立正式 capability matrix

優先更新 `docs/product-requirements.md` + `docs/traceability.yaml`；若已能清楚表達，不另建大文件。

至少涵蓋：

### Source / replay

- provider-neutral verified source
- cache prepare / reuse / rebuild
- deterministic multi-stream replay
- multi-instrument
- multi-day / trading-date handling
- firm / indicative isolation
- no-look-ahead / visibility

### Markets / instruments

依產品需求逐項：

- TWSE equity
- TWSE warrant
- TPEx equity
- TPEx warrant
- TAIFEX future
- TAIFEX option

若某項其實只有 normalization、沒有完整 execution/accounting，必須明確揭露並決定：

- 補齊到既有 product claim
- 或修正 product claim

不能文件說完整、code 只走半條路。

### Market states

- continuous
- opening auction
- closing auction
- delayed opening/closing
- periodic/disposal
- volatility interruption
- Unknown / degraded evidence

### Strategy

- initialize / event / feedback / finalize
- explicit universe / sessions
- timer
- scheduled request
- deterministic outputs
- callback transaction / failure behavior
- no network / wall clock / future data

### Execution

以產品目前支援為準：

- market ROD
- limit ROD
- partial fill
- quantity evidence
- displayed depth
- subsequent-event causality
- slippage
- market-data latency
- order latency
- scheduled activation / expiry
- visible-depth policies
- auction cross policy
- cancellation / session end / run end
- stability restrictions

**不要因「完整」自行加入 IOC/FOK 或所有 exchange order types，除非先修改產品需求成新 scope。**

### Accounting / economics

依產品目前設計：

- cash
- position
- realized P&L
- unrealized P&L / marking
- fee
- tax
- day-trade tax adjustment
- cash charge
- quantity unit
- multiplier
- currency
- equity accounting
- futures accounting
- option premium accounting
- reconciliation
- per-fill cost lineage

### Artifacts

- immutable publication
- effective config checksum
- plan identity
- source/cache lineage
- orders / fills / cash charges
- positions / performance
- event/final-state checksums
- inspect

## 測試策略

不是每格都建新 integration test。

優先：

1. 指向既有 adequate test。
2. 補 table-driven unit / integration case。
3. 只對真正跨邊界 capability 加 end-to-end case。

建立少量 representative end-to-end scenarios，至少：

- equity continuous market/limit + accounting
- auction/delayed/periodic + execution causality
- scheduled visible depth + latency
- futures economics
- options economics
- multi-instrument / multi-market run
- failure/reconciliation

所有 fixtures 必須 repo-owned synthetic / provider-neutral，除非測 provider mapping。

## 缺口處理

若找到：

```text
documented supported
but production path missing
```

必須在本 goal 內：

- 以最小方案補上
- 或若文件 claim 明顯超出實際產品意圖，修正 claim

不要留下「未來再說」後進入 Goal-007 cleanup。

## 不做

- 不重構 execution architecture；Goal-007。
- 不大改 accounting ownership；Goal-008。
- 不新增 out-of-scope exchange features。
- 不做 performance optimization。
- 不建 coverage framework / DB。

## 驗收

- [ ] capability matrix 與產品需求一致。
- [ ] 每個 in-scope capability 有 production owner。
- [ ] 每個 in-scope capability 有可定位驗證 evidence。
- [ ] equity / warrant / futures / option 的 claim 與實際 execution/accounting 一致。
- [ ] continuous / auction / scheduled path 均有 regression。
- [ ] accounting end-state reconciliation 有代表性 end-to-end evidence。
- [ ] out-of-scope 能力明確，不被「完整回測」誤解。
- [ ] 沒有 unresolved documented-vs-implementation gap。
- [ ] workspace fmt / test / clippy 通過。
- [ ] before / after Rust LOC 已記錄。

## 執行紀錄

- Baseline revision / working tree：`454eda0`；baseline working tree clean。
- Rust LOC before：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,652 blank / 273 comment / 42,241 code。
- Capability matrix summary：在 `docs/product-requirements.md` 新增 source/replay、六類 market/instrument、market state、strategy、execution、accounting/economics、artifacts 與七個 representative E2E scenario 的 production owner／evidence／scope matrix；`docs/traceability.yaml` 同步提供 machine-readable status、owner、evidence 與 out-of-scope legend。
- Gaps found：TAIFEX option 原本有 normalizer fixture 與 `OptionsV1` accounting unit test，但缺少 production `run_multi_backtest`／`MultiLedger` 的 options E2E；runner 的既有 multi-backtest regression 也只有單一 TAIFEX future stream。另發現 traceability 與 replay model 的 run config、run manifest、event schema、cache 與 market-types version 欄位落後實作。
- Gaps fixed / docs corrected：`crates/osmium-runner/src/lib.rs` 的 test stream factory 改為依 `ReplayStreamBinding` 隔離 stream，新增 TWSE equity + TAIFEX option 的 multi-market run，驗證 6 events、4 orders、4 fills、EquityV1／OptionsV1、`-102` realized P&L、shared cash `999898` 與兩個 instrument ledger；traceability contract versions 對齊 code，`docs/architecture/replay-model.md` 的 `event_schema` 修正為 7。六類商品均明確標示為 explicit generic simulation/accounting model scope，不宣稱 full exchange matching；IOC/FOK 保持 out of scope。
- Validation：`rtk cargo fmt --check` 通過；`rtk cargo test -p osmium-runner` = 16 passed（2 suites）；`rtk cargo test --workspace` = 328 passed（55 suites）；`rtk cargo clippy --workspace --all-targets --all-features -- -D warnings` 無問題；Python YAML／symbol-reference check 通過（traceability valid、55 references resolved）；`rtk git diff --check` 通過。
- Rust LOC after / delta：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,656 blank / 273 comment / 42,468 code；相對 baseline code `+227`。
- 剩餘風險：repository fixtures 仍為 synthetic 且 `complete_day: false`；warrant／futures／options 的宣稱是 provider normalization + explicit generic execution/accounting model，不包含交易所完整撮合、queue position、exercise／assignment 或其他 out-of-scope order types。完整交易日證據仍需 repository 外的 authorized release gate。
- 下一步：007-consolidate-execution-paths.md
