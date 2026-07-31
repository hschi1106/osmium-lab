# M2：真實資料與離線回測

## 1. 文件目的

本文件定義 `osmium-lab` 第二個可獨立驗證的增量：以 Teralion 的 TWSE `2330`
單一已結束交易日為基準，完成資料規劃、分頁下載、verified local source、
rebuildable replay cache、離線 replay、策略下單、保守成交模擬、帳務與結果
inspection。

```text
increment_contract_version = 1
milestone                  = M2
reference_market           = TWSE
reference_symbol           = 2330
reference_trading_date     = 2026-07-27
reference_session          = regular
```

M2 承接 M1 已固定的 domain event、`match_time` ordering、MarketState、
TradingContext、strategy callback 與 canonical checksum，不重新解讀 Teralion
flags，也不更改 M1 golden 來配合 simulation。

產品範圍、術語與優先順序以[產品需求](../product-requirements.md)為準。詳細需求
來自：

- [資料需求](../requirements/data.md)
- [回播需求](../requirements/replay.md)
- [策略需求](../requirements/strategy.md)
- [模擬與帳務需求](../requirements/simulation.md)
- [操作與非功能需求](../requirements/operations.md)

## 2. 交付結果

M2 完成時，使用者必須能以一份版本化設定及一個頂層入口執行：

```text
config
-> plan local state and required actions
-> sync missing Teralion pages and daily instrument data
-> verify and atomically publish reusable local source
-> build or reuse a source-bound replay cache
-> freeze an offline execution plan
-> replay by match_time
-> update MarketState and derive TradingContext
-> run strategy and validate order intents
-> evaluate pending orders on subsequent eligible events
-> atomically update fills, cash, positions and P&L
-> reconcile ledger
-> atomically publish inspectable run artifacts
```

資料準備與 backtest 必須可以分開執行。完成 `sync`、`verify` 及必要 cache build
後，`replay`、`backtest` 與 `inspect` 必須在無網路、無 Teralion API key 的環境
成功。

## 3. Entry gates

M2 acceptance 開始前必須滿足：

1. M1 的 event、ordering、MarketState、TradingContext、strategy output 與 golden
   checksum 已固定。
2. M1 network-disabled gate 已關閉；M2 implementation 可以先進行，但 M2 不得在
   M1 formal acceptance 未完成時標為 `Passed`。
3. reference dataset 的取得及保存範圍具有合法 authorization 與 provenance。
4. Teralion endpoint、cursor、認證及時間契約與
   [Teralion interface](../interfaces/teralion.md)一致。
5. 下列空白 design／operations 文件必須在對應程式碼前先固定首版契約：
   [data sync](../design/data-sync.md)、
   [execution simulation](../design/execution-sim.md)及
   [local data operations](../operations/local-data.md)。
6. M2 CLI 及 config schema 變更必須更新
   [CLI 操作契約](../operations/cli.md)。

## 4. 範圍

### 4.1 包含

- 單一已結束 trading date：`2026-07-27`。
- 單一 explicit universe：TWSE `2330`。
- `regular` session 及固定五分鐘 download／replay margins。
- Teralion coverage、symbol range、ticks、daily instrument 與 opaque cursor。
- `STOCK_SNAPSHOT`、`STOCK_REALTIME` 及 M1 已確認的 composition／mapping。
- source partition 的 missing、building、complete、incomplete、corrupt 狀態。
- 已驗證來源資料的 immutable revision、per-page zstd storage與 provenance。
- 依 source checksum 與 versions 綁定、可刪除重建的 replay cache。
- 一個 compile-time linked Rust strategy instance。
- `Market` 與 `Limit` order intent。
- deterministic acceptance／rejection、pending、partial fill、fill 與 end-of-run
  cancellation。
- configurable price evidence、quantity cap、slippage、fee、tax、quantity-unit
  size、instrument multiplier、initial cash、cost basis 及 marking policy。
- TWD cash、signed net position、realized／unrealized P&L 與 reconciliation。
- plan、sync、verify、replay/backtest、inspect 的頂層 CLI workflow。
- successful、failed 及 explicit degraded diagnostics。
- source、cache、event、state、strategy、order、fill、ledger 與 result checksums。

### 4.2 不包含

