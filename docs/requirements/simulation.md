# 模擬與帳務需求

## 1. 文件目的

本文件將[產品需求](../product-requirements.md)中的 `SIM-01` 與 `SIM-02` 細化為
可設計、實作與驗證的系統需求。

第一版模擬的原則是保守、容易理解、可重現，且不宣稱 Teralion 五檔 snapshot
無法支持的撮合或排隊精度。

本文件不固定：

- Rust order／fill／ledger types
- matching data structure 或演算法
- fee、tax 與 slippage 的實際費率
- monetary decimal representation
- persistence format
- CLI configuration syntax

上述選擇由 [execution simulation 設計](../design/execution-sim.md)記錄。具體設定
不得放寬「下一個可用事件才可判定」、「無法確認時預設不成交」及「不模擬真實
queue position」等邊界。

## 2. 範圍與模型限制

模擬層接收策略的 order intent，依後續可觀察市場事件判定模擬成交，再更新帳務：

```text
strategy order intent
-> validate and create simulated order
-> wait for subsequent eligible market event
-> apply versioned fill model
-> fill／partial fill／no fill
-> fee and tax
-> cash and position ledger
-> strategy feedback and run result
```

第一版支援：

- market order
- limit order
- configurable slippage
- configurable fee、tax 及 instrument multiplier
- 可選的成交量／顯示量數量限制
- partial fill；當數量限制啟用且可成交量小於剩餘量時
- cash、position、realized／unrealized P&L

第一版不支援或不宣稱：

- 真實交易所 matching engine
- 逐筆委託簿或 queue position
- hidden liquidity
- 真實 order latency 或 network latency
- exchange-specific priority
- 未由來源事件支持的成交
- 實際券商風控、保證金或強制平倉
- corporate action、融券、借券或複雜 margin accounting；除非後續需求明確加入

## 3. 通用定義

### 3.1 Origin event

`origin event` 是策略產生 order intent 時正在處理的 market event。

### 3.2 Subsequent eligible event

`subsequent eligible event` 是 deterministic replay order 中位於 origin event
之後、屬於目標商品、其 `TradingContext` 允許 matching，且帶有該 fill model
判定所需市場資料的事件。

其他商品的下一事件不會使訂單取得目標商品的價格。相同 `match_time` 但 tie-break
位於 origin event 之後的目標商品事件屬於後續事件；它不得在 intent 建立前被策略
看見。

### 3.3 Observable price

`observable price` 是 subsequent eligible event 中由已確認 source mapping 提供的
trade 或完整 snapshot price。推算的 queue、hidden liquidity 或未完成 bar price
不是 observable price。

### 3.4 Fill model

`fill model` 是將 order 與後續 market event 轉為 fill／no fill 的具版本規則。
每次 run 必須選定並記錄一個明確模型及其參數。

### 3.5 Conservative uncertainty

當可觀察資料不足以證明某模型允許的成交時，default conservative model 必須
選擇 no fill 或繼續等待，不得為提高成交率自行補充市場事實。

### 3.6 Accounting event

`accounting event` 是由 accepted order、fill、fee、tax、mark 或明確資金設定造成的
ledger 變化。每筆變化必須有穩定 identity 及來源關聯。

## 4. SIM-01：基礎成交模型

### SIM-01.1 Order intent 驗證

模擬層必須在建立 simulated order 前驗證 intent，至少包含：

- strategy instance 可識別。
- market／symbol 在 execution plan 及 strategy universe 內。
- side 受支援。
- quantity 合法且大於零。
- order type 受支援。
- limit order 具有合法 limit price。
- 計價、數量及必要 instrument metadata 可用。
- origin event 的 `TradingContext` 允許該類型的新 order entry。

不合法 intent 必須產生可追溯 rejection feedback，不得：

- 靜默捨棄。
- 自動改成另一 order type。
- 將非法 quantity 或 price 截斷成合法值。
- 修改 origin market event。

### SIM-01.2 Eligibility boundary

策略在 origin event 產生的 order，最早只能從 subsequent eligible event 開始判定
fill。

系統不得：

- 使用 origin event 的 trade／book 回填同一事件產生的 order。
- 使用 deterministic order 中位於 origin event 之前的資料。
- 使用下一事件之後才知道的 high／low、volume 或 final statistics。
- 因為事件具有相同 `match_time` 就忽略 tie-break 的先後。

order acceptance 可以在 intent 產生後立即回報，但 fill 必須遵守上述 eligibility。

