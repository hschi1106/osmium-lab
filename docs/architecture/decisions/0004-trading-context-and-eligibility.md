# ADR-0004：以 TradingContext 分離下單、撮合與成交資格

- 狀態：Accepted
- 決策日期：2026-07-30
- 適用契約：`TradingEligibilityPolicy`
- policy version：`1`
- 主要需求：`REPLAY-03`、`REPLAY-04`、`STRAT-01`、`SIM-01`、`OPS-02`、
  `NFR-01`、`NFR-03`

## 1. Context

同一份可觀察 MarketState 不能只用一個 `is_tradeable` boolean 判斷是否「可交易」。
實際上至少有四個不同問題：

1. 目前 session phase 是否允許策略提出新 order intent。
2. 該 market／instrument 是否仍接受特定類型的新委託。
3. 交易所目前是在試算、撮合、暫緩撮合或未知狀態。
4. 某一筆 pending order 是否能以目前 event 作為 fill evidence。

這些問題的答案可以不同。例如 TWSE 開盤前試算時：

- `WarmUp` phase 允許策略預掛受支援的 limit order。
- 試算行情可以更新 MarketState 並觸發 strategy callback。
- `trial=true` 表示目前 event 是 indicative observation。
- 該 trial event 不得產生 fill。

盤中的 pre-close trial 也仍位於 `Active` phase，但同樣不是 fill evidence。
緩跌／緩漲期間可以繼續接受符合規則的委託，交易所提供 indicative matching，
卻尚未產生實際撮合結果。漲停／跌停則是價格限制，不等同於整個商品停止撮合。

目前相關輸入分散於：

- [ADR-0003](0003-session-windows-and-strategy-activation.md)的 SessionPlan 與 phase。
- market interface 解碼的 trial、auction、緩跌／緩漲及其他 annotations。
- [SIM-01](../../requirements/simulation.md)的 subsequent-event 與 fill model。

若讓 strategy、replayer 與 simulation 各自解讀這些訊號，會產生互相矛盾的
「可交易」判斷。

## 2. Decision

採用版本化 `TradingEligibilityPolicy`，在每個 accepted event 完成 MarketState
更新後，建立一份唯讀 `TradingContext`。

`TradingContext` 分開表達：

- session phase。
- 新 order entry availability。
- matching availability。
- 造成限制的穩定 reason codes。

`FillEligibility` 不放入 `TradingContext` 作為單一 market-level boolean；它必須
針對「一筆 pending order + 目前 event + TradingContext + fill model」逐次判定。

責任流程：

```text
accepted DomainEvent
-> atomically update MarketState
-> evaluate TradingContext
-> strategy callback(event, state, trading_context)
-> validate new order intents
-> evaluate previously pending orders against current event
-> emit deterministic feedback
```

同一 callback 產生的新 order 不得使用 origin event 成交，即使該 event 對較早的
pending order 是 eligible。

## 3. Responsibility boundary

### 3.1 Replayer

Replayer：

- 依 OrderingRule 選出目前 event。
- 推進 replay clock。
- 提供 materialized session segment 與 phase。
- 協調 reducer、TradingContext evaluator、strategy 及 simulation 的固定順序。

Replayer 不直接解碼 TWSE／TPEx／TAIFEX raw flags，也不決定 fill price。

### 3.2 MarketState reducer

Reducer 只保存 source-derived facts：

- 完整 book。
- trade／trade batch。
- cumulative volume。
- raw flags 與已確認 annotations。
- event／state identity。

`TradingContext` 是 execution-derived context，不是 exchange fact，因此：

- 不加入 DomainEvent。
- 不加入 canonical event bytes。
- 不作為獨立 MarketState source field。
- 不用 synthetic status event 更新。

實作可以為效能暫存 derived context，但它必須能由相同輸入與 policy version
完全重算，且不能反向修改 MarketState。

### 3.3 Market rule evaluator

Market-specific rule evaluator 將 interface 已確認的 annotations 投影成 matching
與 order-entry restrictions。它不得：

- 依 symbol pattern 猜測未定義規則。
- 以本機 wall clock 判斷恢復撮合。
- 在缺少來源 evidence 時假設 halt／interruption 已結束。
- 將漲停／跌停自動解讀成全面停止交易。

### 3.4 Strategy

Strategy 可以讀取 `TradingContext`，用來：

- 避免在已知不接受新單時產生無效 intent。
- 區分 indicative、matching、suspended 與 unknown observations。
- 記錄決策當時可見的限制。