- 當日增量下載或即時交易。
- TPEx、TAIFEX、權證、選擇權或多商品策略。
- 盤中零股、盤後零股、盤後定價、鉅額交易。
- TAIFEX 夜盤、跨日 trading date 或 derivatives multiplier discovery。
- 逐筆委託簿、真實 queue position、hidden liquidity 或交易所 matching engine。
- cancel／replace intent、stop order、iceberg、IOC、FOK 或複雜 order lifecycle。
- 多 strategy account、multi-account、multi-currency 或 FX conversion。
- corporate action、dividend、borrow、margin、risk engine 或 liquidation。
- 動態 strategy loading、腳本策略或 distributed replay。
- 日內 bars、日終 `close`／`stats` 作為 replay event。
- 未經明確公式與 period 定義的 Sharpe ratio 等高階績效指標。

M2 可以使用 synthetic order scenarios 驗證 simulation branches，但 source format、
session、price、quantity及 flags mapping 仍必須以實際 Teralion fixture 或官方文件
為證據。

## 5. Reference run configuration

M2 必須提供一份可提交、無 secret 的 acceptance config。logical fields 至少包含：

```text
RunConfig {
    config_version
    market = TWSE
    trading_dates = [2026-07-27]
    universe = [2330]
    session_kinds = [regular]
    strategy_binding
    data_root
    source_policy
    cache_policy
    replay_data_policy
    fill_model
    quantity_allocation
    slippage_model
    fee_model
    tax_model
    instrument_economics
    initial_cash
    position_accounting
    marking_policy
    output_policy
}
```

設定檔格式、欄位名稱與 canonical encoding 由 CLI／operations design 固定，但必須：

- 具有 schema version。
- 所有 defaults 可在 effective plan 中看見。
- 不依賴 locale、filesystem iteration order 或本機 wall clock 改變 domain result。
- 不包含 API key value。
- 對 fee、tax、slippage、quantity unit size、multiplier 與 rounding 使用明確值及
  provenance；不得使用未記錄的「市場慣例」。
- 產生 canonical effective-config checksum。

reference config 必須同時包含至少一個 market-order 與一個 limit-order
acceptance scenario，並使 partial fill、rejection 或 end-of-run cancellation 至少
各有一條可檢查路徑。scenario strategy 只能依目前 callback 可見資訊作決策，
不得讀取預先計算的未來 event ordinal 或 golden price。

## 6. Planning 與 SessionPlan

### 6.1 Frozen execution plan

任何 download 或 backtest 副作用前，planner 必須建立可檢查的 effective plan：

```text
ExecutionPlan {
    plan_identity
    config_checksum
    requested_partitions
    source_actions
    verification_actions
    cache_actions
    replay_plan
    strategy_binding
    simulation_binding
    accounting_binding
    result_binding
    network_requirement
    degraded_scopes
    version_set
}
```

plan identity 只由 canonical effective values 產生，不含 absolute machine path、
API key、wall-clock duration 或 random run ID。

### 6.2 Reference session

TWSE `2330` regular session 使用
[ADR-0003](../architecture/decisions/0003-session-windows-and-strategy-activation.md)：

```text
official session: 09:00–13:30 Asia/Taipei
download window:  [08:55, 13:35) by received_at
replay window:    [08:55, 13:35) by match_time
```

planner 必須 materialize absolute times、calendar／session versions 及 trading
date。strategy 只宣告 `regular`，不能覆寫五分鐘 margins 或自行提供絕對開收盤
時間。

### 6.3 Plan actions

每個 partition 必須被分類為：

- `ReuseCompleteSource`
- `DownloadMissingSource`
- `ResumeOrRestartBuilding`
- `RejectIncomplete`
- `RejectCorrupt`
- `CoverageUnavailable`
- `ReuseValidCache`
- `RebuildCacheFromCompleteSource`

`plan` 本身不得下載 ticks、修改 complete source、建立成功 run 或執行 strategy。
若 plan 需要線上 coverage lookup，必須明確標示 network requirement；offline
planning 只能使用已驗證的 local coverage／manifest。

## 7. Teralion sync

### 7.1 Online boundary

只有 `sync` 可以要求 Teralion credential 並建立 Feed Archive HTTP client。
`verify`、cache build、`replay`、`backtest` 與 `inspect` 不得：

- 讀取 `TERALION_API_KEY`。
- 隱式 fallback 至網路。
- 因本地資料缺少而自動執行 sync。

credential 只從 runtime secret source 取得，不得寫入 config、request identity、
manifest、cursor checkpoint、log、error 或 result。