simulation 必須使用
[ADR-0004](../architecture/decisions/0004-trading-context-and-eligibility.md)定義的
`TradingEligibilityPolicy`。只有 `matching=Enabled(...)` 的 subsequent event
可以進入 price／quantity fill model；`Indicative(...)` 或 `Unknown` matching
不得 fill。`CoolDown` 是 phase gate，不是 event；它不合成 market observation，
且該 phase 不允許 fill。

### SIM-01.3 Market order

market order 必須使用第一個能依目前 fill model 提供合法執行價格的 subsequent
eligible event，再套用設定的 slippage。

必要行為：

- buy slippage 不得使價格對策略更有利；sell 同理。
- 若事件沒有模型所需價格，訂單不得以 `0`、前一價或未來價成交。
- 沒有可用價格時，訂單維持 pending 或依明確 order policy 結束；policy 必須記錄。
- fill price 的選價來源及 slippage 計算可追溯至該 market event。
- 若套用 slippage 後價格不合法，必須拒絕該 fill 並明確報錯。

使用 trade、best quote 或其他 observable price 的優先順序由 versioned fill model
定義，不得依當次執行臨時改變。

### SIM-01.4 Limit order

limit order 只有在 subsequent eligible event 提供下列證據之一時才可判定 fill：

- 後續可觀察 trade 達到或穿越 limit。
- 後續完整 snapshot 的可執行一側 quote 達到或穿越 limit，且所選 fill model
  明確允許 quote-based fill。

方向必須正確：

- buy 只在可觀察賣價或成交價不高於 buy limit 時具價格資格。
- sell 只在可觀察買價或成交價不低於 sell limit 時具價格資格。

觸及 limit 只表示所選簡化模型允許成交，不代表平台知道真實 queue position。
結果必須記錄使用 trade-based、quote-based 或其他明確模式。

fill price 不得違反 order limit；對策略更有利的 price improvement 是否允許及如何
選價，必須由 fill model 明確定義並納入版本。

### SIM-01.5 Quantity 與 partial fill

fill model 可以設定：

- 不限制可成交量的簡化模式。
- 以 subsequent trade quantity 限制。
- 以 snapshot 顯示量限制。

啟用數量限制時：

- fill quantity 不得超過 order remaining quantity。
- fill quantity 不得超過該模型從目前事件認定的可用量。
- 小於 remaining quantity 時必須記錄 partial fill。
- order remainder 的 pending／cancel 行為必須由明確 policy 決定。
- 同一事件的顯示量或成交量不得被同一模擬帳戶的多筆 order 無限制重複使用；
  allocation rule 必須 deterministic 且版本化。

顯示量不是實際可成交保證。使用 displayed-volume model 時，結果必須清楚標示
這是估算，不能宣稱真實市場成交。

### SIM-01.6 Order processing order

同一 strategy 或多個 strategy orders 競爭同一 subsequent event 的有限模擬量時，
必須使用固定且版本化的 allocation order。

allocation order 不得使用：

- hash map iteration order
- thread completion order
- memory address
- 未固定 random value

平台排序只保證模擬可重現，不代表交易所真實 priority。

第一版可以只支援單一 strategy instance；即使如此，同一 instance 的多筆 order
仍必須有 deterministic order identity 與 processing order。

### SIM-01.7 Slippage

slippage 設定必須：

- 明確指定模型、方向、單位及數值。
- 對 buy／sell 採符合不利成本方向的規則；除非使用者明確選擇其他已命名模型。
- 使用不受 locale 或浮點非決定性影響的計算與 rounding policy。
- 納入 fill model configuration 及 run result。

若使用者選擇較寬鬆或可能更有利的 slippage／fill policy，結果必須以模型名稱及
參數清楚標示。

### SIM-01.8 Fee、tax 與 multiplier

每次 run 必須記錄：

- fee model/version 及參數
- tax model/version 及參數
- slippage model/version 及參數
- 每個商品使用的 multiplier value
- multiplier 來自 Teralion、使用者設定或其他 reference source

缺少計算必要 multiplier 時，不得猜測。需要 P&L 的 run 必須在開始前取得明確值，
否則停止並指出 market、symbol、trading date 與缺少欄位。

費稅的適用市場、side、時點及 rounding 必須由 model 明確定義；同一 fill 不得因
iteration order 重複計費或漏計。

### SIM-01.9 Conservative 與寬鬆模型

default model 必須採 conservative uncertainty：無法確認時不成交或繼續等待。

平台可以提供較寬鬆模型，但必須：