Strategy 不得自行解碼 raw flags 取代平台 evaluator，也不是 order acceptance 或 fill
的最終權威。平台仍須驗證每個 intent。

### 3.5 Simulation

Simulation：

- 依 `TradingContext` 驗證新 order entry。
- 保存 accepted pending order。
- 對每個 subsequent event 計算 `FillEligibility`。
- 再由 fill model 檢查 price、quantity、slippage 與 allocation。

market rule 決定「目前能否撮合」；fill model 決定「此 order 是否有足夠 observable
evidence 成交」。兩者都允許才可 fill。

## 4. Logical model

本 ADR 固定 logical semantics，不固定 Rust enum layout：

```text
TradingContext {
    instrument
    event_identity
    state_version
    session_segment_id
    phase
    new_order_entry
    matching
    restriction_reasons
    eligibility_policy_version
    market_rule_name
    market_rule_version
}
```

`event_identity` 與 `state_version` 使 context 可追溯至產生它的 observation。

### 4.1 New order entry

```text
NewOrderEntryState {
    Allowed
    Restricted(allowed_order_types, reasons)
    Blocked(reason)
    Unknown(raw_or_reason)
}
```

語意：

- `Allowed`：通過 phase／market baseline；仍需驗證 order type、price、quantity、
  universe 與 instrument metadata。
- `Restricted`：只允許明確列出的 order types 或 actions。
- `Blocked`：新 order intent 必須 deterministic reject。
- `Unknown`：default conservative policy 拒絕新 order，不猜測為 allowed。

這個 state 只處理「建立新 order」。cancel、replace、session-close carry 等 lifecycle
能力由 execution-sim design 另行定義，不可從 `Allowed` 自動推論。

### 4.2 Matching state

```text
MatchingState {
    Enabled(MatchingMethod)
    Indicative(IndicativeReason)
    Unknown(raw_or_reason)
}
```

`MatchingMethod` 至少可以表達 interface 已確認的：

- `CallAuction`
- `Continuous`

`IndicativeReason` 至少可以表達：

- `PreOpenTrial`
- `PreCloseTrial`
- `DelayedOpen`
- `DelayedClose`
- `VolatilityInterruptionDown`
- `VolatilityInterruptionUp`
- `UnclassifiedTrial`

`Enabled` 只表示市場規則未阻止目前 event 成為 fill evidence；它不代表 event
一定帶有可用 price／quantity，也不保證 order 可以成交。

`CoolDown` 不屬於 `MatchingState`。它是 SessionPlan 依 `match_time` 計算的
execution phase，不能製造 `Closed` matching observation。

### 4.3 Fill eligibility

```text
FillEligibility {
    Eligible(FillEvidence)
    Ineligible(FillBlockReason)
}
```

`FillEvidence` 至少關聯：

- current event identity。
- observable price source。
- 可用的 quantity source。
- matching method。
- eligibility／market-rule／fill-model versions。

`FillBlockReason` 使用穩定 machine-readable code，例如：

- `OriginEvent`
- `DifferentInstrument`
- `PhaseCoolDown`
- `IndicativeMatching(reason)`
- `UnknownMarketCondition`
- `MissingPriceEvidence`
- `MissingQuantityEvidence`
- `PriceNotMarketable`
- `UnsupportedOrderType`

localized message 不參與 deterministic identity。

## 5. Evaluation inputs and precedence

`TradingContext` evaluator 的輸入只能是 callback 當時已知的：

```text
materialized SessionSegment
current SessionPhase
current accepted DomainEvent
post-reducer MarketState
market interface mapping/rule version
TradingEligibilityPolicy version
```

不能讀取下一 event、未來 flags、日終結果或本機現在時間。

判定 precedence：

1. invalid／incompatible input 使 run 依既有 error policy 失敗，不建立 partial
   context。
2. `CoolDown` phase 一律 `new_order_entry=Blocked(PhaseCoolDown)`，且 phase gate
   不允許 fill；它不覆蓋目前 event 的 `MatchingState`，也不合成 event。
3. `WarmUp`／`Active` 只提供 phase baseline；market-specific rule 可以進一步限制。
4. pre-open trial、pre-close trial 與緩跌／緩漲映射成
   `Indicative(reason)`；unknown condition 映射成 `Unknown`。
5. matching enabled 後，仍須由 order-specific fill model 驗證 observable evidence。