### 7.2 Cursor state machine

ticks 與 daily instrument 的每個 paged query 必須：

1. 固定不含 credential 的 query identity。
2. 將 cursor 當成 opaque bytes／text 原樣傳回。
3. 先 durable stage page payload、page count、response checksum 與 next cursor。
4. 只在服務回傳 terminal cursor 時完成 query。
5. 偵測 cursor 未前進、循環、重複 page identity 或 query identity 改變。
6. retry 或程序重啟後產生與不中斷 run 相同的 published source bytes/checksum。

page size、retry count 與 worker completion order 不得改變 source partition identity。

### 7.3 Coverage 與 zero records

sync 前必須確認：

- market coverage。
- `2330` 的可用日期範圍。
- `2026-07-27` 是已結束且可用的 exchange trading date。

coverage 不包含、合法 zero records、query failure 與未走完 cursor 必須是不同狀態。
terminal cursor 只證明 query chain 結束，不單獨證明 exchange session 完整。

## 8. Verified local source

### 8.1 Partition identity

M2 最小 source partition identity：

```text
source
+ market
+ symbol
+ trading_date
+ logical session coverage
+ immutable revision checksum
```

physical layout 由 data-sync design 固定，但不能讓相同 symbol、不同 market／date
互相覆寫。path 不能取代 checksum identity。

### 8.2 Manifest

published source manifest 至少保存：

- partition identity 與 revision。
- sanitized endpoint／query identity。
- requested download windows。
- source formats、record counts、uncompressed/compressed byte counts。
- page identities、page count 與 terminal cursor reached。
- 每個 ticks page及 daily instrument使用 `ZstdPerPageV1`：zstd level 3、frame
  checksum、無 dictionary，且 data root不保存 uncompressed `.json` payload。
- uncompressed semantic及 compressed storage checksum algorithms/versions與
  checksums。
- daily instrument payload identity及雙 checksum。
- calendar、session-window policy 及 source schema identity。
- acquisition／verification tool versions。
- completeness state 與 reasons。
- atomic publish identity。

full cursor、authorization header、cookie、signed URL 或 API key 不得保存。
source revision identity以解壓後 exact response bytes計算；compression level、
implementation/version及 compressed output不得改變 semantic source identity。

### 8.3 State 與 atomic publish

對外至少呈現：

```text
missing -> building -> complete
                    -> incomplete
complete ----------> corrupt
incomplete --------> building
corrupt -----------> building
```

只有 cursor、payload、counts、schema、daily instrument、trading-date ownership 與
checksums 全部驗證成功後，staging revision 才能 atomic publish 為 `complete`。
crash、disk-full 或 validation failure 不得留下看似 complete 的目錄。

### 8.4 Immutable reuse

相同 partition 再次 sync：

- complete source checksum 與 provenance 相同時直接 reuse，HTTP request count 為
  zero。
- source 內容不同時建立明確 revision 或停止；不得覆寫原 revision。
- cache stale／missing 不得觸發 source redownload。
- repair／replacement 必須是使用者可見且可稽核的 action。

## 9. Verify 與 degraded policy

`verify` 不執行 strategy，至少檢查：

- manifest schema／version。
- partition identity。
- payload、daily instrument、count 與 checksum。
- cursor terminal evidence。
- source formats 可由 current normalizer 識別。
- calendar／session／trading-date ownership。
- cache lineage、checksum、descriptor bounds 及 version compatibility；若存在。
- P&L 所需 instrument economics 是否有明確來源。

default `Strict` backtest 只接受 `complete` source 與 valid compatible cache。

M2 可以提供 `ExplicitDegraded`，但只能在 plan freeze 前逐一列出允許略過的
instrument／date／segment／format scope；它不得：

- 接受 corrupt bytes。
- 破壞 event atomicity、ordering monotonicity 或 no-lookahead。
- 在 runtime 自行擴大 omission。
- 把缺少值表示成合法零值。

degraded run 必須使用不同 completion quality、result identity 與 manifest 欄位。
M2 acceptance 的 reference backtest 必須使用 `Strict`；degraded 只驗證 failure
visibility，不作為 golden P&L。

## 10. Replay cache

### 10.1 Derived boundary

cache 是從 `complete` local source 建立的衍生 artifact，不是 source data：

- 可以刪除並離線重建。
- 不得修改 source revision。
- 不得成為唯一 provenance。
- invalid cache 不得在 replay runtime 邊讀邊 fallback 至 source。

