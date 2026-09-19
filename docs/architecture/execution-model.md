# 模擬與帳務

## 1. 模型邊界

execution simulation 只根據 strategy intent、已提交的市場事件、更新後 `MarketState`、`TradingContext` 與 plan 中的模型設定工作。它不重建 exchange matching、逐筆委託、queue position 或 hidden liquidity。

所有 order 都需通過 instrument、universe、side、type、quantity unit、price、session 與 capability 驗證。rejected intent 會產生穩定原因，不會進入 ledger。

## 2. Subsequent-event 模型

`subsequent_event_v1` 是預設 policy。order 在 origin event callback 成功後建立，最早只能由同商品的後續 eligible event 填入。

判定原則：

- `Market`：使用後續 event 提供的可觀察價格。
- `Limit`：buy 需要可觀察價格小於等於限價；sell 需要大於等於限價。
- fill evidence 可使用 trade 或 top-of-book，依設定固定。
- quantity 可由 observed volume 或 visible quantity 限制，支援 deterministic partial fill。
- slippage 只向不利方向套用。
- 沒有合法 evidence、matching disabled、不同 instrument 或 origin event 時不成交。
- TWSE／TPEx context 若指出盤中暫緩撮合（瞬間趨漲／趨跌），不以該 event 嘗試成交；既有未完成 market ROD 依交易所規則取消，limit ROD 保留。scheduled market order 在 activation 時遇到 restricted indicative state 會失敗；已 active 的 market ROD 在 stability trigger 時取消，未 activation 的 strategy request 不視為已送到交易所。試算價量本身不作一般 fill evidence。

scheduled simulator 將最新可見交易資格與 firm depth 分開保存。`MarketStatus`／`IndicativeAuction` 也更新 matching／order-entry 限制，但不刷新 book timestamp 或補回已消耗 depth；activation 與後續撮合都使用最新可見資格。slippage 後的價格仍須符合商品 `PricePolicy`，整批 depth fills 驗證通過後才修改 simulator。

同一 event 的 order 依 acceptance sequence 等版本化 allocation 規則處理，避免重複消耗有限 evidence。

## 3. Scheduled visible-depth 模型

`scheduled_visible_depth_v1` 是 opt-in policy，區分：

```text
observation match_time
observation visible_time = match_time + market_data_latency
order activate_at
order expire_at?
```

runner 維護 control-time queue。control action 不是 `DomainEvent`，不進入 ReplayCore、不修改 MarketState 或 replay checksum，但會進入 plan、trace 與 run identity。

可使用的 snapshot 必須同時滿足：

- `visible_time <= control_time`。
- `control_time - snapshot.match_time <= max_stale_ms`。
- snapshot 是完整 book，且 matching/context 符合所選 policy。

支援的 scheduled 行為：

- activation 時依當下可見五檔 sweep。
- activation 後等待第一筆 matching-enabled 完整 book，再嘗試一次並於 expiry 取消剩餘量。
- auction order 等待第一筆正式 call-auction trade price，以 strict-cross 規則判定；價格相等時採保守未成交。

深度只使用設定的 1–5 檔。顯示量是模型 quantity cap，不代表真實可成交量或 queue。

相同 control time 的處理順序固定為：

```text
release visible observations
-> strategy timers
-> expiry / cancel
-> activation
-> fill allocation
-> accounting
-> feedback
```

每次 strategy callback 的 output 先完整驗證，再依版本化順序提交 timer、cash charge 與 order。cash charge 的時間由 runner 使用目前 `match_time` 指派；identity 由 callback origin 與 output sequence 決定。整批 charge 在新 order 資金判定前原子扣除，strategy 不能自行指定時間或直接修改 ledger。

scheduled request 的 `activate_at` 已是最終 activation time；runner 不重複套用 order latency。

## 4. Order 與 feedback

order identity 連結 strategy、origin occurrence、output sequence 與可選 client/batch id。狀態依 policy 可能包含 scheduled、active、filled、partially filled、match attempted、expired、cancelled 或 rejected。

fill 保存 order、trigger event/control action、price、quantity、slippage、evidence 與 allocation identity。simulation feedback 只在 order/fill transition 提交後傳給 strategy；feedback callback 不能改寫既有 order 或 ledger。

## 5. Instrument economics

每個 instrument 需明確提供：

- quantity unit 與 `units_per_trading_unit`。
- currency。
- multiplier。
- provenance。

equity、futures 與 options 使用分開的 accounting model。options premium 依 `price × economic quantity × multiplier` 移動 cash；futures 依其模型計算 position 與 P&L。無法確認 economics 時拒絕執行，不套用猜測 default。