session phase 不得把 market-specific restriction 放寬；market annotation 也不得把
`CoolDown` phase 變成可成交。若 `(C, C + 5m)` 沒有 source event，平台不在 `C`
或其後合成 TradingContext／callback。

## 6. Event-scoped and carried observations

Market interface 必須為每個 annotation 定義 scope：

- event-scoped marker：只描述目前 event，例如 opening／closing marker。
- carried condition：由每筆來源訊息明確重申，或依 interface 定義保留到明確解除。
- unknown：raw 保存，但不建立未確認的 carry rule。

平台不得因 clock 經過預期 duration 自動解除狀態。例如 TWSE 緩跌／緩漲不能只因
兩分鐘經過就恢復；必須等待來源提供可辨識的後續 matching event／annotations。

缺少新 observation 不等於 condition 已清除。若 interface 無法證實 retention
semantics，matching 使用 `Unknown` 並採 no-fill。

## 7. TWSE first-version mapping

下表是 phase baseline 與 TWSE quote annotations 的組合結果：

| Situation | New order entry | Matching | Current event fill |
| --- | --- | --- | --- |
| pre-open trial | 僅允許 policy 支援的預掛 order | `Indicative(PreOpenTrial)` | 不允許 |
| pre-open trial + `delayed_open` | 依 market rule restricted | `Indicative(DelayedOpen)` | 不允許 |
| opening result，非 trial | `Active` baseline allowed | `Enabled(CallAuction)` | 較早 pending order 可判定 |
| continuous quote | allowed | `Enabled(Continuous)` | 依 fill model 判定 |
| pre-close trial | allowed／restricted 依 order rule | `Indicative(PreCloseTrial)` | 不允許 |
| pre-close trial + `delayed_close` | 依 market rule restricted | `Indicative(DelayedClose)` | 不允許 |
| 緩跌 | supported order 可以維持／進入 pending | `Indicative(VolatilityInterruptionDown)` | 不允許 |
| 緩漲 | supported order 可以維持／進入 pending | `Indicative(VolatilityInterruptionUp)` | 不允許 |
| closing result，非 trial | 新單 blocked | `Enabled(CallAuction)` | 較早 pending order 可判定 |
| unknown／reserved market condition | default blocked | `Unknown` | 不允許 |

補充規則：

- opening／closing result 可以評估先前 pending order；同 event callback 新建的 order
  仍受 origin-event 禁止成交規則。
- closing marker 已揭示該 segment 最後撮合結果，因此 callback 產生的新單不能再進入
  該 segment。
- `instant_trend` 為緩跌／緩漲時，pending order 不因 no-fill 自動取消。
- 後續明確回到 matching enabled 後，pending order 才重新接受 fill evaluation。
- 漲停／跌停 annotations 本身不把 matching 設為 `Indicative`；fill model 仍須檢查
  side、limit price 與 observable evidence。
- `status_flags=16` 表示 continuous matching annotation，不可簡化成永久
  `MarketOpen`。
- generic `trial` 必須由 TWSE market rule 依已確認的 session subphase 分成
  `PreOpenTrial` 或 `PreCloseTrial`；無法分類時使用
  `Indicative(UnclassifiedTrial)`，仍不得 fill。

目前 2330 fixture 只有 `limit_flags=0`。緩跌／緩漲的 rule 在加入 non-zero fixture
前只能視為 specification-backed behavior，不得宣稱已有 local data evidence。

## 8. Generic fill decision

一筆 pending order 對目前 event 只有在下列條件全部成立時才能進入價格／數量模型：

1. current OrderingKey 嚴格晚於 order origin OrderingKey。
2. event instrument 等於 order instrument。
3. phase 允許既有 order fill。
4. `matching=Enabled(...)`。
5. current event 不是 indicative／unknown observation。
6. event 提供所選 fill model 所需 observable price。
7. order type、side、price、quantity 與 instrument metadata 合法。

任一條件不成立都產生 deterministic no-fill／pending decision 及 reason。是否在特定
reason 後 cancel、expire 或 carry order，由 order lifecycle policy 決定，不可由
eligibility evaluator 靜默處理。

## 9. Callback and feedback order

每個 event 的 observable sequence 固定為：

```text
1. validate event and phase context
2. atomically update MarketState
3. evaluate TradingContext
4. invoke strategy callback
5. validate and create/reject new orders from this callback
6. evaluate orders whose origin OrderingKey is earlier than current event
7. allocate fills and update accounting
8. emit order/fill/accounting feedback
```