### 10.2 Cache identity

cache descriptor 至少綁定：

```text
market + symbol + trading_date
source partition identity + source checksum
cache format version
normalizer mapping versions
market/event/canonical schema versions
ordering rule version
session/calendar ownership versions
event count + first/last ordering key
cache payload checksum
```

任一語意或 checksum 不相容時，plan 必須選擇 offline rebuild 或停止。若 complete
source 仍有效，rebuild 不得下載。

### 10.3 Stream contract

M2 cache reader 必須：

- 只開啟 execution-plan universe 的 symbol／date stream。
- 產生已驗證且依完整 OrderingKey non-decreasing 的 DomainEvent。
- 保留合法 duplicate occurrences。
- 驗證 expected count、bounds、checksum 與 EOF。
- 提供 bounded read／prefetch；不得把完整期間 events 全載入記憶體作為正式
  backtest 路徑。

M2 reference universe 只有 `2330`，仍須以 outside-universe sentinel stream 測試
證明它未被 open。

## 11. Strategy 與 order intent

### 11.1 API extension

M2 啟用 strategy sink 的 `emit_order_intent`，並加入 `on_feedback`。最小 logical
intent：

```text
OrderIntent {
    instrument
    side: Buy | Sell
    quantity
    order_type: Market | Limit { limit_price }
    time_in_force: Day
}
```

具體 Rust types 與 canonical version 由 execution-sim／strategy design 固定。
不得使用 generic JSON/map 或讓 strategy 傳入 raw market flags。

### 11.2 Validation

simulation 建立 order 前至少驗證：

- strategy instance、origin occurrence 及 output sequence。
- instrument 位於 frozen plan 與 strategy universe。
- side、order type 與 `Day` policy 受支援。
- quantity 大於零且 unit 與 instrument economics 相容。
- limit price 存在、正值且 exact。
- origin TradingContext 允許該 order type 的新 order entry。

invalid intent 產生 deterministic rejection feedback 及 record；不得靜默捨棄、
截斷數值、修改 event 或轉換 order type。

### 11.3 Identity 與 feedback

order identity 必須由 strategy identity、origin occurrence、output sequence 及
versioned canonical intent 決定。fill identity 必須再關聯 triggering occurrence
及 fill sequence。

M2 feedback 至少表達：

- accepted。
- rejected(reason)。
- partially filled。
- filled。
- cancelled(end_of_run)。

feedback 只在 current event 的 validation、較早 pending orders、fills 與 accounting
完成後發布。feedback 產生的新 intent 最早由再下一個 eligible occurrence 評估。

M2 不支援 strategy 主動 cancel／replace；stream 完成後仍 pending 的 `Day` order
以 `Cancelled(EndOfRun)` 結束並保留 record。

## 12. Simulation

### 12.1 Event ordering boundary

每個 accepted event 的順序固定為：

```text
1. select DomainEvent by OrderingKey
2. commit replay clock and MarketState
3. derive TradingContext
4. Strategy.on_event
5. validate and create current callback order intents
6. evaluate orders that were pending before this occurrence
7. commit fills and accounting atomically
8. Strategy.on_feedback
```

步驟 5 建立的 order 不得在步驟 6 使用 origin event。即使下一 occurrence 具有相同
`match_time`，只要 OrderingKey 嚴格在 origin 後，才可以成為 subsequent event。

### 12.2 Fill eligibility

每筆 pending order 對 current occurrence 個別判定。至少下列情況不得 fill：

- current occurrence 是 origin event。
- instrument 不同。
- phase 是 `CoolDown`。
- matching 是 `Indicative(...)` 或 `Unknown`。
- event 缺少選定 model 所需的 price／quantity evidence。
- limit 未觸及或 adverse slippage 後會違反 limit。

pre-open／pre-close trial、delayed open／close、緩跌／緩漲都可以更新 state 並觸發
strategy，但不能成為 fill evidence。closing result 只能評估更早 pending order。

### 12.3 Required fill models

M2 至少提供兩個具穩定 identity/version 的 evidence modes：

| Mode | Price evidence | Quantity evidence |
| --- | --- | --- |
| `TopOfBook` | buy 使用 subsequent best ask；sell 使用 subsequent best bid | 可設定 unlimited 或該 level displayed quantity |
| `TradePrint` | subsequent ordered trade price | 可設定 unlimited 或該 print quantity |

