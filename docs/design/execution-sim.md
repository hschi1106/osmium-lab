# Execution Simulation 設計

## 1. 文件目的

本文件定義 M2 的 order intent、validation、pending order、保守 fill、fee／tax、
position accounting、P&L、feedback、reconciliation 與 canonical artifacts。

```text
execution_sim_version       = 1
order_schema_version        = 1
fill_model_version          = 1
ledger_schema_version       = 2
position_accounting_version = 1
canonical_order_version     = 1
canonical_fill_version      = 1
canonical_ledger_version    = 2
result_schema_version       = 1
```

本文固定 logical contract 與 observable behavior，不固定 crate、module、trait 名稱或
內部 collection。Rust 實作 types 在 M2 step 2/8 定義，但不得使用 generic JSON、
binary float、unordered iteration 或 serializer default 取代本文 contract。

依據：

- [產品需求](../product-requirements.md)
- [Simulation requirements](../requirements/simulation.md)
- [M2 offline backtest](../increments/M2-offline-backtest.md)
- [Strategy API](strategy-api.md)
- [MarketState](market-state.md)
- [TradingContext ADR](../architecture/decisions/0004-trading-context-and-eligibility.md)
- [Replay Engine](replay-engine.md)

## 2. 模型能力與限制

M2 simulation：

- 接受 `Market` 與 `Limit`、`Day` order intent。
- 使用後續可觀察 top-of-book 或 trade print 判定成交。
- 支援 deterministic rejection、pending、partial fill、filled 與 end-of-run
  cancellation。
- 支援 configurable adverse slippage、quantity cap、fee、tax、unit size、
  multiplier、Average Cost 與 final marking。
- 使用單一 account、TWD cash 與 signed net position。

M2 不提供：

- 真實 exchange acceptance、queue position 或 matching engine。
- cancel/replace、stop、IOC、FOK、iceberg 或 order expiry time。
- borrow、short-sale eligibility、margin、risk liquidation。
- multi-account、multi-currency、FX 或 corporate action。
- source data 不支持的 intratick ordering、hidden liquidity 或 future statistics。

`filled` 表示此版本模型在明示 evidence 下產生的估算，不表示歷史上真實委託一定成交。

## 3. Logical model

### 3.1 `OrderIntent`

```text
OrderIntent {
    instrument: InstrumentId
    side: Buy | Sell
    quantity: Quantity
    order_type:
        Market
      | Limit { limit_price: Price }
    time_in_force: Day
}
```

strategy 不能指定：

- order ID、acceptance time 或 fill time。
- market flags、matching state、phase 或 eligibility override。
- fill price、fee、tax、multiplier 或 quantity cap。
- future event ordinal、source offset 或 queue rank。

### 3.2 Intent envelope

engine 為 committed callback output 加上：

```text
IntentEnvelope {
    strategy_identity
    origin_occurrence
    origin_state_version
    output_sequence
    canonical_intent
}
```

同一 callback 的 `output_sequence` 從零開始，依 sink emission order 遞增。callback
transaction 失敗時，其 envelopes 全部不發布。

### 3.3 Order

```text
OrderRecord {
    order_id
    intent_identity
    acceptance_sequence
    instrument
    side
    original_quantity
    filled_quantity
    remaining_quantity
    order_type
    time_in_force
    origin_occurrence
    status
    rejection_reason?
    cancellation_reason?
    model_bindings
}
```

status：

```text
Rejected
Pending
PartiallyFilled
Filled
Cancelled
```

`Accepted` 是建立 `Pending` order 時的 feedback/action，不是可與 `Pending` 矛盾的
持久 status。order lifecycle 只能：

```text
intent -> Rejected
intent -> Pending
Pending -> PartiallyFilled -> PartiallyFilled -> Filled
Pending -> Filled
Pending -> Cancelled(EndOfRun)
PartiallyFilled -> Cancelled(EndOfRun)
```

terminal order 不再進 fill evaluation。M2 不支援從 terminal state 返回 pending。

### 3.4 Fill

```text
FillRecord {
    fill_id
    order_id
    triggering_occurrence
    fill_sequence
    evidence
    evidence_price
    slippage
    fill_price
    fill_quantity
    economic_quantity
    gross_notional
    fee
    tax
    cash_effect
    model_bindings
}
```

`evidence` 必須指出 event fingerprint、event kind、price source、quantity source 及
eligibility decision identity。fill record 的數值必須足以獨立重算 notional、
fee、tax 與 cash effect。

## 4. Identity

identity 只使用 versioned canonical bytes：

