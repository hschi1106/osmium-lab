---
name: osmium-engineering
description: >
  Use for changing, reviewing, debugging, or testing Osmium code or
  architecture, including marketdata reading, replay, book state, matching,
  accounting, the strategy API, the runner, the CLI, versions, and tests. Not
  for running backtests with an existing build (use osmium-backtest).
---

# Osmium Engineering

Canonical docs 擁有 domain truth；code 與 tests 是 executable truth。本 skill 只提供 routing、
invariants 與 workflow。

## Ownership 與文件

```text
osmium-types    → 價格、股數、金額、時間、feed、訊息、委託與委託事件的型別
osmium-md       → parquet → Message、參考表；schema 與 manifest 檢查（docs/reference/marketdata.md）
osmium-replay   → 跨 feed 全序、事件時間線（docs/reference/replay.md 第 2、3 節）
osmium-book     → 撮合方式、交易模式、目前五檔（docs/reference/matching.md 第 4 節）
osmium-matching → 委託檢查、撮合、排隊估計、價格穩定範圍（docs/reference/matching.md 第 5–8 節）
osmium-ledger   → 現金、部位、費稅、交割、當沖、信用交易、公司行動、損益
                  （docs/reference/accounting.md；不讀檔，參考表由 osmium-md 讀取）
osmium-strategy → 策略 trait、Context、行情與帳戶視圖、參數、註冊表（docs/reference/strategy.md）
osmium-runner   → 交易日流程、延遲、設定、輸出（docs/reference/replay.md 第 4 節以後、
                  config.md、artifacts.md）
osmium-cli      → 指令與錯誤輸出；osmium-examples → 範例策略
osmium-fixture  → 測試用的合成 marketdata 樹（不屬於產品）
```

先定位 semantic owner；不要把規則移到 caller。只讀 owner 相關文件，不確定時從 `docs/README.md`
導航。架構、相依方向與不變條件在 `docs/design/architecture.md`；規則的出處在
`docs/design/research/`。

## High-risk invariants

- **不偷看未來**：撮合只用已經到達交易所的訊息；策略只看得到已經送達它的訊息（加上
  `latency.market_data`）與已經送達它的回報。帳戶視圖是回報產生當下的快照。
- **模擬委託不影響行情**：真實的 book 與成交不因模擬委託改變；成交量不得超過資料顯示的量。
- **不猜測缺值**：marketdata 沒有的事實（升降單位、每張股數、公司行動條件）不外推；依規格
  拒絕、停止，或記錄診斷後採用設定明示的近似。`unknown` 與 `false` 不同。
- **精確算術**：價格是 1e-4 元的整數，金額是整數元，比率是十進位數。不使用浮點數；每一個
  進位方式都寫在帳務規格。
- **決定性**：相同的資料、設定與版本產生位元組相同的輸出。不依賴 `HashMap` 的走訪順序、
  時間、執行緒排程。
- **只讀 marketdata**：不寫入 `md_root`；衍生資料放在別處。
- **一個時鐘**：所有事件依回放規格的時間線排序；同一時刻的順序由規格決定，不由實作細節決定。

## Version／identity

同一個版本不得代表兩套行為。每次修改會改變結果時，依 `docs/reference/versions.md` 更新對應的
常數（decoder、ordering、matching rules、ledger rules、output、策略版本），並寫入 `CHANGELOG.md`。
只改變速度、記憶體或錯誤訊息的修改不更新版本，但要證明輸出位元組相同
（`crates/osmium-runner/tests/run.rs` 的 golden、真實資料 run 的 `run.json` 的 `files`）。
除非使用者要求，不加 legacy compatibility。

## Scope 與 workflow

優先使用 concrete function／enum／struct。不要為 hypothetical future 新增 framework、trait forest、
generic adapter、compatibility layer 或 one-call-site wrapper。Abstraction 必須移除真實 duplicated
semantics 或釐清 ownership。

```text
locate owner → read contract → inspect code/tests → smallest coherent change
→ regression → identity check → focused/full validation
```

規則改變時先改規格，再改 code 與測試：撮合規則同時更新 `crates/osmium-matching/tests/` 的例子，
帳務規則同時更新 `crates/osmium-ledger/tests/ledger.rs` 的手算例子並跑 `tests/invariants.rs`。
只有 contract 改變才更新 canonical docs。

## Validation

```sh
./.agents/skills/osmium-engineering/scripts/validate.sh focused <area>
./.agents/skills/osmium-engineering/scripts/validate.sh full
```

`<area>`：`types`、`md`、`book`、`replay`、`matching`、`ledger`、`strategy`、`runner`、`cli`、
`examples`。真實資料的測試與比對見 `docs/development/validation.md`；只宣稱實際成功的 command。