Market order 使用第一個具有合法 evidence 的 subsequent eligible event。Limit：

- buy evidence price 必須 `<= limit`。
- sell evidence price 必須 `>= limit`。
- price improvement 只能使用 current evidence price。
- adverse slippage 後若超出 limit，該 occurrence 不 fill；不得 clamp 成 limit。

model 不能沿用 stale price 填補缺少 evidence，也不能使用 future high／low、
final statistics 或 queue inference。

### 12.4 Quantity allocation

啟用 quantity cap 時：

- fill quantity 不超過 remaining order quantity。
- 不超過 current evidence 可用 quantity。
- 多筆 order 依 deterministic order identity／acceptance sequence 分配。
- 同一 event 的有限 quantity 不得被同一 account 重複使用。
- 部分成交後 remainder 保持 pending，直到後續 fill 或 end-of-run cancellation。

unlimited mode 必須以不同 model identity 明示它沒有使用市場量限制，不能宣稱真實
成交能力。

### 12.5 Slippage

M2 reference run 使用 exact decimal 的 adverse fixed-price-delta model：

- buy 對 evidence price 加上 configured delta。
- sell 對 evidence price 減去 configured delta。
- delta 不得為負。
- 結果必須是合法正價格。
- limit order 調整後仍不可違反 limit。

未來 tick-size／basis-point model 可以新增，但必須有不同 identity、rounding
policy 與 tests。不得用 binary floating point 或 locale-dependent parsing。

## 13. Instrument economics、fee 與 tax

TWSE `TradingUnit` 的 economic quantity 必須由經驗證 metadata 或 explicit config
提供 `units_per_trading_unit`。不得在 core type 中假設所有股票永遠為固定張數。

每個 instrument 的 plan／result 至少記錄：

- quantity unit。
- units per trading unit。
- currency；M2 reference 為 TWD。
- instrument multiplier；TWSE reference 可為明確的 `1`，但仍須記錄 provenance。
- value source、version 及 applicable trading date。

fee／tax model 必須使用 exact configured rate、applicable side、minimum、precision
與 rounding policy。M2 不把某個法規費率硬編碼成永遠正確的 hidden default；
acceptance config 使用明確值驗證算式。

notional、slippage、fee、tax 與 cash effect 必須能由 fill record 重算。缺少
`units_per_trading_unit`、multiplier、currency 或必要 rounding policy 時，在第一個
event 前停止。

## 14. Accounting

### 14.1 Ledger boundary

M2 使用單一 account、TWD cash 與版本化 position accounting。至少保存：

- accepted、rejected、pending、partially filled、filled、cancelled orders。
- fills。
- initial／final cash。
- signed net position。
- cost basis。
- fee 與 tax。
- realized／unrealized P&L。
- final mark 及來源。

strategy 不能直接修改 ledger。

### 14.2 Atomic accounting transaction

每個 fill 必須在單一 transaction 中同時：

1. 建立 fill record。
2. 更新 order filled／remaining quantity。
3. 計算 slippage、fee、tax 與 economic notional。
4. 更新 cash。
5. 更新 position 與 realized P&L。
6. 記錄 accounting trace。

任一步驟失敗不得發布 partial fill／cash／position state。

### 14.3 Position 與 marking

M2 必須選定並版本化一種 cost method；reference implementation 使用
`AverageCostV1`，且必須正確處理加碼、減碼、平倉及 signed-position reversal。
允許 signed position 是 simulation accounting 能力，不代表平台模擬 borrow 或
exchange short-sale eligibility。

unrealized P&L 使用 versioned final marking policy，只能讀 replay 結束前已合法
觀察的 MarketState。缺少合法 mark 時必須為 unavailable，不得插入盤後 stats、
future close 或零值。

### 14.4 Reconciliation

successful run 前必須重算並驗證：

- 每張 order 的 fill sum 不超過 original quantity，且等於 recorded filled。
- cash 等於 initial cash 加所有 fill cash effects、fee 與 tax。
- position 與 cost basis 可由 fills 重建。
- realized／unrealized P&L 符合 model。
- 每筆 ledger change 都有 order／fill／accounting identity。

reconciliation failure 使 run `Failed`，不得輸出成功 performance summary。

### 14.5 Basic performance summary

至少輸出：