- 由使用者明確選擇。
- 具有不同且穩定的 model identity/version。
- 在 plan 與 result 顯示。
- 不使用未來事件。
- 不聲稱提高後的 fill rate 是真實 exchange matching。

模型切換不得改變 source event、MarketState 或 replay ordering。

### SIM-01.10 Order 與 fill trace

每個 simulated order 必須可追溯至：

- strategy instance
- origin event
- order intent
- validation result
- order state changes

每個 fill 必須可追溯至：

- simulated order
- subsequent eligible event
- observable price source
- fill model/version
- quantity／allocation rule
- slippage
- fee、tax 及 multiplier

trace identity 必須穩定，使相同輸入可產生相同 order／fill sequence 及 checksum。

### SIM-01.11 驗收條件

`SIM-01` 至少必須由下列證據驗證：

- order 不在 origin event 成交的無前視測試。
- pre-open trial、pre-close trial、indicative matching 與 unknown matching 不成交
  的測試。
- `CoolDown` phase 不合成 event，且不允許新 order／fill 的測試。
- opening／closing result 只評估較早 pending order 的測試。
- market buy／sell 使用後續價格及不利 slippage 的測試。
- 沒有可用價格時 no fill／pending 的測試。
- limit buy／sell 未觸及、觸及及穿越 limit 的測試。
- trade-based 與 quote-based model 的差異測試。
- volume cap、partial fill 及 remaining quantity 的測試。
- 多筆 order allocation 不重複消耗有限模擬量的測試。
- invalid intent rejection 測試。
- 缺少 multiplier 明確失敗的測試。
- 相同輸入產生相同 order／fill checksum 的測試。
- eligibility／market-rule／fill-model versions 完整進入 run 與 decision trace 的測試。

M2 必須以 TWSE 2330 單日資料驗證 market／limit fill、slippage、fee、tax 及基本
P&L；M3 補 TAIFEX futures multiplier 與 `TradeBatch`／`BookSnapshot` 模型。

## 5. SIM-02：帳務

### SIM-02.1 Ledger 範圍

第一版帳務至少必須包含：

- order records
- fill records
- cash
- position
- fee
- transaction tax
- realized P&L
- unrealized P&L
- basic performance summary

ledger 必須由 simulation 事件按 deterministic order 更新，策略不得直接修改 cash、
position 或 P&L。

### SIM-02.2 Order 與 fill records

order record 至少必須保存：

- order identity
- original intent identity
- market／symbol
- side、quantity、order type 及 limit；若適用
- origin replay time
- state transitions 及原因
- filled／remaining quantity

fill record 至少必須保存：

- fill identity
- order identity
- market／symbol
- fill replay time 及觸發 market event
- price、quantity 及 side
- slippage
- fee、tax
- multiplier 及其 provenance

拒絕、取消及未成交 order 也必須保留，不得只輸出成功 fills。

### SIM-02.3 Cash

每個 fill 對 cash 的影響必須由 side、price、quantity、multiplier、fee 及 tax 明確
計算。

cash update 必須：

- 與 fill 在同一 accounting transaction 中原子套用。
- 採明確 currency、precision 及 rounding policy。
- 不使用 binary floating-point 的 debug representation 作為 canonical 結果。
- 可由 fill records 重新計算並核對。

多 currency 或 FX conversion 若未有明確需求及 reference rate，第一版不得自行
換算。

### SIM-02.4 Position

每個商品 position 至少必須保存：

- net quantity
- 成本基礎或可計算 realized P&L 的等價資料
- realized P&L
- 最新 mark 及 mark source；若用於 unrealized P&L

position accounting method 必須固定且版本化。若未來支援 FIFO、average cost
或其他方法，run 必須明確記錄實際使用方法。

position update、cash update、fee／tax 與 fill record 必須原子一致；失敗時不得
留下部分帳務。

### SIM-02.5 Realized P&L

realized P&L 只能由已發生 fills 及明確 accounting method 計算。它必須包含或能
分別顯示：

- trading gain／loss
- fee
- tax

不得使用尚未發生的 market price、日終價格或假設 fill 計入 realized P&L。

### SIM-02.6 Unrealized P&L 與 marking

unrealized P&L 必須使用 execution plan 允許且在計算時已可觀察的 mark。

marking policy 必須明確定義：

- 使用 trade、mid、bid／ask 或其他明確設定的可觀察來源。
- 缺少 mark 時的 unavailable／fallback 行為。
- 計算時點。
- price、multiplier 及 rounding。

