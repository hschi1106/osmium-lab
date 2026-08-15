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

## 6. Fee 與 tax

charge model 支援：

- `configured_rate`：依設定 basis 與 rate 計算。
- `fixed_per_unit`：依 fill quantity 計算，不按 fill record 次數收取。

每個 charge 指定 sides、minimum、precision、rounding 與 provenance。所有 exact values 使用 decimal string 與 deterministic rounding。

當沖優惠稅率依同帳戶、同商品、同 trading date FIFO 配對，支援先買後賣與先賣後買；只有列為 eligible 的 quantity 使用優惠率。未通過 eligibility validation 時不自動套用。

## 7. Ledger 與績效

ledger 原子更新：

- fill quantity 與 notional。
- fee 與 tax。
- cash 與 position。
- average cost、realized P&L 與 unrealized P&L。
- instrument 與 aggregate performance。

marking 使用 plan 中版本化 policy，預設以最後可觀察 mark；midpoint fallback 只有設定允許時使用。沒有合法 mark 時保留 unknown，不以零替代。

執行結束需驗證 fill sum、position、cash、charges、P&L 與 per-instrument/aggregate ledger checksum。一致性檢查失敗時 run 標記為 failed。

## 8. 可重現與限制揭露

run identity 保存 fill model、quantity policy、allocation、latency、scheduled policy、depth、staleness、slippage、charges、accounting、marking 與 economics。相同輸入與版本必須產生相同 orders、fills、feedback 與 ledger。

回測結果是依已選模型估算，不代表真實成交保證。報告必須保留模型名稱、版本與假設，讓不同設定的結果可區分。