```text
intent_identity =
  BLAKE3-256(
    strategy identity
    + origin EventOccurrence
    + output_sequence
    + canonical OrderIntent
    + order_schema_version
  )

order_id =
  BLAKE3-256(intent_identity + execution_sim_version)

fill_id =
  BLAKE3-256(
    order_id
    + triggering EventOccurrence
    + fill_sequence
    + canonical fill evidence
    + fill_model identity
  )
```

`acceptance_sequence` 是 deterministic run-local order，不取代 content identity。
wall clock、thread ID、pointer、random UUID、filesystem path 或 source JSON offset
不得參與。

collision 或同一 identity 對應不同 canonical bytes 是 fatal invariant failure。

## 5. Callback 與 event transaction

每個 accepted `DomainEvent` 的 observable 順序固定：

```text
1. Replay Engine 選出下一個 occurrence
2. commit replay clock、MarketState 與 TradingContext
3. Strategy.on_event
4. commit callback output batch
5. validate current callback intents；建立 Rejected 或 Pending records
6. evaluate 此 occurrence 前已 pending 的 orders
7. atomic commit fills、cash、positions、P&L 與 accounting trace
8. Strategy.on_feedback
9. commit feedback callback output batch
```

核心規則：

- 步驟 5 新建的 order 不參與同一 occurrence 的步驟 6。
- 下一 occurrence 即使 `match_time` 相同，只要完整 `OrderingKey` 嚴格較後，才是
  subsequent occurrence。
- `on_feedback` 產生的新 intent，origin 為該 feedback occurrence，最早由再下一個
  eligible occurrence 評估。
- strategy callback failure 丟棄當前 output batch；已 commit state 不 rollback，run
  failed。
- fill/accounting transaction failure 不發布 partial ledger state，且不呼叫該
  transaction 的 success feedback。

## 6. Intent validation

validation 按下列順序執行，第一個失敗 reason 成為 stable rejection：

| 順序 | Code | 條件 |
| --- | --- | --- |
| 1 | `UnknownStrategyOrigin` | strategy/origin/output sequence 無法關聯 current callback |
| 2 | `InstrumentOutsidePlan` | instrument 不在 frozen execution plan |
| 3 | `InstrumentOutsideStrategyUniverse` | instrument 不在 strategy declaration |
| 4 | `UnsupportedSide` | side 非 `Buy`／`Sell` |
| 5 | `UnsupportedOrderType` | 非 M2 `Market`／`Limit` |
| 6 | `UnsupportedTimeInForce` | 非 `Day` |
| 7 | `QuantityNotPositive` | quantity 為零或負值 |
| 8 | `QuantityUnitMismatch` | quantity unit 與 instrument economics 不相容 |
| 9 | `MissingLimitPrice` | Limit 未提供 price |
| 10 | `InvalidLimitPrice` | limit 非 exact positive legal price |
| 11 | `UnexpectedLimitPrice` | Market 攜帶 limit |
| 12 | `NewOrderEntryBlocked` | origin `TradingContext`/phase 不允許此 intent |
| 13 | `DuplicateIdentity` | identity 已存在或 canonical collision |

rejection：

- 建立 immutable `Rejected` order record。
- quantity/price 不得 clamp、round-to-fit 或改成另一 order type。
- 產生 `Rejected(reason)` feedback。
- 不進 pending set，不改 cash/position。

M2 不以可用現金或 position 作 pre-trade risk rejection；signed position 是 accounting
能力。若未來加入 risk model，必須有新版本、明確 validation order 及 reason。

instrument economics、currency、model 或 rounding 缺少時，preflight 必須在第一個
event 前使整個 run 失敗；這不是某一筆 intent 的合法 rejection branch。

## 7. Trading eligibility

fill 必須同時通過：

```text
phase gate
+ TradingContext matching gate
+ instrument/occurrence gate
+ selected evidence model gate
+ order price gate
+ quantity gate
```

一律不可 fill：

- current occurrence 等於 order origin occurrence。
- current instrument 不同。
- phase 為 `CoolDown`。
- matching 為 `Indicative(...)` 或 `Unknown`。
- pre-open trial、pre-close trial、delayed open/close、緩跌／緩漲。
- event 缺少 selected model 的 current price/quantity evidence。
- evidence 是 stale MarketState carry，而非 current occurrence observation。
- adverse slippage 後 limit 被違反。

`WarmUp` 可以接受新 limit order intent，但不代表開盤前即可 fill。是否具有 fill
eligibility 仍由 current `TradingContext` 與 market rule 決定。M2 TWSE trial/
indicative observations 不可 fill。

