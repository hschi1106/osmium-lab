# 008：收斂 economics / accounting 資料流與重複模型

Status: complete
Depends on: 007-consolidate-execution-paths.md

## 目標

保留所有 Goal-006 accounting capabilities，同時讓：

```text
config/reference economics
-> validated runtime economics
-> fill cost / ledger
-> performance / artifacts
```

只做必要轉換。

移除：

- 同一概念多套 enum/struct 只為層間 forwarding
- duplicate validation
- duplicate rounding / multiplier / quantity interpretation
- accounting config 與 runtime model 間不必要 copy
- ledger / runner 的重複 reconciliation 或 marking logic

## Audit 範圍

優先：

```text
run-planner config economics types
osmium-config economics parsing/materialization
execution-sim/accounting.rs
execution-sim public accounting exports
osmium-runner ledger construction / final marking
artifacts accounting serialization
```

建立概念對照：

```text
currency
quantity unit
multiplier
accounting model
fee
tax
day-trade tax
cash charge
rounding
marking
position accounting
```

對每個概念指出 config representation、runtime owner、conversion points、duplicate validation。

## 原則

- exact arithmetic 不退化成 float。
- config parse type 與 runtime type 可以不同，但只有責任真的不同時。
- serialization DTO 不滲入 ledger。
- ledger 不猜 instrument metadata。
- runner 不重做 accounting formula。
- day-trade tax adjustment / negative delta 保留。
- cash charge callback transaction 保留。
- futures/options economics 不被 equity path 強行泛化。

## 可能精簡

只有有證據才做：

- config enum 只轉 runtime enum 一次
- 合併相同 ChargeSides / RoundingPolicy 定義
- InstrumentEconomics 成單一 validated runtime value
- 刪 forwarding builder/wrapper
- final marking 唯一化
- 純 legacy accounting 留 Goal-010 處理

## 不做

- 不新增 portfolio/risk/margin engine。
- 不新增 accounting plugin system。
- 不改 formula 換 LOC。
- 不把不同商品 accounting 模型錯誤合併。

## 必要 regression

- equity fee/tax/cash/position/P&L
- day-trade tax recomputation
- futures multiplier/accounting
- option premium/multiplier
- quantity unit
- cash charge transaction
- partial fills
- multi-instrument ledger
- final mark
- reconciliation failure
- deterministic accounting artifacts

## LOC

記 total、execution-sim、run-planner、osmium-config。

## 驗收

- [ ] 每個 economics 概念有單一清楚 runtime owner。
- [ ] config -> runtime 轉換最小且可追蹤。
- [ ] duplicate accounting formula/validation 已刪或有不同語義理由。
- [ ] runner 不重做 ledger formula。
- [ ] Goal-006 accounting matrix 全通過。
- [ ] exact arithmetic/reconciliation 保留。
- [ ] 無新 accounting framework。
- [ ] fmt/test/clippy 通過。
- [ ] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：`2509bfb`；baseline working tree clean。
- Rust LOC before：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,657 blank / 273 comment / 42,450 code。
- Duplicate concepts：planner 的 `InstrumentEconomicsConfig` 與 `InstrumentReferenceConfig` 都驗證 quantity unit／units／multiplier／provenance；`EquityV1` 與 `OptionsV1` 的 cash notional formula 完全相同。`ChargeConfig`／`ChargeModel`、`DayTradeTaxConfig`／`DayTradeTaxModel` 與 config/runtime economics 則分屬不同 crate contract：前者保存 provenance／eligibility，後者做 exact calculation 與 defensive validation，不能以 forwarding enum 硬合併。
- Removed / retained models：`crates/run-planner/src/config.rs` 新增單一 `valid_economics_terms`，由 reference 與 instrument economics validation 共用；`crates/execution-sim/src/accounting.rs` 將 equity/options 相同 cash settlement branch 合併，仍保留不同 `AccountingModel` identity 與獨立 futures branch。新增 `docs/architecture/execution-model.md#5.1` 概念對照，記錄 currency、quantity、multiplier、charges、day-trade、marking、cash charge 的唯一 runtime owner 與一次 conversion boundary。
- Validation：`rtk cargo fmt --check` 通過；focused `rtk cargo test -p run-planner -p osmium-config -p execution-sim` = 93 passed（8 suites）；`rtk cargo test --workspace` = 328 passed（55 suites）；`rtk cargo clippy --workspace --all-targets --all-features -- -D warnings` 無問題；`rtk git diff --check` 通過。
- Rust LOC after / delta：114 files / 3,658 blank / 273 comment / 42,461 code；相對 baseline code `+11`。增加來自共用 economics-term validator 的顯式責任邊界；同時刪除 5 行重複 accounting formula，runtime behaviour 與 exact arithmetic 未變。
- execution-sim / run-planner / osmium-config LOC after：execution-sim = 4 files / 410 blank / 21 comment / 5,739 code（`-4`）；run-planner = 9 files / 366 blank / 6 comment / 3,594 code（`+15`）；osmium-config = 1 file / 130 blank / 1 comment / 1,702 code（`+0`）。
- Breaking changes：無 public schema／artifact breaking change；`SourceUnit` 或空白 provenance 的 economics reference 現在經共用 validator 明確拒絕，與既有 effective economics contract 一致。
- 剩餘風險：目前 `Currency` 僅支援 TWD，runtime ledger 以 exact decimal atoms 計算且不提供跨幣別換算；完整 portfolio／margin／options exercise-assignment 仍不在 scope。config/runtime charge 與 economics type 仍各自存在，但 conversion 僅在 composition root 發生，且不同責任已在 execution model 文件列明。
- 下一步：009-simplify-config-planner-runner.md
