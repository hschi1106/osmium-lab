# 010：移除不再需要的 legacy / compatibility production paths

Status: pending
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

- [ ] production 無舊版成功 decode/encode path。
- [ ] 無 deprecated API alias/wrapper 只為相容存在。
- [ ] current version rejection 邊界清楚。
- [ ] 歷史 docs 不被誤刪。
- [ ] fmt/test/clippy 通過。
- [ ] Goal-006 matrix 未倒退。
- [ ] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- Legacy candidates：
- Deleted compatibility paths：
- Retained rejection checks：
- Validation：
- Rust LOC after / delta：
- 剩餘風險：
- 下一步：011-final-redundancy-cleanup.md