exact close `C` 屬 `Active`。closing result occurrence 可以評估更早 pending order；
同一 closing occurrence 產生的新 intent 不可用它成交。`CoolDown` 只處理來源 final
observation與回饋，不接受新 order/fill，也不合成 event。

## 8. Fill evidence models

### 8.1 `TopOfBookV1`

| Side | Current price evidence | Current quantity evidence |
| --- | --- | --- |
| Buy | current occurrence 明確提供的 best ask | `Unlimited` 或該 ask level displayed quantity |
| Sell | current occurrence 明確提供的 best bid | `Unlimited` 或該 bid level displayed quantity |

quote event 必須在 current occurrence 原子 payload 中觀察到該 side level。
MarketState 中沿用自先前 event 的 book 不是 current evidence。empty/missing side 不
fill。

### 8.2 `TradePrintV1`

current occurrence 的 ordered trade print 是 price evidence。quantity policy：

- `Unlimited`：不以 print quantity 限制。
- `Observed`：以該 print quantity 限制。

`TradeBatch` 有多筆 trade 時，依 event 已保存 source order逐筆建立 evidence slice。
不得先用 batch high/low 或 total volume 對每張 order提供重複 capacity。

### 8.3 Market order

使用第一個 subsequent eligible occurrence 的合法 selected evidence。若 occurrence
沒有 evidence，order 保持 pending；不能沿用上一價或偷看下一價。

### 8.4 Limit order

未套 slippage 前：

- buy：`evidence_price <= limit_price`。
- sell：`evidence_price >= limit_price`。

price improvement 使用 actual evidence price。套 adverse slippage 後：

- buy final price 仍必須 `<= limit_price`。
- sell final price 仍必須 `>= limit_price`。

超出 limit 時該 occurrence 不 fill，order 保持 pending。不得 clamp 成 limit。

### 8.5 Evidence consumption

每個 occurrence 為每個 instrument/evidence slice 建立單一 capacity bucket。只有
`Observed` quantity mode 消耗 bucket；`Unlimited` 使用不同 model identity，並在
result 明示沒有市場量限制。

同一 trade/quote capacity 不能：

- 被多張 order 各自完整使用。
- 因 partial fill retry 在同一 occurrence 再使用。
- 在不同 evidence mode 間無記錄地共用或重置。

## 9. Quantity allocation

pending candidates 先以 `acceptance_sequence`，再以 `order_id` canonical bytes
排序。對每個 evidence slice：

```text
allocatable =
  min(order.remaining_quantity, evidence.remaining_capacity)
```

若 quantity unit 無法 exact 換算，該 order 在 replay 前應因 economics/config
失敗，不得在 fill 時無聲捨去 remainder。

allocation 結果：

- `allocatable = 0`：不產生 fill。
- `0 < allocatable < remaining`：產生 fill，order 變 `PartiallyFilled`。
- `allocatable = remaining`：產生 fill，order 變 `Filled`。

多 worker 可以平行計算 proposal，但 commit order 及 capacity consumption 必須與
上述單執行緒 canonical algorithm 相同。

## 10. Slippage

M2 使用 `AdverseFixedDeltaV1`：

```text
Buy  fill_price = evidence_price + configured_delta
Sell fill_price = evidence_price - configured_delta
```

規則：

- delta 使用 exact `Price`/`Decimal`，不得為負。
- 結果必須為 exact positive legal price。
- rounding/tick policy 必須由 config 明示；M2 acceptance 使用可 exact 表示且不需
  額外 rounding 的 delta。
- limit order 套用後仍須通過 limit。
- delta、model identity、rounding policy 與 evidence/fill price 都寫入 fill record。

`delta = 0` 是合法明示設定，不等於缺少 slippage model。

## 11. Instrument economics、fee 與 tax

### 11.1 Economics

每個 instrument 在第一個 event 前 freeze：

```text
InstrumentEconomics {
    quantity_unit
    units_per_trading_unit
    currency
    multiplier
    value_source
    source_version
    applicable_trading_date
}
```

M2 reference：

- market/symbol：TWSE `2330`。
- currency：TWD。
- multiplier：明示 `1` 並保存 provenance。
- `units_per_trading_unit` 由 verified daily instrument 或 explicit acceptance
  config 提供，不藏在 core type default。

```text
economic_quantity =
  fill_quantity * units_per_trading_unit

gross_notional =
  fill_price * economic_quantity * multiplier
```

所有乘法必須 checked exact arithmetic；overflow 是 fatal simulation error。

### 11.2 Fee model

```text
FeeModel {
    model_id
    version
    rate
    applicable_sides
    minimum
    precision
    rounding_policy
}
```

### 11.3 Tax model

```text
TaxModel {
    model_id
    version
    rate
    applicable_sides
    minimum
    precision
    rounding_policy
}
```

