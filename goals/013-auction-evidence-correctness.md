# 013：修正 auction partial-knowledge semantics 與 final acceptance 缺口

Status: done
Depends on: 012-final-acceptance.md
Executor: Luna / max

## 目標

修正 `codex-refactor` 在 Goal-004～012 完成後仍存在的 correctness / acceptance 缺口。

本 goal **不是重新設計 market-state architecture**，而是修正已定位的 contract violation：

1. `AuctionObservation` 目前把 `purpose`、`delayed`、`disposal` 壓成 concrete values，無法表達 partial knowledge。
2. TWSE／TPEx 無法分類的 trial 被錯誤映射成 `AuctionPurpose::Periodic`，即使程式註解本身宣稱應保持 unclassified。
3. Teralion 沒有可靠 disposal evidence 時，現行 constructor 仍把 `disposal=false` 寫成已知事實。
4. README final positioning 與實作／PRD 不一致，且仍有 stale `config_version: 2`。
5. Goals status vocabulary 與 `goals/README.md` 不一致：006–012 使用 `complete` 而非 `done`。
6. `codex-refactor` branch 尚未有 GitHub Actions run；本 goal 完成後必須建立可由 PR CI 驗證的乾淨狀態。

修正後必須維持目前已通過的 provider-neutral architecture、execution/accounting capability 與 performance。

---

## 1. 執行模型

本 goal 由 **Luna max** 單獨完成。

原因：

- 問題已經具體定位。
- 正確性 contract 可以直接驗收。
- 主要工作是修改型別／mapping／regression tests／文件與完整驗證。
- 不需要再開新的架構研究。

不得把本 goal 擴張成 Goal-014 的 redundancy refactor。

完成後：

- 一個 goal 一個 commit。
- commit message 建議：`fix(market-state): preserve partial auction evidence`
- 不 push。
- 若 necessary acceptance 無法成立，標 `blocked`，不要 commit 成完成狀態。

---

## 2. 已確認問題

以下以 `codex-refactor` head 為已知 baseline；執行前仍重新核對實際 code。

### 2.1 `AuctionObservation` 遺失 partial knowledge

目前形狀：

```rust
pub struct AuctionObservation {
    purpose: AuctionPurpose,
    delayed: bool,
    disposal: bool,
    direction: Option<VolatilityDirection>,
}
```

這無法區分：

```text
delayed=false
vs
這筆沒有 delayed evidence

disposal=false
vs
source 根本不知道 disposal

purpose=Periodic
vs
只知道目前正在 auction，但 purpose 不明
```

這違反先前已確立的：

```text
NoObservation != Unknown != false
```

### 2.2 unclassified trial 被誤標 `Periodic`

目前 TWSE / TPEx `auction_phase(...)` 的 `(false, false)` branch：

```rust
Ok(Some(AuctionObservation::periodic(false, false)))
```

但同一段註解說應：

> keep the indicative observation unclassified until a real source fixture verifies that mapping

`Periodic` 不是 unclassified。

`Periodic` 對 reducer 有真實 post-state 語義：

```text
Periodic uncross -> next Periodic auction
VolatilityInterruption uncross -> Continuous
```

因此不可把未知盤中 trial 偷偷具體化。

### 2.3 disposal unknown 被當 false

Goal-004/005 執行紀錄已承認：

> Teralion 目前沒有足夠來源證據可填 `disposal=true`

同理，沒有證據時也不能把 `disposal=false` 當已知事實。

### 2.4 stale README

目前 README 仍：

- 將 Osmium 描述成「使用 Teralion 歷史行情的平台」。
- 在「目前狀態」寫 `config_version: 2`。

實作為：

```rust
pub const RUN_CONFIG_VERSION: u16 = 3;
```

產品定位應與 PRD 一致：

> provider-neutral backtesting platform，目前內建 Teralion provider。

### 2.5 goal status vocabulary

`goals/README.md` 定義：

```text
pending
in_progress
blocked
done
```

Goal 006～012 使用：

```text
Status: complete
```

統一成 `done`。

---

## 3. 修正方向：保留小型 neutral model，不建新 framework

### 3.1 `AuctionObservation` 必須能表達 partial knowledge

不強制 exact API，但語義必須成立。