- initial／final cash。
- final positions。
- realized P&L。
- unrealized P&L 或 unavailable reason。
- total fee／tax。
- order、rejection、cancellation、fill 及 partial-fill counts。
- 明確定義時的 closed trade／round-trip count。

M2 不要求未定義 period／denominator 的高階比率。

## 15. CLI workflow

M2 擴充目前的 `osmium` binary。最終 command spelling 由 CLI contract 固定，但
至少提供下列獨立能力：

```text
osmium plan      --config <file>
osmium sync      --config <file>
osmium verify    --config <file>
osmium replay    --config <file> --output <new-directory>
osmium backtest  --config <file> --output <new-directory>
osmium inspect   --run <run-directory>
osmium run       --config <file> --output <new-directory>
```

語意：

- `plan`：無 market-data download、無 strategy execution。
- `sync`：只處理 plan 判定需要下載／恢復的 source partitions。
- `verify`：離線驗證 source、cache 與 instrument economics。
- `replay`：離線，只有 event/state/strategy observation；simulation binding
  `NotUsed`。
- `backtest`：離線 replay + strategy + simulation + accounting。
- `inspect`：不重跑，讀取既有 successful／failed／degraded artifacts。
- `run`：plan → sync → verify → cache prepare → backtest 的 convenience
  orchestration；每個 stage 仍保留獨立結果。

M1 的 fixture-based `osmium replay --fixture ...` 可以保留為 acceptance／developer
入口，但不能與 M2 config-based source partition 混淆。

exit status 必須至少可 machine-detect：

- success。
- explicit degraded success。
- usage／config error。
- missing／incomplete／corrupt data。
- external service／network failure。
- incompatible version。
- strategy／simulation／reconciliation failure。

## 16. Run artifacts

successful M2 backtest output directory 至少包含：

```text
effective-config.yaml
execution-plan.yaml
run-manifest.yaml
data-lineage.yaml
cache-lineage.yaml
event-stream.blake3
final-state.blake3
strategy-output.bin
strategy-output.blake3
orders.bin
orders.blake3
fills.bin
fills.blake3
ledger.bin
ledger.blake3
positions.yaml
performance.yaml
warnings.yaml
run-summary.yaml
```

具體 binary encoding 由 design 固定；不得使用 Rust `Debug` 或 serializer default
作 canonical bytes。

`run-manifest.yaml` 至少記錄：

- run status／completion quality。
- effective config 及 execution plan checksums。
- source/cache lineage。
- event、ordering、session、state、strategy、eligibility、fill、fee、tax、
  accounting、marking 與 result versions。
- strategy binary／params identity。
- instrument economics provenance。
- event、warning、order、fill、position counts。
- deterministic artifact checksums。
- failure stage／identity 或 degraded scopes。

wall-clock duration、throughput、host path 等 operational diagnostics 可以保存，但不
進 domain result checksum。

failed run 可以保存 processed prefix、已 commit orders／fills 及 diagnostics，但：

- 不得有 `outcome: passed`。
- 不得把 prefix checksum 放進 complete checksum 欄位。
- 不得產生看似合法的 final performance。
- atomic publisher 不得把 partial staging directory 冒充 successful output。

## 17. Offline、security 與 second-run contract

M2 必須以兩段程序驗收：

```text
online preparation:
  plan -> sync -> verify -> cache build

network-disabled execution:
  plan/reuse -> backtest -> inspect
```

第二段必須：

- 移除 Teralion 及相關 auth environment variables。
- 由 network-disabled CI/container 強制無網路。
- 使用第一次發布的 complete source／valid cache。
- HTTP request count 為 zero。
- 只開啟 `2330 + 2026-07-27` 的 selected stream。
- 產生與 network-enabled local execution 相同的 deterministic artifacts。

第二次完整 workflow 執行必須 reuse complete source，不能重新下載。刪除 cache 後
則必須只從 local source 離線重建，source checksum 保持不變。

secret scan 至少涵蓋 source、manifest、cache descriptor、run artifacts、logs 與
repository，並檢查 API key、authorization、bearer、cookie 及 credential-like URL
query。

## 18. Determinism 與效能基線

相同 source revision、config、strategy binary 與 version set 必須產生相同：

- execution plan identity。
- normalized event bytes／checksum。
- final MarketState checksum。
- strategy outputs。
- order／fill sequence 與 checksums。
- ledger／positions／performance 與 result checksum。

至少比較：