M2 不 hardcode 永遠正確的法規費率。acceptance config 明示 fee/tax exact rates、
side、minimum、precision、rounding 與 provenance。

計算順序固定：

1. exact gross notional。
2. exact unrounded fee/tax。
3. 分別套用各 model precision/rounding。
4. 套 minimum policy。
5. 計算 signed cash effect。

```text
Buy  cash_effect = -(gross_notional + fee + tax)
Sell cash_effect = +(gross_notional - fee - tax)
```

fee/tax 不可先合併再 rounding。

## 12. Accounting

### 12.1 Ledger

M2 ledger 包含：

```text
Ledger {
    account_id
    currency
    initial_cash
    current_cash
    positions
    realized_pnl
    accumulated_fee
    accumulated_tax
    entries
}
```

strategy 沒有 mutable ledger handle。每個 ledger entry 必須關聯 order、fill、
occurrence、economics 及 model versions。

### 12.2 Atomic fill transaction

每筆 proposed fill 必須一次 commit：

1. validate order remaining 與 evidence capacity。
2. 計算 fill price/economic quantity/notional/fee/tax/cash effect。
3. 計算新的 order quantities/status。
4. 計算新的 cash。
5. 套用 position transition及 realized P&L。
6. 建立 fill 及 ledger entries。
7. 驗證 transaction invariants。
8. 同時發布 order/fill/ledger/position 新狀態。

任一步失敗，步驟 2–8 的可見狀態全部不變。已 commit replay event/MarketState 不
rollback；run 以 simulation/accounting failure 結束。

### 12.3 `AverageCostV1`

每個 instrument position：

```text
Position {
    signed_quantity
    average_cost?
    realized_pnl
}
```

規則：

- 同方向加碼：以原 carrying notional 與新 fill notional 計算新 exact average。
- 反方向但未跨零：以原 average cost 計算 closed quantity realized P&L；剩餘部位
  保留原 average cost。
- 剛好平倉：quantity 及 cost basis 歸零／none。
- 穿越零反向：closed portion 以原 average cost realized；超過部分以本次 fill price
  建立反向 position 的新 average cost。
- fee/tax 已透過 cash effect/ledger 記錄；是否分攤至 cost basis 由
  `AverageCostV1` 固定為不併入 unit average cost，P&L summary 另列 costs。

position quantity 使用 strategy/order quantity unit；economic P&L 使用
units-per-unit 與 multiplier。

### 12.4 Marking

`LastObservableMarkV1` 在 replay 正常結束後，對每個 open position依 plan 固定優先序
選擇 replay window 內已合法 commit 的 final MarketState value。M2 reference 優先：

1. last trade price。
2. 若 config 明示允許，last valid midpoint。

不得使用 replay 結束後的 close/stats、future event、零值或 external daily close
補 mark。沒有合法 mark：

```text
unrealized_pnl = Unavailable(reason)
```

不是數值零。mark value、origin event、state version 與 policy identity進 result。

### 12.5 P&L

對 signed open position：

```text
long unrealized =
  (mark - average_cost) * economic_open_quantity * multiplier

short unrealized =
  (average_cost - mark) * economic_open_quantity_abs * multiplier
```

result 分開呈現 realized P&L、unrealized P&L、fee、tax、cash 與 equity components；
不得用一個未定義的 net figure 隱藏 fee/tax。

## 13. End of run

只有 input stream 正常驗證 EOF 後執行：

1. 對所有 `Pending`／`PartiallyFilled` Day orders，依 acceptance order產生
   `Cancelled(EndOfRun)`。
2. 發布 cancellation feedback；`finalize` 不可再產生 intent。
3. 執行 final marking。
4. reconciliation。
5. 只有 reconciliation success 才建立 complete performance/result。

stream/cache checksum failure、strategy failure 或 simulation failure 不執行正常
success finalize。可以保存 committed prefix diagnostics，但不得取消成看似正常完成
或產生完整 P&L。

## 14. Feedback

feedback logical variants：

```text
Accepted {
    occurrence
    order_id
    accepted_quantity
}

Rejected {
    occurrence
    order_id
    reason
}

PartiallyFilled {
    occurrence
    order_id
    fill_id
    fill_quantity
    remaining_quantity
    fill_price
}

Filled {
    occurrence
    order_id
    fill_id
    total_filled_quantity
    fill_price
}

Cancelled {
    occurrence_or_end_of_run
    order_id
    reason: EndOfRun
    unfilled_quantity
}
```

feedback 包含 exact model/accounting versions及必要 trace identity。顯示 message
可以改善，但 stable reason/discriminant 才參與 canonical record。