可考慮：

```rust
pub struct AuctionObservation {
    purpose: Observation<AuctionPurpose>,
    delayed: Observation<bool>,
    disposal: Observation<bool>,
    direction: Observation<VolatilityDirection>,
}
```

或使用等價的小型表示。

硬性要求：

- `purpose` 可表達 known / no observation / unknown。
- `delayed` 可表達 true / false / no observation / unknown。
- `disposal` 可表達 true / false / no observation / unknown。
- `direction` 缺失不應自動使整顆 auction observation invalid。
- 不新增第二個 market-state manager。
- 不新增 generic evidence framework / FSM / rule engine。

### 3.2 Reducer 必須做 partial merge

同一輪 auction：

```text
先前：
purpose = VolatilityInterruption
delayed = true

下一筆 trial：
purpose = NoObservation
delayed = NoObservation
```

結果應保留：

```text
purpose = VolatilityInterruption
delayed = true
```

而不是變成 `Periodic / false`。

新 auction / trading date / clear / unknown evidence 才依 contract reset / invalidate。

### 3.3 Unknown 與 NoObservation

硬性語義：

```text
NoObservation
= 本筆沒有提供該欄資訊
= 同一輪可保留前態

Unknown
= 本筆指出該欄目前不可確定／舊知識失效
= 不能保留舊 known value
```

不要把 `Unknown` 當 `NoObservation`。

### 3.4 Teralion trial mapping

若：

- opening evidence 明確 → Opening
- closing evidence 明確 → Closing
- status-only VI trigger 明確 → VolatilityInterruption
- subsequent trial 沒有重新攜帶 purpose/direction → `NoObservation`，交給 reducer 沿用前態
- trial 沒有前態、也無可證明 purpose → purpose Unknown / NoObservation（依 source contract），**不得 Periodic**
- disposal 無證據 → disposal NoObservation / Unknown，**不得 false**
- delayed bit 無有效 evidence → delayed NoObservation，**不得 false**

只有真實 source evidence 能證明 `Periodic` 時才輸出 `Periodic`。

### 3.5 Post-state

仍保持：

```text
Opening uncross             -> Continuous / known intraday regime
Closing uncross             -> Closed
VolatilityInterruption      -> Continuous
Periodic uncross            -> next Periodic auction
```

若 purpose 不明，不得猜 post-state。

必須保守維持 Unknown / unavailable / existing valid state，依現有 core contract 處理。

---

## 4. 必要 regression tests

### VI continuity

```text
status-only VI trigger
-> purpose=VolatilityInterruption

subsequent trial:
  trial=true
  no opening/closing marker
  no repeated instant-trend direction

=> reducer still knows VolatilityInterruption
=> does NOT become Periodic
```

TWSE / TPEx 各至少一個 provider mapping regression，或共用 contract + provider-specific focused case。

### Unknown disposal

```text
opening trial with no disposal source evidence
=> purpose=Opening
=> disposal is not Known(false)
```

### Unknown delayed

```text
auction observation without valid delayed evidence
=> delayed is not Known(false)
```

除非該 source record 能明確證明 false。

### NoObservation vs Unknown

至少：

```text
NoObservation keeps same-round prior value
Unknown invalidates prior value
```

### Periodic evidence

仍需 provider-neutral synthetic case：

```text
known Periodic
-> uncross
-> next Periodic
```

### Event codec / canonical

若 layout 改變：

- bump relevant versions
- current round-trip tests
- invalid encoding tests
- deterministic encoding
- stale cache/current version rejection

### Strategy / execution

確認：

- TradingContext 看見 partial auction evidence 時不自行猜 false / Periodic。
- order-entry eligibility 不因 unknown 被錯誤放寬。
- scheduled execution 同樣消費 neutral state。

---

## 5. 文件修正

### README

將產品定位改成：

> Osmium Lab 是以 Rust 建立、可接入不同歷史行情供應商的台灣市場 replay / backtesting platform；目前內建 Teralion provider。

不要再說產品本質是「使用 Teralion 歷史行情的平台」。

把 stale：

```text
config_version: 2
```

修正為：

```text
config_version: 3
```

並搜尋 current docs 中其他 stale TUI / old source / old config version。