1. 原始 source discovery order。
2. 三個固定 shuffle／page-size perturbations。
3. 同一 input 連續 10 runs。
4. cache build 與 cache reuse。
5. cache deletion 後 offline rebuild。
6. debug 與 release。
7. 支援範圍內不同 prefetch／worker settings。

M2 建立 reference performance report，但在量測前不武斷制定硬門檻。至少記錄：

- source sync uncompressed/compressed bytes、ratio、pages及 records。
- cache build elapsed time 與 output bytes。
- cache-hit backtest elapsed time。
- events per second。
- peak resident memory；若量測工具可用。
- HTTP request count。
- source zstd object decode count及 JSON parse count。
- opened stream count。

cache-hit backtest 必須證明不重新下載、不解壓 source zstd object且不重新解析全部
source JSON。正式 replay working set不得與完整 event count線性成長。

## 19. Acceptance scenarios

M2 verification plan 必須為下列 criteria 配置穩定 test IDs 與 machine-readable
evidence：

| ID | 情境 | 預期結果 |
| --- | --- | --- |
| `M2-AC-01` | 以 acceptance config 執行 `plan` | 正確顯示 download/reuse、verify、cache、universe、models 與 network requirement，無 market-data write |
| `M2-AC-02` | Teralion query 超過一頁 | opaque cursor 走到 terminal，無截斷、循環或 query drift |
| `M2-AC-03` | sync 中斷、retry exhausted、partial zstd frame 或 partial write | partition 維持 building/incomplete，未發布為 complete；重跑的 uncompressed semantics 與 uninterrupted run 相同 |
| `M2-AC-04` | 相同 identity 第二次 sync | reuse complete source，HTTP request count 為 zero；不同內容不靜默覆寫 |
| `M2-AC-05` | verify valid／missing／incomplete／corrupt partitions | 五種狀態與修復建議正確；Strict 拒絕非 complete |
| `M2-AC-06` | cache hit、刪除後 rebuild、version/checksum mismatch | valid cache reuse；只用 local source deterministic rebuild；stale/corrupt cache 不進 replay |
| `M2-AC-07` | execution universe 只有 2330 | 只開啟 2330/date stream，outside-universe sentinel 從未 open |
| `M2-AC-08` | network-disabled、no-key backtest 與 inspect | 完整成功，零 HTTP request，結果與 prepared run 相同 |
| `M2-AC-09` | strategy 在 event 產生 order | intent 可立即 accept/reject，但 origin event 永不 fill；feedback 不前視 |
| `M2-AC-10` | pre-open/pre-close trial、緩跌／緩漲、closing result、CoolDown | indicative/unknown/CoolDown 不 fill；closing result 只評估較早 pending order |
| `M2-AC-11` | market/limit buy/sell、slippage、quantity cap 與多 order allocation | 僅用 subsequent evidence；limit 不被違反；partial fill 與 capacity consumption deterministic |
| `M2-AC-12` | fee、tax、unit size、multiplier、cash、position、mark 與 P&L | exact arithmetic/provenance 正確，missing economics 在 replay 前失敗 |
| `M2-AC-13` | ledger reconciliation 及人為破壞 | valid ledger 可由 records 重算；不一致使 run Failed |
| `M2-AC-14` | 10 runs、3 perturbations、cache hit/rebuild、debug/release | event/state/strategy/order/fill/ledger/result bytes 與 checksums 全部相同 |
| `M2-AC-15` | inspect successful、failed、degraded run | 不重跑即可看到 config、lineage、versions、counts、records、P&L 與 failure/degraded reasons |

live API acceptance 與 offline CI 必須分開：

- live／authorized evidence 證明實際 endpoint、coverage、cursor 與 published source。
- recorded contract tests 提供每次 CI 可重跑的 network error／cursor branches。
- network-disabled job 只使用已發布 local source/cache，不使用 mock HTTP 來冒充
  無網路。

## 20. Requirement traceability