同一 occurrence 的 feedback order：

1. current intents 的 acceptance/rejection emission order。
2. older pending orders 的 canonical evaluation order。
3. 每張 order 的 fill sequence。

## 15. Reconciliation

successful result 前從 immutable records重建並驗證：

- 每張 order `sum(fill quantity) == filled_quantity <= original_quantity`。
- `remaining == original - filled`。
- terminal status 與 remaining 一致。
- 每個 observed capacity bucket 的 allocated sum 不超過 capacity。
- fill price 符合 evidence、slippage及 limit。
- economics/notional/fee/tax/cash effect 可重算。
- final cash 等於 initial cash 加全部 cash effects。
- positions、average cost、realized P&L可由 fills 重建。
- final mark 及 unrealized P&L符合 policy。
- order/fill/ledger identities唯一且引用存在。
- accumulated fee/tax等於 records sum。

任何不一致：

- `reconciliation = Failed`。
- run status = `Failed`。
- 不發布 successful performance summary。
- 保存 stable mismatch category、record identity 與 expected/actual exact values。

## 16. Canonical artifacts

orders、fills、ledger 使用各自 versioned binary stream：

```text
header {
    magic
    schema_version: u16
    record_count: u64
}
records {
    discriminant: u8
    payload_length: u32
    canonical_payload: bytes
}
```

magic：

```text
orders = ASCII "OSORDER1"
fills  = ASCII "OSFILL01"
ledger = ASCII "OSLEDGR1"
```

primitive encoding 沿用 market types：fixed-width big-endian integer、exact decimal
atoms、length-prefixed UTF-8、fixed 32-byte digest。Optional 及 enum 都使用明示
discriminant。record order 是 lifecycle commit order。

checksums：

```text
orders.blake3 = BLAKE3-256(orders.bin exact bytes)
fills.blake3  = BLAKE3-256(fills.bin exact bytes)
ledger.blake3 = BLAKE3-256(ledger.bin exact bytes)
```

`positions.yaml`／`performance.yaml` 是 machine-readable projection，必須保存 schema
version 及 underlying canonical checksums。YAML key order或 formatting 不作為
canonical identity；完整 result identity 由 versioned semantic manifest/framed
canonical components 計算。

## 17. Failure taxonomy

| Category | 例子 |
| --- | --- |
| `IntentValidation` | unsupported type、invalid quantity/price、entry blocked |
| `EvidenceUnavailable` | current occurrence 缺 selected evidence；order 保持 pending，不是 run failure |
| `ModelInvariant` | negative slippage、illegal final price、capacity overuse |
| `Arithmetic` | exact decimal overflow、非法 rounding |
| `EconomicsMissing` | unit、multiplier、currency/provenance 缺少 |
| `AccountingInvariant` | cash/position transaction 無法 commit |
| `Reconciliation` | records 重建結果不一致 |
| `VersionIncompatible` | order/fill/ledger/model schema不相容 |

合法 intent rejection 是 domain record，不是 CLI process failure。model、arithmetic、
accounting 或 reconciliation invariant failure 則使 run failed。

## 18. Verification contract

至少需要：

- validation order及所有 stable rejection reason tests。
- origin occurrence no-fill、same-match-time subsequent occurrence tests。
- WarmUp intent、trial/indicative/unknown/CoolDown no-fill tests。
- closing result 只 fill older pending order test。
- TopOfBook/TradePrint market/limit buy/sell tests。
- limit improvement、adverse slippage limit violation tests。
- observed/unlimited quantity、partial fill及 multi-order capacity tests。
- fee/tax side、minimum、precision、rounding及 exact arithmetic tests。
- unit size、multiplier、currency/provenance missing preflight tests。
- Average Cost 加碼、減碼、平倉、reversal及 signed position tests。
- mark priority/missing mark tests。
- atomic transaction rollback及 ledger corruption/reconciliation tests。
- 10 runs、input perturbation、debug/release的 order/fill/ledger/result byte identity。

穩定 test IDs 與 acceptance evidence 見
[Verification Plan](../verification/plan.md)。

## 19. Increment boundary

| Milestone | Scope |
| --- | --- |
| M2 | 單一 TWSE 2330 account、Market/Limit Day、TopOfBook/TradePrint、AverageCostV1、TWD ledger |
| M3 | TAIFEX economics、跨 segment order policy及 multi-instrument portfolio |
| M4 | market-specific fee/tax/economics extensions；不改變 no-lookahead boundary |

新增模型必須用新 identity/version與獨立 golden evidence；不得修改 V1 semantics 來
提高歷史 fill rate。
