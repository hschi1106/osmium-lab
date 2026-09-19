# 005：關閉 instrument-day market background 的 production gap

Status: pending
Depends on: 004-unified-market-state.md

## 目標

Goal-004 可以用 synthetic input 告訴 core：

```text
intraday matching regime
disposal identity
auction purpose / delay
```

本 goal 要回答 production run 中這些資訊究竟從哪裡來、哪些其實不需要額外背景，並選擇最小方案。

優先目標是**刪除不必要的背景依賴**，不是新增 profile system。

## 第一原則：先證明需要哪些 background

逐項追蹤：

```text
intraday_matching
disposal
auction post-state
order-entry policy
execution fill policy
strategy-visible metadata
```

對每項回答：

1. 能否由目前/已提交的 neutral event evidence 得到？
2. 能否由既有 instrument contract / verified provider metadata 得到？
3. 是否真的影響 execution/accounting，還是只有資訊展示？
4. 缺少時是否可以正確維持 Unknown，而不阻止 unrelated replay？

如果 Goal-004 中暫時加入的 `MarketBackground` 欄位實際可被 event evidence 取代，直接刪除。

## 偏好的最小資料流

優先級：

```text
current event evidence
    ↓
existing verified instrument/day metadata
    ↓
minimal explicit initialization value（只有真的缺不可）
```

不要直接跳到：

```text
new profile file format
metadata DB
crawler
lookup service
rules DSL
provider registry
```

## Provider-neutral contract

若確實需要 initialization/background，必須是 neutral value，不帶：

```text
Teralion
API endpoint
provider field name
raw status bits
```

provider 可輸出 neutral value；core 只消費 value。

若 Teralion 現有資料無法證明某欄，回 `Unknown` / missing；不得用稀疏成交、時間間隔或 trial 頻率猜。

## Reproducibility

任何會改變：

- market state
- order eligibility
- fill
- accounting

的 initialization/background，必須進 execution identity / run reproducibility input。

不要偽裝成 market event provenance。

## Production integration

不能只有 test setter。

真正：

```text
config / provider verified metadata / existing reference
-> plan or run initialization
-> MarketState
-> TradingContext
-> execution
```

要能走通。

但若分析證明某欄不再需要，最好的 integration 是**刪掉它**。

## 不做

- 不新增 metadata service。
- 不做 provider-specific lookup in core。
- 不為 disposal 建 rule engine。
- 不把今日處置名單套歷史。
- 不新增第二個 market-state manager。
- 不自行增加產品 out-of-scope 交易制度。

## 驗收案例

至少：

- normal instrument production path
- periodic/disposal synthetic production path
- missing background 仍可 replay unrelated events
- background unknown 不默認 false / Continuous
- opening result 能正確得到 next phase
- periodic result 能正確得到 next phase
- cross-day reset
- multi-instrument isolation
- same inputs -> same run identity / result
- contradictory provider/background evidence 有明確 error / diagnostic，不 silent override

## 驗收

- [ ] 每個 background field 的必要性都有 code-path 證據。
- [ ] 可從 event 推導的欄位沒有重複持久化。
- [ ] 真正必要 background 使用 provider-neutral representation。
- [ ] core 不查 Teralion / web / provider API。
- [ ] production runner 能取得需要的值，不只是 tests。
- [ ] 不新增 profile platform / DB / crawler。
- [ ] disposal / periodic production path 不只存在 synthetic state test。
- [ ] determinism / identity tests 通過。
- [ ] workspace fmt / test / clippy 通過。
- [ ] before / after Rust LOC 已記錄。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- Background fields audited：
- Fields deleted / retained：
- Production source of retained fields：
- Validation：
- Rust LOC after / delta：
- 剩餘風險：
- 下一步：006-backtest-capability-closure.md