步驟 6 可以包含 callback 前已 pending 的 orders，但不得包含步驟 5 剛建立且 origin
是目前 event 的 order。

相同 `match_time` 的後續 event 若 OrderingKey 較晚，可以成為 eligibility candidate；
這不允許 strategy 在較早 callback 看見它。

## 10. Versioning and provenance

Execution plan／run manifest 至少記錄：

```text
trading_eligibility_policy_version = 1
market_rule_name
market_rule_version
session_policy_versions
event_schema_version
fill_model_name
fill_model_version
```

order decision／fill trace 至少記錄：

- event identity 與 state version。
- phase、new-order-entry 與 matching results。
- stable restriction／block reason codes。
- 上述 policy／rule／model versions。

只改變 `TradingEligibilityPolicy` 且 domain annotations 足以重算時，不需要重新下載
source 或重建 replay cache；但 run identity、order/fill result 與 checksum 必須改變。

若 market mapping 改變 raw field interpretation、domain annotations 或 canonical
event bytes，則依 mapping／event schema compatibility 規則重建 replay cache。

## 11. Considered alternatives

### 11.1 `MarketState.is_tradeable`

拒絕。單一 boolean 無法表達「可掛單但不可撮合」、「市場可撮合但 event 缺少價格」
或「特定 order 不符合 limit」。

### 11.2 只依 SessionPhase

拒絕。`Active` 內仍可能有 pre-close trial、緩跌／緩漲或其他 indicative
condition。

### 11.3 由 strategy 解碼 raw flags

拒絕。會重複 market semantics，使不同 strategy 對同一 event 得到不同 eligibility。

### 11.4 把 TradingContext 寫入 canonical event

拒絕。它包含 session、market rule 與 simulation policy 的 derived decision，不是
source observation。

### 11.5 只在 fill model 判斷

拒絕。Strategy 仍需要知道是否值得送出 intent，且 order-entry restriction 與
price／quantity fill model 是不同責任。

### 11.6 以預定 duration 自動解除 interruption

拒絕。歷史來源可能延遲、缺漏或有例外；必須等待來源可辨識 evidence。

## 12. Verification

至少需要：

- phase × matching × new-order-entry decision table tests。
- `TradingContext` 只使用目前及過去資料的 no-lookahead test。
- 相同輸入與 policy versions 產生相同 context／reason sequence。
- pre-open trial 與 pre-close trial 都更新 state／callback，但不能 fill。
- opening event 只允許較早 pending order 進入 fill evaluation。
- continuous event 依 fill model 判定。
- pre-close trial 不 fill。
- closing marker 可以評估較早 pending order，但拒絕 callback 新單進入該 segment。
- 緩跌／緩漲期間 pending order 不成交且不自動取消。
- 沒有明確 resume evidence 時不以 clock 自動恢復。
- 漲停／跌停不被誤判成 indicative matching。
- unknown／reserved condition deterministic no-fill。
- `CoolDown` phase 不接受新 order／fill，也不合成 event／TradingContext。
- order origin event 永遠不能 fill。
- 不同 instrument event 不能填入 order。
- 改變 eligibility policy version 改變 run identity，但不使 source partition 失效。

## 13. Delivery

- M1：ExampleStrategy 不送單；可以先固定 logical context 與 deterministic
  evaluator boundary，不要求完整 order lifecycle。
- M2：以 TWSE 2330 實作 pre-open trial、pre-close trial、opening、continuous、
  closing、phase gate 與 origin-event boundary；緩跌／緩漲需補 fixture evidence。
- M3：由 TAIFEX interface 以真實 fixture 定義 regular／after-hours 的 matching
  與 order-entry rules，不共用 TWSE flags。
- M4：TPEx 加入 market rule version 與 fixture。
- M5：warrant、option 各自加入 contract rule version 與 fixtures。

## 14. Traceability

- `REPLAY-03`：MarketState facts 與 derived TradingContext 分離。
- `REPLAY-04`：event、context、callback、order 與 feedback 固定順序。
- `STRAT-01`：strategy 可讀 context，但不自行決定 acceptance／fill。
- `SIM-01`：new-order validation、subsequent-event 與 fill eligibility。
- `OPS-02`：run manifest 保存 eligibility／market-rule／fill-model provenance。
- `NFR-01`：相同輸入與版本產生 deterministic decisions。
- `NFR-03`：policy compatibility 與 cache／run invalidation boundary。
