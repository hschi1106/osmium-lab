# ADR-0006：以 opt-in control time 執行排程委託與可見五檔

- 狀態：Accepted
- 決策日期：2026-08-03
- 適用契約：`ScheduledVisibleDepthExecutionPolicy`
- policy version：`1`
- 主要需求：`REPLAY-04`、`STRAT-01`、`SIM-01`、`SIM-02`、`OPS-02`、
  `NFR-01`、`NFR-03`

## 1. Context

預設 `SIM-01` 讓 order 從 origin event 之後的第一個 eligible market event 開始判定
fill。這個模型保守、容易解釋，也必須繼續作為既有設定的預設語意。

部分研究需要在已知時間啟用或到期 order，並以該時間以前已對 strategy 可見的最新完整
五檔作為簡化 fill evidence。例如 execution timer 到時可能沒有新的 market event；若一律
等待下一個 event，模擬 activation time、使用的 snapshot 與實際設定會產生未被記錄的
偏移。

直接把 timer 合成 `DomainEvent` 會污染 replay event stream、MarketState version 與
checksum；讓 strategy 自行宣告 fill 則會使正式 orders、fills、ledger 與 strategy state
失去單一權威來源。

## 2. Decision

平台可以提供 opt-in、版本化的 scheduled visible-depth execution policy。既有
subsequent-event policy 保持不變；只有 execution plan 明確選擇新 policy 時才啟用。

新 policy 分離三種時間：

```text
observation match_time
observation visible_time = match_time + market_data_latency
order activation_time
```

`order_latency` 只參與建立 `activation_time`；已帶有最終 `activation_time` 的 scheduled
request 不得再次套用 latency。

runner 維護 deterministic control-time queue，將下一個 market event 與下一個 execution
control action 依時間協調。control action：

- 不是 `DomainEvent`。
- 不進入 ReplayCore。
- 不修改 MarketState 或 event/state checksum。
- 必須進入 execution trace 與 run identity。

activation 時的 fill evidence 只能來自：

```text
snapshot.visible_time <= activation_time
activation_time - snapshot.match_time <= configured_staleness
```

符合條件時，policy 可以依完整 snapshot 第一至第五檔 deterministic sweep。顯示量是模型
假設下的 quantity cap，不代表真實 queue、hidden liquidity 或保證成交。平台提供兩種明確
區分的 scheduled policy：

- `VisibleDepthAtActivationV1`：在 `activate_at` 立即使用當時最新的合格可見 snapshot sweep，
  並在該次嘗試後進入 terminal status。
- `VisibleDepthUntilExpiryV1`：在 `activate_at` 只進入 `Active`，不使用試撮或其他
  `matching != Enabled` 的 snapshot 成交；之後第一筆 matching-enabled 的完整 book 變得可見時
  sweep 一次。完整成交進入 `Filled`，部分成交進入 `PartiallyFilled`，零成交進入
  `MatchAttempted`；後兩者的剩餘量保留至精確 `expire_at`，再以 `Expired` 取消。
- `AuctionCrossAtFirstMatchV1`：在 `activate_at` 只進入 `Active`，等待第一筆來源明示 trade
  price 且 `matching == Enabled(CallAuction)` 的正式撮合結果。買單只在 clearing price 嚴格
  低於 limit 時全成，賣單只在 clearing price 嚴格高於 limit 時全成；價格相等保守未成交。
  此 policy 不讀取五檔數量、不重建同價 queue，且同一 order 只判定第一筆正式結果一次。

後兩種 policy 都可表達「先掛入 auction、正式撮合結果出現後才判定成交」；前者以正式完整
book sweep，後者以交易所 strict-cross 價格優先規則判定。兩者都不把 indicative book 當成
正式 fill evidence，只有 `AuctionCrossAtFirstMatchV1` 明確保證 strict cross 時全成。

## 3. Processing order

相同 logical time 的順序固定為：

```text
1. 釋放 visible_time 已到的 observations
2. 執行 strategy decision timers
3. 執行 order expiry／cancel control actions
4. 啟用 scheduled orders
5. 依 policy 分配可見五檔並產生 fills
6. 套用 accounting events
7. 傳遞 deterministic order／fill feedback
```