| Requirement | M2 直接證據 |
| --- | --- |
| `DATA-01` | coverage/range、cursor、retry、resume、zero-record 與 second-sync tests |
| `DATA-02` | partition identity、immutable revision、atomic publish、offline reuse |
| `DATA-03` | completeness state、verify、Strict/degraded policy |
| `DATA-04` | cache lineage、invalidation、selective open、offline rebuild/reuse |
| `DATA-05`（TWSE 部分） | daily instrument、unit size、multiplier／currency provenance |
| `REPLAY-04` | event → strategy → current intents → older pending fills → feedback ordering |
| `REPLAY-05` | frozen universe、bounded stream reader、selective open |
| `REPLAY-06` | source/cache/runtime failure 與 degraded diagnostics |
| `STRAT-01` | OrderIntent、transactional sink、feedback、no-lookahead |
| `SIM-01` | market/limit、eligibility、slippage、partial fill、allocation、trace |
| `SIM-02` | order/fill records、cash、position、P&L、mark、reconciliation |
| `OPS-01` | one config、plan/sync/verify/replay/backtest/inspect/run |
| `OPS-02` | lineage、versions、orders、fills、positions、performance、checksums |
| `NFR-01` | repeated/cache/debug-release deterministic equality |
| `NFR-02`（baseline） | sync/cache/backtest timing、I/O、memory 與 throughput report |
| `NFR-03` | offline boundary、secret scan、version compatibility |

正式 requirement、design、implementation 與 evidence paths 由
[traceability matrix](../traceability.yaml)維護。M2 完成前不得只把本文件列為
`verification_evidence`。

2026-07-31 的 TWSE 2330 reference slice 已完成 live sync、offline sandbox、
cache rebuild、10-run、debug/release、artifact inspection 與 secret scan；實際
identity、checksum及結果見
[M2 reference acceptance](../verification/m2-acceptance.md)。

## 21. Completion criteria

M2 只有在下列條件全部成立時完成：

- data-sync、execution-sim、local-data、CLI 及 M2 verification contracts 已 review。
- authorized reference source 由 Teralion 完整 cursor sync 並 atomic publish。
- source manifest 可重算 checksum、counts、query 與 provenance。
- source只保存 per-page/daily-instrument `.json.zst`，雙 checksum及 streaming decode
  已驗證，沒有 uncompressed JSON source file。
- 第二次 sync 對 complete source 發出零 HTTP requests。
- cache 可 reuse、可刪除、可離線重建，且不修改或重新下載 source。
- replay/backtest 只開啟 declared universe streams。
- strategy order intent、feedback 與 origin-event no-fill 已自動驗證。
- market／limit、trade／quote evidence、slippage、partial fill 及 deterministic
  allocation 已驗證。
- fee、tax、instrument economics、cash、position、realized／unrealized P&L 與
  reconciliation 已驗證。
- successful／failed／degraded artifacts 可由 inspect 正確呈現。
- debug、release、no-key、network-disabled、repeated 及 perturbation suites 通過。
- golden event/state/strategy/order/fill/ledger/result checksums 已 review。
- traceability 登錄實際 code paths 與 machine-readable evidence。
- 沒有 secret、未解釋的 `NotRun`、`Blocked` 或 failed required test。

## 22. 建議實作切分

每一步應形成小型、可獨立 review 的 commit；不得一次同時重寫 data、replay、
simulation 與 CLI：

1. 完成 data-sync、local-data、execution-sim、CLI M2 design 與 verification plan。
2. 定義 versioned config、effective plan、partition identity 及 completeness types。
3. 實作 Teralion client boundary、sanitized request identity 與 cursor state machine。
4. 實作 per-page zstd staging、雙 checksum、manifest、atomic source publish及
   immutable revision。
5. 實作 verify、state classification、reuse／repair planning 與 second-sync no-op。
6. 定義 replay cache descriptor、canonical payload、builder、reader、invalidation。
7. 將 M1 in-memory replay 接到 bounded cache stream 與 frozen ReplayPlan。
8. 定義 OrderIntent、order/fill identities、transactional validation 與 feedback API。
9. 實作 TradingEligibility gate、TopOfBook／TradePrint fill 及 origin-event guard。
10. 實作 quantity allocation、partial fill、slippage、fee、tax 及 instrument economics。
11. 實作 AverageCost ledger、marking、P&L、reconciliation 與 canonical results。
12. 擴充 CLI plan、sync、verify、replay、backtest、inspect、run 與 atomic publisher。
13. 建立 live API、recorded contract、offline、golden、determinism、failure 與
    performance evidence。
14. 更新 acceptance、traceability、operations runbook，完成 formal M2 review。

每一步先跑 focused tests，再跑 workspace debug／release。任何 downstream design
若發現本 increment 無法安全實作，必須先更新本文件或提出 ADR，不得以暫時
JSON、隱式 default、runtime fallback 或未版本化 model 繞過。