盤中策略 feedback 不得使用未來 mark。final summary 可以使用 replay 結束時已
合法觀察的最後 mark，不得為了補齊結果插入 replay timeline 之外的盤後統計。

### SIM-02.7 Accounting traceability

所有帳務變化必須形成可追溯鏈：

```text
strategy callback
-> order intent
-> simulated order
-> subsequent market event
-> fill
-> fee／tax
-> cash／position transition
-> P&L／performance result
```

任何 cash、position、fee、tax 或 P&L 變化若無法追溯至明確 accounting event，
必須視為 invariant violation。

### SIM-02.8 Reconciliation

平台必須能在 run 結束時驗證至少下列 invariant：

- order filled quantity 不超過 original quantity。
- fill quantity 總和與 order record 一致。
- position 由 fills 重算後一致。
- cash 由 initial cash、fills、fee 及 tax 重算後一致。
- realized／unrealized P&L 與所選 accounting／marking policy 一致。
- ledger 中不存在無來源 identity 的變化。

reconciliation failure 必須使 run 標示失敗，不得仍輸出成功績效。

### SIM-02.9 Basic performance summary

第一版 basic performance summary 至少提供：

- initial／final cash
- final positions
- realized P&L
- unrealized P&L；若 mark 可用
- total fee
- total tax
- order count
- fill count
- trade 或 round-trip count；若定義明確

任何比率型指標只有在公式、period、denominator 及 unavailable behavior 明確時
才可加入。第一版不要求任意選定 Sharpe ratio 或其他高階績效指標。

### SIM-02.10 驗收條件

`SIM-02` 至少必須由下列證據驗證：

- buy／sell、加碼、減碼、平倉及反向 position 的帳務測試。
- partial fill 的 cash／position transition 測試。
- fee、tax、multiplier 及 rounding 測試。
- realized P&L 與 chosen cost method 的 golden tests。
- missing mark 與合法 final mark 的 unrealized P&L 測試。
- 由 records 重算 cash／position 的 reconciliation test。
- 人為破壞 ledger 時 run 失敗的 negative test。
- 相同 fills 產生相同 ledger／result checksum 的測試。

## 6. 跨需求不變條件

任何 fill model 與 accounting design 都必須維持：

1. order 最早從 origin event 之後的目標商品可用事件判定。
2. market／limit fill 只使用後續可觀察資料。
3. default model 無法確認時不成交或等待。
4. 模型不宣稱 queue position、hidden liquidity 或真實 matching。
5. volume allocation 不重複使用有限模擬量。
6. fill、fee、tax、multiplier 及 accounting method 具版本與 provenance。
7. 每筆帳務變化可追溯至 intent、order 及 fill。
8. position、cash 與 P&L 可以由 records reconciliation。
9. 同一資料、設定與版本產生相同成交及帳務結果。
10. reconciliation failure 不得偽裝成成功 backtest。

## 7. 與其他需求的關係

| 關係 | 說明 |
| --- | --- |
| `DATA-05` | 提供 multiplier 或記錄明確補充來源 |
| `REPLAY-02` | 定義 origin 與 subsequent event 的 deterministic 順序 |
| `REPLAY-04` | 保證 strategy callback 不使用未來事件 |
| `STRAT-01` | 產生 intent 並接收 order／fill feedback |
| `OPS-02` | 保存模型設定、orders、fills、positions 及 P&L |
| `NFR-01` | 要求 fill 與 accounting 結果可重現 |
| `NFR-03` | 要求 fill model 及不相容版本可識別 |

正式 requirement、design、implementation 與 test mapping 由
[traceability matrix](../traceability.yaml)維護。

## 8. 待下游文件決定的事項

| 議題 | 文件 |
| --- | --- |
| Order、fill、ledger types 與 processing pipeline | [execution simulation 設計](../design/execution-sim.md) |
| Strategy intent／feedback API | [strategy API 設計](../design/strategy-api.md) |
| Observable market price 與 state views | [market state 設計](../design/market-state.md) |
| Multiplier 與 reference-data provenance | [資料需求](data.md)及[data sync 設計](../design/data-sync.md) |
| Config、run manifest 與結果 inspection | [CLI 操作](../operations/cli.md) |
| Acceptance fixtures 與 golden accounting results | [驗收規格](../verification/acceptance.md) |

下游文件不得以較高 fill rate 或簡化帳務為由使用未來資料、猜測 multiplier、重建
queue position，或產生無法追溯的 cash／position 變化。
