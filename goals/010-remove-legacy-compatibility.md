# 010：移除不再需要的 legacy / compatibility production paths

Status: complete
Depends on: 009-simplify-config-planner-runner.md

## 目標

利用「不要求向後相容」授權，清掉仍為舊版 encode/decode/API 行為存在的 production branch。

只保留能辨識「輸入版本不支援」並明確拒絕所需的最小邊界檢查。

## Audit

全 repo 搜：

```text
legacy
deprecated
compat
v1/v2/old branch
fallback
alias
migration
old schema
old manifest
old canonical
```

已知候選重新核對：

```text
LEGACY_CANONICAL_STRATEGY_OUTPUT_VERSION
LEGACY_ACCOUNTING_VERSION
legacy feedback channels
legacy manifest/config branches
old cache/artifact readers
deprecated aliases/wrapper exports
```

## 分類

A. 舊資料仍可成功讀/寫  
→ 刪 compatibility implementation。

B. 只為清楚 unsupported-version error  
→ 保留最小 current-version check。

C. 其實是現行 semantics，只是叫 legacy  
→ 必要就改準確名稱或記理由。

D. 歷史 changelog/docs  
→ 保留歷史事實。

## 不做

- 不改現行 domain behavior。
- 不刪 current versioning/corruption validation。
- 不寫 migration。
- 不新增 compatibility layer。

## Tests

可刪只證明舊版成功 decode 的 tests。

保留/改寫：

- old version rejected
- current round-trip
- malformed/corrupt input
- deterministic current encoding

## LOC

預期下降；若沒有要說明。

## 驗收

- [x] production 無舊版成功 decode/encode path。
- [x] 無 deprecated API alias/wrapper 只為相容存在。
- [x] current version rejection 邊界清楚。
- [x] 歷史 docs 不被誤刪。
- [x] fmt/test/clippy 通過。
- [x] Goal-006 matrix 未倒退。
- [x] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：`1c3e781`；Goal 010 開始前 worktree clean。
- Rust LOC before：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,666 blank / 273 comment / 42,580 code。
- Legacy candidates：`LEGACY_CANONICAL_STRATEGY_OUTPUT_VERSION` 的 v1 fallback、control indicator 的 v2 canonical output、未使用的 `LEGACY_ACCOUNTING_VERSION` export、TWSE／TPEx context evaluator aliases，以及 config／manifest error 中把現行 unsupported-version rejection 稱為 legacy 的訊息。掃描也確認沒有可成功讀取舊 cache、source manifest、run manifest 或 canonical event 的 reader；`v1`／`v2` 的 accounting、session、execution policy 與 artifact magic 是目前 semantics，不是舊格式相容分支。
- Deleted compatibility paths：`StrategyOutput::canonical_version` 現在一律發出 `CANONICAL_STRATEGY_OUTPUT_VERSION`（v3），移除 v1／v2 encode branch 與舊常數；移除 `LEGACY_ACCOUNTING_VERSION` 及 `TwseTradingContextEvaluator`／`TpexTradingContextEvaluator` public aliases，workspace callers 改用 `MarketTradingContextEvaluator`。feedback order channel 與 scheduled execution-fill channel 保留，因兩者都是現行不同 callback semantics；只重命名 test，沒有刪現行 channel。
- Retained rejection checks：config 仍拒絕非 `RUN_CONFIG_VERSION`；cache descriptor、events header、source manifest、run manifest 與 canonical event decoder 仍只接受目前版本並拒絕 mismatch、corruption、trailing bytes；歷史 CHANGELOG、migration validation record、license/legal 與 user workflow docs 保留。
- Validation：`rtk cargo fmt --all -- --check` 通過；`rtk cargo clippy --workspace --all-targets --all-features -- -D warnings` 通過；`rtk cargo test --workspace` = 328 passed（55 suites）；focused tests = 128 passed（14 suites）；fixture generator + fixture diff、compact fixture、fixture bundle、license verifier 與 Python acceptance = 8 passed；release `osmium` build 通過。fresh temporary-root smoke 通過 config check、plan、data verify、cache prepare（reused）、replay、backtest、run（output 與 replay-only）、inspect，以及內建 example strategy backtest／inspect（1 order / 1 fill）。`rtk git diff --check` 通過，removed symbols 搜尋無 production references。
- Rust LOC after / delta：`~/cloc/cloc --include-lang=Rust crates` = 114 files / 3,665 blank / 271 comment / 42,566 code；相對 baseline code `-14`（blank `-1`、comment `-2`）。
- 剩餘風險：移除 aliases、舊 export 與 canonical output version fallback 是刻意 breaking API／artifact identity change；舊 strategy output bytes 不再由 current binary 產生，也沒有 migration reader。已發布舊 run／cache 仍應由目前版本的既有 checksum/version boundary 拒絕並以新流程重建。artifact `OSFILLS1/2`、`OSLEDGR1/2/3`、`OSORDERS1/2` 等保留，因它們描述 current control／cash-charge semantics，不是發現到的 legacy reader。
- 下一步：011-final-redundancy-cleanup.md
