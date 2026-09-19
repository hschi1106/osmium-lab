# 005：關閉 instrument-day market background 的 production gap

Status: done
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

- [x] 每個 background field 的必要性都有 code-path 證據。
- [x] 可從 event 推導的欄位沒有重複持久化。
- [x] 真正必要 background 使用 provider-neutral representation。
- [x] core 不查 Teralion / web / provider API。
- [x] production runner 能取得需要的值，不只是 tests。
- [x] 不新增 profile platform / DB / crawler。
- [x] disposal / periodic production path 不只存在 synthetic state test。
- [x] determinism / identity tests 通過。
- [x] workspace fmt / test / clippy 通過。
- [x] before / after Rust LOC 已記錄。

## 執行紀錄

- Baseline revision / working tree：`d83f5f4` / clean。
- Rust LOC before：`~/cloc/cloc --include-lang=Rust crates`：113 files / 3,637 blank / 270 comment / 42,008 code。
- Background fields audited：
  - `intraday_matching` 由 event `MarketSignal` 與 reducer-owned `MarketPhase` 產生；沒有 event evidence 時 context 維持 `Unknown`，不補 `Continuous`。
  - `disposal` 只存在 neutral `AuctionObservation`；目前 Teralion 沒有已驗證的處置來源欄位，不新增 instrument-day 名單，也不把處置制度重算成 fill/accounting policy。
  - auction post-state 由 reducer 依 `AuctionPurpose`／`AuctionUncross` 固定推導；order-entry 由 event signal、post-state 與 session phase 投影；fill policy 由既有 effective simulation、contract 與 economics config 提供；strategy-visible metadata 來自 event、state、context 與 session。
- Fields deleted / retained：沒有新增或持久化 `MarketBackground`。移除 `QuoteSnapshot`、`BookSnapshot`、`TradeBatch` 建構子對 `Continuous` 的隱含預設，改為 `NoObservation`；保留既有 provider-neutral contract/session/economics/simulation 設定，這些已進 effective config／execution identity。
- Production source of retained fields：TWSE／TPEx normalizer 已在 provider boundary 輸出 firm signal；TAIFEX I020／I080／I082 明確輸出 `Continuous`、I022 輸出 opening `AuctionCollecting`；`ReplayCore -> MarketState -> TradingContext -> execution` 直接消費 neutral event。reserved provider bits 轉成 `Unknown` 並保留 warning，不靜默覆寫成 `Continuous`。
- Validation：`cargo fmt --check`、`cargo test --workspace`（327 passed / 55 suites）、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 均 exit 0。新增 generic no-default-signal test；新增 Teralion provider-to-reducer/context production path tests，覆蓋 missing-signal replay、real normalizer periodic、neutral disposal periodic result、event matching 與 post-state 分離。fixture generator/diff、compact/bundle/license verifier 與 acceptance Python tests（8 passed）通過。
- Rust LOC after / delta：`~/cloc/cloc --include-lang=Rust crates`：114 files / 3,652 blank / 273 comment / 42,241 code；相較 before 為 `+1 file / +15 blank / +3 comment / +233 code`。
- Identity/version：`MARKET_TYPES_VERSION` 8→9；TAIFEX outright mapping 3→4、calendar spread 1→2、options 2→3。canonical event frame layout 未變，mapping/cache identity 會使受影響 derived cache rebuild；既有 effective config、session plan 與 accounting identity 保持 provider-neutral。
- 剩餘風險：目前 Teralion source 沒有可證明 `disposal=true` 的欄位，故 production mapping 不臆造處置名單；neutral production-path test 只驗證 core 可正確消費 `disposal=true`。來源 reserved/矛盾證據保留 `Unknown`／diagnostic，需後續 verified source evidence 才能細化。
- 下一步：006-backtest-capability-closure.md