歷史 changelog 不需改寫。

### Goal status

將 006～012：

```text
Status: complete
```

改為：

```text
Status: done
```

不改執行紀錄內容。

---

## 6. 不做

- 不進行 production LOC cleanup。
- 不重新設計 execution-sim。
- 不合併 accounting。
- 不重新整理 config/planner/runner。
- 不新增第二 provider。
- 不加入 disposition lookup service。
- 不用 wall-clock / source sparsity 猜 auction purpose。
- 不為了減少 enum variant 犧牲 Unknown semantics。

---

## 7. LOC

本 goal 是 correctness fix，不要求 production LOC 下降。

記錄：

```text
total Rust LOC before/after
production src Rust LOC before/after
test Rust LOC before/after
```

Production LOC 依 `goals/README.md` 使用 tracked + untracked current working tree Rust files 計數。

---

## 8. 完整驗證

至少：

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p teralion-provider
```

Fixtures：

```sh
python3 tools/acceptance/generate_synthetic_fixtures.py
git diff --exit-code -- fixtures
tools/acceptance/verify_compact_fixtures.sh
tools/acceptance/verify_fixture_bundle.sh \
  --bundle . \
  --manifest fixtures/smoke/manifest.yaml
python3 -m unittest discover -s tools/acceptance -p 'test_*.py'
```

Offline smoke 依 final validation docs 跑：

```text
config check
plan
data verify
cache prepare
replay
backtest
run
inspect
```

至少重跑：

- neutral replay determinism
- scheduled tests
- accounting representative tests

若 canonical / cache identity 改變，說明 checksum 差異原因；不要直接更新 expected 略過分析。

---

## 9. GitHub CI

目前 `.github/workflows/ci.yml` 不會因 feature branch push 自動執行。

本 goal 不修改 CI trigger 只為跑一次。

完成並 commit 後，使用者建立 PR 到 `main` 或手動 `workflow_dispatch` 後，CI 應能驗證此 branch。

執行紀錄明確寫：

```text
local full validation: PASS / FAIL
GitHub Actions: not run yet / PR run result
```

不可把「沒有 CI run」寫成 GitHub CI PASS。

---

## 10. 驗收

- [x] `AuctionObservation` 可表達 purpose/delayed/disposal partial knowledge。
- [x] `NoObservation` 與 `Unknown` 行為不同且有 tests。
- [x] Teralion unclassified trial 不再映成 `Periodic`。
- [x] Teralion 無 disposal evidence 時不產生 Known(false)。
- [x] Teralion 無 delayed evidence 時不產生 Known(false)。
- [x] VI trigger -> later trial 能保留 VI purpose。
- [x] 真正 Periodic semantics regression 通過。
- [x] strategy / normal / scheduled execution 不自行猜 unknown。
- [x] relevant canonical/schema/mapping versions 正確 bump。
- [x] README provider-neutral positioning 正確。
- [x] README `config_version` 與 code 一致。
- [x] Goal 006～012 status 統一為 `done`。
- [x] workspace fmt/test/clippy 全通過。
- [x] provider / fixtures / offline smoke 全通過。
- [x] production/test/total cloc 已記錄。
- [x] 一個 focused commit，未 push。
- [x] GitHub Actions 未跑時明確寫未跑，不冒充通過。

## 執行紀錄

- Baseline revision / working tree：`84fa3635019790e6246eccbd600b6d4790e6d077`；Goal 013 開始前 working tree clean，`codex-refactor` 僅比 remote 多本地 goal-definition commit，未覆寫既有使用者修改。
- Executor / reasoning：Luna max
- Total Rust LOC before：115 files / 3,674 blank / 271 comment / 42,834 code
- Production Rust LOC before：77 files / 3,108 blank / 261 comment / 34,417 code
- Tests Rust LOC before：33 files / 489 blank / 4 comment / 7,043 code
- Correctness defects confirmed：`AuctionObservation` 原本無法區分 known、未觀察與失效 unknown；TWSE／TPEx `(false, false)` trial 會錯誤成為 `Periodic`；Teralion 缺乏 delayed／disposal evidence 時被 constructor 補成 `false`；reducer、strategy 與 execution 可能因此採用過度具體的 post-state 或 eligibility。
- Contract changes：新增小型 `AuctionEvidence<T>`，逐欄支援 `Known`／`NoObservation`／`Unknown`；`NoObservation` 沿用同一輪前態、`Unknown` 使該欄失效。Indicative auction 與 market signal 在 reducer 合併 partial observation；purpose 不明的 uncross 只保留 unknown phase，不猜 post-state。TWSE／TPEx 未分類 trial 保留 unclassified，VI status-only 只填明確 purpose／direction，無證據的 delayed／disposal 保持 `NoObservation`。
- Versions/checksum changes：`MARKET_TYPES_VERSION 9 -> 10`、`EVENT_SCHEMA_VERSION 7 -> 8`、`CANONICAL_EVENT_VERSION 7 -> 8`；`MARKET_STATE_VERSION 6 -> 7`、`STATE_REDUCER_VERSION 5 -> 6`、`CANONICAL_MARKET_STATE_VERSION 6 -> 7`、`CANONICAL_FINAL_STATE_SET_VERSION 6 -> 7`；strategy `market_rule_version 2 -> 3`；TWSE quote/warrant mapping `10/6 -> 11/7`、TPEx quote/warrant mapping `7/6 -> 8/7`、TAIFEX outright/options mapping `4/3 -> 5/4`（calendar spread 未受影響）。Auction canonical layout 與 provider mapping 改變會使 event、state、cache identity／checksum 改變；舊 cache 依 descriptor version 拒絕並重建，未以更新 expected checksum 掩蓋差異。離線 smoke 產生 event checksum `b642c09945985960d20c2d1857874946a03e44868d74e04e2a8403fb6b3cf5e3`、final-state checksum `b52e79639f8762289172e42897f30773413e951d799f971c7488b7bdf44b1fe9`。
- Regression tests：新增 `NoObservation` carry／`Unknown` invalidation、VI continuity、unclassified trial、unknown uncross、partial IndicativeAuction merge、codec round-trip／invalid encoding／deterministic canonical tests；更新 TWSE／TPEx／TAIFEX fixtures、strategy、normal execution 與 scheduled execution。`cargo test --workspace --locked`：333 passed；Teralion provider：67 passed；market-types：43 passed；market-state：28 passed；strategy-api：29 passed；replay-engine：17 passed；execution-sim：50 passed；osmium-runner：16 passed。
- Full validation：PASS。`cargo fmt --all -- --check`、workspace clippy（含 `--locked`，`-D warnings`）、workspace tests、fixture generator／diff、compact／bundle verifier、acceptance Python 8 tests、license verifier、release build、offline `data verify -> cache prepare -> replay -> backtest -> inspect`、compiled strategy smoke、clean-machine archive smoke 與 `SOURCE_DATE_EPOCH=0` byte-identical reproducibility 均通過；strategy smoke 核對 `orders: 1`、`fills: 1` 與 strategy id。
- GitHub Actions：not run yet。未修改 CI trigger，也未把本地結果冒充 GitHub CI；依本 goal 要求不 push，待 PR 或 `workflow_dispatch` 驗證。
- Total Rust LOC after / delta：115 files / 3,706 blank / 278 comment / 43,420 code（blank +32、comment +7、code +586）
- Production Rust LOC after / delta：77 files / 3,131 blank / 268 comment / 34,727 code（blank +23、comment +7、code +310）
- Tests Rust LOC after / delta：33 files / 498 blank / 4 comment / 7,319 code（blank +9、comment +0、code +276）
- Breaking changes：auction observation public getter 改回傳 `AuctionEvidence`；event／state canonical frame 與版本不相容，舊 event、state、cache artifact 必須拒絕或重建；provider trial mapping 不再把無證據資料解讀成 `Periodic`／`false`。
- Remaining risks：目前仍只有 Teralion provider；來源沒有提供的 disposal／delayed evidence 仍會保持 partial，而非補值。GitHub Actions 尚未執行；Goal 014 再處理 semantic redundancy audit，不在本 goal 擴張 cleanup。
- Commit：`fix(market-state): preserve partial auction evidence`；本 goal 單一 focused commit，未 push。
- 下一步：014-semantic-redundancy-audit.md