### 5.1 Economics boundary

設定與 runtime 的責任保持單向轉換：

| concept | config／planner representation | runtime owner | conversion rule |
| --- | --- | --- | --- |
| currency | `Currency`／`CurrencyAmount` 與每商品 `InstrumentEconomicsConfig` | `MultiLedger` 的 exact decimal cash | 目前只接受 TWD；currency identity 留在 effective config，runtime 不假設跨幣別換算 |
| quantity unit | `InstrumentEconomicsConfig.quantity_unit` | `InstrumentLedgerConfig` 與 `MultiSimulator` | composition root 複製一次；fill quantity 必須再次通過 ledger unit check |
| units／multiplier | `units_per_trading_unit`、`multiplier` | `InstrumentEconomics` | composition root 複製一次；notional／P&L 只由 accounting formula 使用 |
| accounting model | explicit `InstrumentClass` | `AccountingModel` | CLI 只在建立 ledger 時選擇 `EquityV1`、`FuturesV1` 或 `OptionsV1`；`MultiLedger` 再驗證 market/model 相容性 |
| fee／tax／rounding | planner `ChargeConfig`（含 provenance） | `ChargeModel`／`DayTradeTaxModel` | CLI 的 `charge` 是唯一 mapping；runtime 只保留計算所需 exact fields |
| day-trade tax | per-instrument `DayTradeTaxConfig` | `DayTradeTaxModel` | eligibility 在 planner 綁定 trading dates；runtime 維持 FIFO 配對與後續 negative adjustment |
| position accounting／marking | `PositionAccountingConfig`、`MarkingPolicyConfig` | `Ledger`、runner `final_mark` | config 只選已支援版本；position formula 由 ledger 擁有，runner 只提供 final mark |
| cash charge | strategy output `CashChargeRequest` | runner → `CashChargeRecord` → `MultiLedger` | runner 只建立 identity／時間；ledger 原子驗證與扣款 |

`InstrumentReferenceConfig` 與 `InstrumentEconomicsConfig` 同時保存 contract metadata 與 economics
terms，是為了在 planner 驗證來源 reference 與執行 economics 一致；兩者不直接進入 ledger。runtime
constructor 仍做 defensive validation，避免公開 crate API 被繞過 config/planner 後留下 invalid
state。

## 6. Fee 與 tax

charge model 支援：

- `configured_rate`：依設定 basis 與 rate 計算。
- `fixed_per_unit`：依 fill quantity 計算，不按 fill record 次數收取。

每個 charge 指定 sides、minimum、precision、rounding 與 provenance。所有 exact values 使用 decimal string 與 deterministic rounding。

當沖優惠稅率依同帳戶、同商品、同 trading date FIFO 配對，支援先買後賣與先賣後買；只有列為 eligible 的 quantity 使用優惠率。未通過 eligibility validation 時不自動套用。

每筆 fill 另保存該次造成的 `fee_delta` 與 `tax_delta`，並以 instrument fill sequence 固定順序。當沖資格在後續反向成交才成立時，系統會在該筆 fill 記錄負的 `tax_delta` 返還先前多計稅額；這是 deterministic adjustment，不代表負稅率。

## 7. Ledger 與績效

ledger 原子更新：

- fill quantity 與 notional。
- fee 與 tax。
- cash 與 position。
- average cost、realized P&L 與 unrealized P&L。
- instrument 與 aggregate performance。
- 具 stable identity、category 與 reference 的通用 cash charges。

marking 使用 plan 中版本化 policy，預設以最後可觀察 mark；midpoint fallback 只有設定允許時使用。沒有合法 mark 時保留 unknown，不以零替代。

執行結束需驗證 fill sum、position、cash、charges、P&L 與 per-instrument/aggregate ledger checksum。一致性檢查失敗時 run 標記為 failed。

Scheduled backtest 會發布 `fill-costs.json` 與 `cash-charges.json`，兩者皆使用 exact decimal atoms 並附獨立 checksum。`fill-costs.json` 與 fills 必須一對一；cash charge identity 重複、筆數不一致或 checksum 損壞都不能發布 successful run。

## 8. 可重現與限制揭露

run identity 保存 fill model、quantity policy、allocation、latency、scheduled policy、depth、staleness、slippage、charges、accounting、marking 與 economics。相同輸入與版本必須產生相同 orders、fills、feedback 與 ledger。

回測結果是依已選模型估算，不代表真實成交保證。報告必須保留模型名稱、版本與假設，讓不同設定的結果可區分。
