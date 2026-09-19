# 008：收斂 economics / accounting 資料流與重複模型

Status: pending
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

- Baseline revision / working tree：
- Rust LOC before：
- Duplicate concepts：
- Removed / retained models：
- Validation：
- Rust LOC after / delta：
- Breaking changes：
- 剩餘風險：
- 下一步：009-simplify-config-planner-runner.md