若同時間另有 market event，其 ReplayCore ordering 仍只由既有 OrderingRule 決定。runner
必須先完成該 market event 的 replay commit，才可將它釋放為可見 observation；不得為符合
timer 倒轉 event ordering。

同一 snapshot 的有限顯示量由 `(activation_time, acceptance_sequence, order_id)` 排序分配，
同一 account 的多筆 order 不得重複消耗同一份 quantity。

## 4. Order lifecycle

scheduled request 至少表達：

```text
client_order_id
batch_id?
activate_at
expire_at?
instrument
side
quantity
order_type
execution_policy
```

平台驗證：

- `activate_at` 不早於 strategy decision time。
- `expire_at` 若存在，必須晚於 `activate_at`。
- instrument、quantity unit、order type 與 universe 合法。
- execution policy 是 execution plan 已核准的 policy。

尚未 activation 的 keyed request 可以 deterministic replace／cancel。activation 後沿用正式
order lifecycle；不得用 replace 偽造更早的 activation。`VisibleDepthUntilExpiryV1` 的 lifecycle
為：

```text
Scheduled -> Active -> Filled
                    -> PartiallyFilled -> Expired
                    -> MatchAttempted   -> Expired
```

`PartiallyFilled` 與 `MatchAttempted` 代表已消耗一次正式 matching snapshot，不會在後續 snapshot
重複推定成交；expiry 只取消剩餘量，已產生的 level fills 仍是正式 fills。

`AuctionCrossAtFirstMatchV1` 的 lifecycle 為：

```text
Scheduled -> Active -> Filled
                    -> MatchAttempted -> Expired
```

其 `Filled` 是單一 clearing-price market-event fill；不會產生 displayed-depth partial fill。

## 5. Responsibility boundary

### Replay core 與 MarketState

維持不變。它們只處理 source-derived events、`match_time` ordering 與完整 snapshot state。

### Runner

負責 control-time queue、observation visibility、同時間順序與 strategy／simulation／accounting
協調，不決定交易價格。

### Strategy

提出 scheduled request 與 timer intent，但不能指定 fill price、fill quantity 或繞過
execution policy。strategy 只能使用已釋放為可見的 observation。

### Simulation

驗證 request、管理 lifecycle、選取 activation-time evidence、執行最多五檔 sweep，並產生
可追溯 level fills 與 aggregate feedback。

### Accounting

只接受 simulation 產生的正式 fills。control timer 或 strategy 估值不得直接改寫 ledger。

## 6. Identity 與 artifacts

plan 與 run artifacts 至少記錄：

- policy name／version。
- market-data latency、order latency、staleness 與 depth levels。
- control ordering version。
- scheduled request、activation、expiry、level fills 與 feedback identity。

相同 source、plan、strategy 與 config 必須得到相同 control actions、orders、fills、ledger 與
checksums。

## 7. Consequences

正面影響：

- 不修改 replay event stream，即可表達沒有同時 market event 的 activation／expiry。
- market-data latency 與 order latency 不再混為同一 eligibility delay。
- 五檔成交假設具有獨立 model identity，可與保守 subsequent-event model 比較。

代價與限制：

- runner 必須協調兩條時間線並版本化同時間順序。
- activation-time snapshot sweep 比預設模型寬鬆，結果必須明確標示模型假設。
- 若沒有符合 visibility／staleness 的完整 snapshot，order 不得成交。

## 8. Compatibility

- 既有 `OrderIntent`、subsequent-event fill model 與 canonical output 保持原語意。
- 新 request／policy 使用新的 schema 與 model version。
- 未選擇 scheduled policy 的 run 不建立 control-time actions，結果必須與升級前一致。

## 9. Validation

至少驗證：

- control action 不改變 replay event/state checksum。
- observation 在 `visible_time` 前不可用。
- activation 不重複套用 order latency。
- expiry 先於同時間 activation 時，過期 order 不成交。
- 五檔依價格順序 sweep，深度不足產生 deterministic partial／unfilled result。
- passive policy 在 activation 不成交，只在 matching-enabled book 可見後嘗試一次，並於精確
  expiry 取消剩餘量。
- 多筆 order 不重複消耗同一 snapshot quantity。
- 未選擇新 policy 時，既有 golden artifacts 不變。
