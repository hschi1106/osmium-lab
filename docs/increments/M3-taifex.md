# M3：TAIFEX 期貨與多商品回播

## 1. 文件目的

本文件定義 `osmium-lab` 第三個可獨立驗證的增量：在 M2 已通過的 TWSE `2330`
單商品離線回測基礎上，加入三個代表不同 session profiles 的 TAIFEX futures
reference instruments，完成真實來源資料正規化、exchange trading-date 歸屬、
`BookSnapshot`／`TradeBatch`、多 stream deterministic merge、每商品獨立市場
狀態、futures multiplier 與 P&L。

```text
increment_contract_version = 1
milestone                  = M3
status                     = Draft
reference_market           = TAIFEX
reference_profiles         = [
  index_futures_after_hours,
  stock_futures_after_hours,
  stock_futures_regular_only
]
reference_symbols          = [TXFH6, CDFH6, CAFH6]
reference_trading_date     = 2026-07-20
companion_instrument       = TWSE 2330
acceptance_universe_size   = 4
source_evidence            = docs/verification/evidence/m3/source-selection-2026-07-31.yaml
fixture_redistribution     = Approved
```

三個 `reference_symbols` 是 Teralion `2026-07-20` daily instrument collection
實際回傳的 exact symbols，不是依 TAIFEX 商品代碼慣例推測。原先偏好的
`2026-07-27` 缺少兩個 after-hours profiles 的午夜前 records；`2026-07-28`
雖有 symbol-level payload，store-wide coverage 卻缺少 `taifex_fut` bucket。
因此依 entry gate 改選第一個同時具 coverage、跨午夜 source、三個 regular
partitions 與 TWSE `2330` metadata 的 `2026-07-20`。選擇證據見
[M3 source selection evidence](../verification/evidence/m3/source-selection-2026-07-31.yaml)。

本文件不定義尚未由 fixture 證實的 JSON 欄位或 `format` mapping。實際 wire
contract 必須在 [TAIFEX interface](../interfaces/taifex.md)由真實 tick 與官方文件
固定後，normalizer implementation 才能開始。

產品範圍、術語與優先順序以[產品需求](../product-requirements.md)為準。詳細需求
來自：

- [資料需求](../requirements/data.md)
- [回播需求](../requirements/replay.md)
- [策略需求](../requirements/strategy.md)
- [模擬與帳務需求](../requirements/simulation.md)
- [操作與非功能需求](../requirements/operations.md)
- [Session Plan ADR](../architecture/decisions/0003-session-windows-and-strategy-activation.md)
- [TradingContext ADR](../architecture/decisions/0004-trading-context-and-eligibility.md)

## 2. M2 baseline 與 M3 delta

M3 承接並保持下列 M2 invariants：

- verified local source 可跨 backtest 重用。
- replay cache 是可刪除、可離線重建的 derived artifact。
- Teralion wire type 與 domain event 分離。
- `match_time` 是唯一 replay time。
- `OrderingRule` 使用版本化 deterministic tie-break。
- 每個 event 先更新 MarketState，再呼叫 strategy。
- strategy 只能讀取 market state，不能修改 replay clock 或歷史事件。
- order 最早只能使用後續同商品 eligible event 判定 fill。
- backtest、replay 與 inspect 預設不存取網路。

M3 只增加：

1. 三個 TAIFEX futures source／metadata／session profiles。
2. TAIFEX `TradeBatch` 與完整五檔 `BookSnapshot` mapping。
3. TAIFEX `I022` 開盤試算映射為 `IndicativeOpeningAuction`，且不宣稱存在 futures
   `IndicativeClosingAuction`。
4. 跨日 `after_hours` 與 `regular` 歸屬同一 exchange trading date。
5. TWSE 與 TAIFEX 多 stream bounded k-way merge。
6. config／CLI 從固定單 partition 擴充為 explicit multi-instrument universe。
7. futures-specific multiplier accounting 與缺漏 provenance failure。
8. 實際 M3 dataset 的 throughput、I/O 與 peak-memory baseline。

不得為了 M3 重寫已通過的 M1／M2 goldens，除非 domain contract 確實變更且有獨立
ADR、version bump 與 migration／rebuild policy。

## 3. 交付結果

M3 完成時，使用者必須能以一份版本化設定執行：

```text
TWSE 2330 verified source/cache
        +
TAIFEX index futures with regular + after_hours
        +
TAIFEX stock futures with regular + after_hours
        +
TAIFEX stock futures with regular only
        |
        v
freeze multi-instrument ExecutionPlan
-> open only selected symbol/date/segment streams
-> bounded k-way merge by match_time and OrderingRule
-> update per-instrument MarketState atomically
-> run one read-only multi-instrument strategy
-> simulate same-instrument subsequent-event fills
-> apply equity or futures accounting by explicit instrument model
-> reconcile every instrument and account total
-> publish deterministic inspectable artifacts
```

資料準備與 backtest 必須分開。三個 TAIFEX partitions 的 source 與 cache 完成後，
TWSE + TAIFEX 四商品 reference backtest 必須在無網路、無
`TERALION_API_KEY` 的環境成功。

## 4. Entry gates

M3 implementation 開始前必須滿足：

1. M1 與 M2 formal acceptance 為 `Passed`，且目前 branch 的 workspace debug／
   release tests 通過。
2. 三個 reference TAIFEX fixtures 的取得、保存及 private repository 使用具有
   明確 authorization 與 provenance。
3. Teralion coverage 證明三個 exact TAIFEX symbols 與 selected trading date
   可用。
4. 每個 daily instrument payload 保存 exact symbol、market、instrument kind、
   expiry 及來源實際提供的其他欄位；缺漏欄位保持 unknown。
5. index futures 與 after-hours stock futures fixtures 同時涵蓋 `after_hours` 與
   `regular`，包含午夜兩側與 segment boundary 附近的有效 `match_time`；
   regular-only stock futures fixture 涵蓋完整 `regular` 並由 session profile
   證明 `after_hours` 不適用。
6. 三種 profiles 的 observed formats、record composition、price／quantity
   units、source counters、flags 與 batch ordering 已記錄於
   [TAIFEX interface](../interfaces/taifex.md)。
7. TAIFEX calendar、trading-date ownership、session profile 及例外日來源具有版本
   或 stable identity。
8. 三個 instruments 的 multiplier 由 verified metadata 或 explicit config 提供，
   且 provenance、applicable instrument／date 已固定。
9. 每個 fixture checksum、secret scan、record counts 與 redistribution scope
   已 review。

若缺少上述任何 gate，M3 狀態維持 `Draft`／`Blocked`，可以先實作與來源無關的
multi-stream property tests，但不得把猜測 payload 當作 TAIFEX format evidence。

## 5. 範圍

### 5.1 包含

- 一個具有 `regular` 與 `after_hours` 的股價指數期貨 exact symbol，例如台指期貨
  profile。
- 一個具有 `regular` 與 `after_hours` 的股票期貨 exact symbol。
- 一個只有 `regular`、不適用 `after_hours` 的股票期貨 exact symbol。
- TAIFEX reference instruments 合計三個，且三者 symbol 不得重複。
- 一個已結束 exchange trading date。
- 前兩個 instruments 的 `after_hours` 與 `regular` segments。
- regular-only stock futures 的 `regular` segment 與不適用 `after_hours` 的
  planning evidence。
- 同一 trading date 內跨越兩個 calendar dates 的 index／stock futures
  after-hours match times。
- 三個 instruments 實際 observed 的 trade、trade batch 與完整五檔 snapshot
  formats。
- `TradeBatch` 與 `BookSnapshot` domain events。
- `I022` 的 `IndicativeOpeningAuction` domain event；`0/0` 保留為
  `NoObservation`，不得當作 actual trade 或 fill evidence。
- TAIFEX raw flags／status 的無損保存及 unknown warning。
- TWSE `2330` 與三個 TAIFEX futures 的 explicit four-instrument universe。
- 每 instrument/date/segment 的 source planning、sync、verify、cache build／reuse。
- bounded multi-stream merge 與每商品獨立 MarketState。
- 一個 compile-time linked multi-instrument Rust acceptance strategy。
- TWSE equity 與 TAIFEX futures 各自的 instrument economics。
- futures position、fee、realized／unrealized P&L 與 final marking。
- strict offline backtest、artifact inspection、determinism 與 performance baseline。

### 5.2 不包含

- TAIFEX options、weekly options 或組合式商品。
- 動態近月／換月選擇；universe 仍使用 exact symbol。
- continuous contract、back-adjustment 或 rollover。
- 跨 trading date 的持倉延續。
- exchange matching engine、逐筆委託簿或 queue position。
- implied order、spread leg reconstruction 或 hidden liquidity。
- margin requirement、variation-margin settlement、risk limit、liquidation 或
  default handling。
- exchange fee／tax／multiplier 的自動猜測。
- multi-currency 或 FX conversion；reference account 仍為 TWD。
- stop、IOC、FOK、cancel／replace 或複雜 order lifecycle。
- Teralion `close`／`stats` 作為 domain event 或 MarketState input。
- TPEx、權證與其他 M4 markets。
- parallel replay；M3 只要求 bounded deterministic merge，不要求 concurrency。

來源保存可以保留 `close`／`stats` raw payload，但 normalizer 必須明確分類為
unsupported non-timeline records；它們不得因只有 `received_at` 而插入 replay。

## 6. Reference dataset 與 fixture

### 6.1 Instrument selection

三個 reference instruments 都必須：

- 是 TAIFEX futures，不是 option。
- 在同一 selected trading date 有完整 `regular` source。
- Teralion 提供至少一種完整五檔 format 及一種成交／成交批次 format。
- 可以由 daily instrument 或 explicit reviewed config 取得各自 multiplier。
- 與 TWSE reference date 有可比較的時間範圍。

profile selection：

| Profile | Required sessions | Selection evidence |
| --- | --- | --- |
| 股價指數期貨 | `after_hours` + `regular` | official session profile、Teralion coverage 與 daily instrument |
| 股票期貨（盤後適用） | `after_hours` + `regular` | underlying/session applicability、Teralion coverage 與 daily instrument |
| 股票期貨（日盤限定） | `regular` only | official non-applicability evidence、Teralion coverage 與 daily instrument |

股價指數期貨以台指期貨 profile 為優先，因
[ADR-0003](../architecture/decisions/0003-session-windows-and-strategy-activation.md)
已記錄其一般 session family；但 exact contract symbol 只能由 Teralion
coverage／daily instrument 決定。

兩個股票期貨必須是不同 exact symbols，並以 instrument-specific session metadata
證明一個適用盤後交易、另一個不適用。不得只因盤後 query 回傳零 records 就推論
商品不適用 `after_hours`。

### 6.2 Required fixture coverage

三份 fixtures 合計至少涵蓋：

- 股價指數期貨與盤後適用股票期貨各自 `after_hours` open／close boundary 附近的
  source records。
- 兩個 after-hours instruments 各自有 `match_time` 位於午夜前與午夜後的
  accepted events。
- 三個 instruments 各自的 `regular` open、continuous trading 與 close 附近
  records。
- 每個 instrument 至少兩次完整五檔變化。
- 每個 observed trade-batch format 至少一個多筆成交 batch；若實際來源只有單筆，
  必須記錄此事，不能合成來源證據。
- 至少一組相同 `match_time` 的 TAIFEX records；若真實資料沒有，另以明示
  `synthetic_domain` case 測 tie-break，不修改 fixture。
- 可保存但不臆測命名的 raw flags／status。
- source 提供的累計成交量或明確 absent evidence。
- 至少一筆 `close`／`stats` 或其他 non-timeline format；若 coverage 中實際存在，
  用來證明它被保存但不正規化。

### 6.3 Fixture provenance

fixture metadata 至少記錄：

```text
provider
source_product
market
exact_symbols
instrument_kind
expiry if available
exchange_trading_date
calendar dates covered
selected session segments per instrument
requested received_at windows
observed match_time bounds
source formats and counts
source page identities and checksums
daily instrument checksum
selection/removal policy
fixture-set checksum
redistribution approval
secret scan result
```

不得提交 credential、authorization header、full cursor、signed URL 或未獲准
散布的原始資料。

### 6.4 Phase 1 frozen selection

| Profile | Exact symbol | Teralion metadata | Official identity | Sessions | Economics |
| --- | --- | --- | --- | --- | --- |
| 股價指數期貨 | `TXFH6` | `kind=index`、`expiry=2026-08` | 臺股期貨 `TX` | `after_hours` + `regular` | TWD 200／指數點／口 |
| 股票期貨（盤後適用） | `CDFH6` | `kind=equity`、`expiry=2026-08` | 台積電期貨 `CD`、underlying `2330` | `after_hours` + `regular` | 2,000 shares／口 |
| 股票期貨（日盤限定） | `CAFH6` | `kind=equity`、`expiry=2026-08` | 南亞期貨 `CA`、underlying `1303` | `regular` | 2,000 shares／口 |

Teralion daily instrument 的 `multiplier`、`currency` 與 `underlying` 在三個
reference instruments 都是 `null`；M3 不得把上表的 official identity 寫回成
來源實際提供的欄位。economics 必須由帶 TAIFEX reference provenance 的 explicit
config 提供，並在 plan identity 保存。

local gitignored acquisition 已完成五個 partitions、156 個 cursor pages 與 769,214
筆 records，checksum manifest 及 secret scan 通過。兩個 after-hours partitions
各自包含 `2026-07-17` 午夜前及 `2026-07-18` 午夜後 records；三個 instruments
都包含完整五檔、multi-trade batch、相同 `match_time` occurrences 及 raw
`close`／`stats`。

這項 evidence 關閉 exact symbol、date、coverage、local acquisition 與
redistribution gates。repository owner 已明確授權將 page-aligned recorded subset
提交至 private `hschi1106/osmium-lab` 供 internal use；17 個 shards 共 74,214 筆，
全體 fixture SHA-256 為
`10972c8a6ee8e58704c3fdbbbcd6d95f37ac8eb4a4519dcdb14429492275ddaf`。
三個 fixture metadata 分別記錄 selection、removal、source/shard checksum 與
approval scope；授權不延伸至 public repository、fork、release、package 或其他
export。

## 7. Trading date 與 SessionPlan

### 7.1 Exchange trading-date ownership

TAIFEX `trading_date` 是 exchange business date，不是由 `match_time` 的 local
calendar date、UTC date 或檔案日期直接推導。

對同一 selected trading date：

- 股價指數期貨與盤後適用股票期貨的 `after_hours` 可以從前一個適用 exchange
  calendar date 開始，跨過午夜並在 selected trading date 清晨結束。
- 三個 instruments 的 `regular` 位於 selected trading date 日間。
- 日盤限定股票期貨不得 materialize `after_hours`；config 選取該 kind 必須在
  planning 時失敗。
- weekend、holiday、補交易日、到期日提早收盤或取消盤後等例外，必須由
  versioned calendar／session profile materialize。
- 「前一日」表示前一個適用 exchange calendar date，不是固定減 24 小時。

無法證明 tick 屬於 selected trading date 時，source partition 不得發布為
`complete`。

### 7.2 Materialized segments

每個 segment 分別使用 ADR-0003 的固定五分鐘 margin：

```text
download window = [official open - 5m, official close + 5m) by received_at
replay window   = [official open - 5m, official close + 5m) by match_time
```

reference plan 必須保存：

- calendar、session profile 與 policy versions。
- absolute open／close 及 timezone。
- logical trading date 與實際 calendar-date bounds。
- download／replay windows。
- source partition 與 cache binding。

strategy 只宣告 `after_hours`／`regular`，不得自行填入絕對時鐘或改變 margin。

### 7.3 Segment boundaries

M3 reference 使用明確的 segment policy：

- 每個 segment 的 MarketState boundary action 由 TAIFEX profile 固定並進入 plan
  identity；不得默認沿用 M2 TWSE policy。
- reference acceptance 使用 `ResetObservableFields`：在新 segment 第一個 event
  的同一 atomic transition 中，先將 book、recent trade、cumulative volume 與
  annotations 重設為 unavailable，再套用目前 event；state version 仍只增加一次。
- 不因 clock 穿越 open／close 合成 market event 或 strategy callback。
- segment 結束時，未完成的 segment-scoped `Day` order 以
  `Cancelled(SegmentEnd)` 結束，不得無聲跨到下一 segment。

這項 policy 不宣稱 exchange book 在實體上消失，而是避免平台在長 session gap
後把 stale observation 當成目前可用狀態。若後續來源證據與使用需求支持 carry，
必須建立不同 MarketState profile version，不得改寫既有 acceptance identity。

## 8. TAIFEX normalization contract

### 8.1 Wire/domain boundary

Teralion response type 只存在於 TAIFEX adapter／normalizer。Strategy、replayer、
MarketState 與 simulation 只接收 domain type。

normalizer 必須：

1. 驗證 market、exact symbol、trading date、format 與必要欄位。
2. 以 exact decimal／integer 解析 price、quantity、volume 與 source counters。
3. 拒絕缺少或無效 `match_time` 的 timeline record。
4. 將已證實的成交 record 轉成 `TradeBatch`。
5. 將 `I022` calculated opening record 轉成 `IndicativeOpeningAuction`；`0/0` 使用
   `NoObservation`。
6. 將已證實的完整五檔 record 轉成 `BookSnapshot`。
7. 保留 source order、raw flags、format 及可用 source sequence。
8. 將 `close`／`stats` 與 unknown format 明確分類；I070／I072 不映射為
   `IndicativeClosingAuction`。
9. 產生帶 market、symbol、trading date、format 與 source context 的 stable error。

### 8.2 `BookSnapshot`

每個 accepted `BookSnapshot` 必須包含：

- TAIFEX instrument identity。
- source format。
- exchange trading date。
- `match_time`。
- 完整且合法的 bid／ask 五檔。
- source tick 同時提供的可用成交、累計量及 raw annotations；若 domain schema
  支持且 fixture 證實。

新的完整 snapshot 完整取代舊 book。不得由連續 snapshot 推論新增、取消、
hidden quantity、queue rank 或真實撮合順序。

若 source record 同時包含不可分割的 book 與 trade，但目前 `BookSnapshot` domain
payload 無法原子表示，必須先 version event schema；不得把同一 source tick 任意拆
成不同時間點。

### 8.3 `TradeBatch`

每個 accepted `TradeBatch` 必須：

- 保存 batch 中全部已確認 trades。
- 保存 source 已提供的 trade order；若來源未定義順序，interface 必須明示限制。
- 使用同一 source tick 的單一 atomic event。
- 保存可用 cumulative volume 與 raw annotations。
- 不清除或合成 book。

batch 內 trade count、price、quantity 或 cumulative relationship 不合法時，拒絕
整個 event，不發布 partial batch。

### 8.4 Unknown 與 unsupported

- unknown timeline format：Strict mode 停止。
- observed `close`／`stats`：保存 raw source、記錄 count、replayer 忽略。
- unknown raw flags：保留 raw value並產生 warning，不自行命名。
- invalid price／quantity／book depth：拒絕，不以零或前值填補。
- 缺少 multiplier：不阻止純 replay cache；需要 simulation／P&L 時在 event 0
  前停止。

## 9. Source、verification 與 cache

M3 沿用 M2 immutable source、per-page zstd、dual checksum、atomic publish 與
cursor state machine，但 repository identity 必須真正支援多 partition：

```text
source + market + symbol + trading_date + logical segments + revision
```

不得再使用 global `current` pointer、固定 attempt name 或 index `[0]` 代表唯一
partition。

每個 partition 分別分類：

- `Missing`
- `Building`
- `Complete`
- `Incomplete`
- `Corrupt`

`sync` 只下載 plan 判定缺少或需恢復的 partitions。TWSE source/cache 已完整時，
加入 TAIFEX 不得重新下載或覆寫 TWSE。

每個 cache stream 至少綁定：

```text
instrument + trading_date + selected segments
source revision and checksum
TAIFEX mapping version
event/canonical schema versions
ordering rule version
calendar/session/profile versions
event count and ordering bounds
payload checksum
```

cache reader 必須先驗證 descriptor、checksum、bounds 與 EOF；runtime 不得在
invalid cache 時隱式 fallback 到 raw source 或網路。

## 10. Multi-stream replay

### 10.1 Selected streams

ExecutionPlan 對每個 `instrument + trading_date` 建立一個或多個已排序 stream
binding。正式 replay 只開啟 explicit universe 需要的 bindings。

acceptance 必須放置至少一個 universe 外 sentinel cache，證明：

- reader factory 沒有 open sentinel。
- 沒有讀取或 decode sentinel payload。
- result manifest 沒有 sentinel lineage。

### 10.2 Bounded k-way merge

merge 最多只保留每個 opened stream 的 head event 與 bounded reader buffer：

```text
memory = O(opened streams + configured bounded buffers)
```

不得先將 TWSE 與 TAIFEX 完整期間載入同一 `Vec` 再排序作為正式路徑。

下一 event 依完整 `OrderingKey` 選取：

1. `match_time`
2. versioned tie-break fields
3. canonical fingerprint

同 `match_time` 的不同 market／symbol 不代表真實因果順序；平台只保證
deterministic callback sequence。合法 duplicate 不得去重。

### 10.3 Clock、state 與 callbacks

- global ReplayClock 只依 selected next event 的 `match_time` 前進。
- 每 instrument 有獨立 MarketState 與 state version。
- 目前 event 只更新所屬 instrument state。
- strategy callback 可以讀取 universe 中所有「截至目前已處理」的 states。
- 尚未收到 event 的另一商品 state 保持 unavailable。
- 同 `match_time` 較後 event 在處理前不可見。

stream discovery order、filesystem order、hash-map iteration、worker completion 或
local timezone 不得改變 output。

## 11. Strategy 與 simulation

### 11.1 Multi-instrument strategy

M3 acceptance strategy 以 compile-time linked Rust type：

- 宣告 TWSE `2330` 與三個 exact TAIFEX symbols。
- 對兩個盤後適用 instruments 宣告 `after_hours` + `regular`，對日盤限定股票
  期貨只宣告 `regular`。
- 對目前 event 及已處理 states 產生 deterministic indicators。
- 對三個 TAIFEX instruments 各發出至少一組可平倉的 order intents。
- 不讀 fixture future ordinal、golden price、next event 或 final state。

strategy declaration 順序不得改變 canonical universe 或 plan identity。

### 11.2 Fill isolation

pending order 只能由相同 instrument 的 subsequent eligible event 評估：

- TWSE event 不得 fill TAIFEX order。
- TAIFEX event 不得 fill TWSE order。
- origin event 不得 fill current intent。
- 另一商品即使具有相同 `match_time` 也不是該 order 的 price／quantity evidence。
- `BookSnapshot` 使用 TopOfBook evidence 時只讀該 snapshot。
- `TradeBatch` 使用 TradePrint evidence 時只讀該 batch。

quantity allocation 只在相同 instrument／event／account 的 pending orders 間分配。
不得跨商品共用 displayed quantity。

### 11.3 Segment end

`SegmentDayV1` 是 M3 reference TIF policy：

- intent 在 origin segment 內 pending。
- segment replay window 完成時尚未 filled 的 remainder deterministic cancel。
- cancellation 不需要虛構 market event。
- cancellation record 保存 segment identity、最後 processed occurrence 及 reason。
- strategy 可以在下一個真實 callback 收到既有 feedback；若 segment 後沒有 event，
  finalization 仍必須保存 cancellation，不產生新的 order intent。

## 12. Instrument economics 與 accounting

### 12.1 Per-instrument binding

M3 將 M2 單一 economics 擴充為 exact instrument keyed bindings：

```text
InstrumentEconomics {
    instrument
    instrument_class
    quantity_unit
    units_per_trading_unit
    currency
    multiplier
    fee_model
    tax_model
    accounting_model
    value_source
    source_version
    applicable_trading_date
}
```

每個 universe instrument 恰有一個 compatible binding。duplicate、missing、
conflicting 或 universe 外 binding 在 replay 前失敗。

### 12.2 Futures accounting

TAIFEX reference 使用 `FuturesRealizedPnlV1`，不得套用 equity full-notional cash
flow：

- position quantity 以明確 futures contract unit 表達。
- 開倉不從 cash 扣除完整 contract notional。
- 每次 fill 仍依 configured model 扣除 fee／tax。
- 平倉 realized P&L：

```text
(exit_price - entry_price)
* signed_closed_contracts
* multiplier
```

`signed_closed_contracts` 使用被關閉原 position 的方向：long 為正、short 為負。
position reversal 必須先計算 closed portion，再以 fill price 建立反向 position 的
新 average entry。

- final unrealized P&L：

```text
(final_mark - average_entry_price)
* signed_open_contracts
* multiplier
```

- 所有 arithmetic 使用 exact decimal 及 checked operation。
- final mark 只能來自 replay 結束前同商品合法 trade 或明確 configured fallback。

M3 不模擬 initial margin、maintenance margin、variation settlement 或 liquidation。
結果必須標示 accounting model 是簡化的 transaction P&L，不宣稱等同期貨商帳戶
權益流程。

### 12.3 Multiplier provenance

multiplier 可以來自：

1. Teralion daily instrument 的 verified field。
2. 使用者 explicit config。
3. 後續 versioned reference-data source。

不得依 symbol、商品名稱或「市場慣例」猜測。結果必須保存 value、source、
version、applicable date 及 checksum。來源衝突時停止。

純 replay 可以在 multiplier unknown 時執行並標示 simulation unavailable；
backtest／P&L 必須在 event 0 前拒絕。

### 12.4 Reconciliation

successful multi-instrument run 必須驗證：

- 每個 fill 關聯正確 instrument、order 與 triggering occurrence。
- 每 instrument position、average entry、realized／unrealized P&L 可由 fills 重建。
- TWSE 使用 equity accounting，TAIFEX 使用 futures accounting。
- account cash 等於 initial cash、equity cash flows、futures realized P&L 及全部
  charges 的 deterministic total。
- final positions 與 performance summary 一致。

任一 instrument reconciliation failure 使整個 run `Failed`。

## 13. Config、planning 與 CLI

M3 config schema 必須能表達 list-based universe，不再接受只有固定
`twse/2330/2026-07-27` 的 resolver：

```text
RunConfig {
    config_version = 2
    trading_dates
    instruments[] {
        market
        symbol
        session_kinds
        instrument_economics
    }
    strategy_binding
    data_policy
    simulation
    output_policy
}
```

要求：

- M2 config version 1 行為保持相容，或以明確 unsupported/migration error 拒絕；
  不得靜默改變 identity。
- universe、dates、sessions 與 economics canonicalize 後產生 config checksum。
- config order 不改變 semantic identity。
- planner materialize 所有 instrument/date partitions，不使用 `[0]`。
- `plan` 顯示每 partition 的 source/cache action、network requirement 與 failure。
- `sync` 只處理需要 network 的 actions。
- `verify`、`replay`、`backtest`、`inspect` 永不建立 transport。
- `run` 可以逐 partition sync，但任一 Strict partition 未完成時不開始 backtest。

CLI command spelling沿用 [CLI 操作契約](../operations/cli.md)。schema／output
變更必須同步更新該文件。

## 14. Error 與 degraded policy

Strict M3 reference acceptance 遇到下列情況必須停止：

- TAIFEX trading-date ownership 無法判定。
- calendar/session profile 缺少或不相容。
- unknown timeline format。
- 缺少／無效 `match_time`。
- invalid book、trade、price、quantity 或 batch shape。
- source/cache checksum、count、bounds 或 EOF 不符。
- selected stream ordering regression。
- missing/conflicting multiplier 或 accounting model。
- order 使用不同 instrument event 作 fill evidence。
- reconciliation failure。

ExplicitDegraded 可以逐 scope 略過完整的 instrument／date／segment／format，但：

- 不接受 corrupt bytes。
- 不拆散 source atomic event。
- 不破壞 global ordering 或 no-lookahead。
- 不把 unknown 值改成零。
- 不允許缺少 TAIFEX multiplier 的 degraded P&L。
- completion quality、plan identity、warnings 與 artifacts 必須不同於 Strict。

M3 golden P&L 只使用 Strict run。

## 15. Run artifacts

M3 沿用 M2 artifact set，並增加或擴充：

```text
execution-plan.yaml
data-lineage.yaml
cache-lineage.yaml
session-plans.yaml
instrument-economics.yaml
event-stream.blake3
final-state.blake3
strategy-output.bin
orders.bin
fills.bin
ledger.bin
positions.yaml
performance.yaml
warnings.yaml
run-summary.yaml
```

manifest 至少記錄：

- 全部 selected instruments／dates／segments。
- 每 partition source revision 與 cache identity。
- calendar、session、mapping、event、ordering 與 accounting versions。
- per-instrument event/state/order/fill/position/P&L counts。
- global checksum 及 per-stream checksums。
- multiplier values 與 provenance。
- skipped/unsupported raw record counts。
- peak memory、bytes read 與 elapsed reference metrics；不參與 domain identity。

`inspect` 必須先驗證所有 attachments，並能按 instrument 顯示 lineage、final
state、orders、fills、position 與 P&L，不重跑 strategy。

## 16. Performance baseline

M3 必須完成首個實際 multi-market benchmark，但在 baseline review 前不任意設定
數值 threshold。

datasets：

1. 股價指數期貨單 trading date，包含跨日 session。
2. 盤後適用股票期貨單 trading date，包含跨日 session。
3. 日盤限定股票期貨單 trading date。
4. 三個 TAIFEX futures 合併 replay。
5. TWSE `2330` + 三個 TAIFEX futures 的 four-instrument replay。

release profile 至少記錄：

- source/cache bytes。
- normalization throughput。
- cache build duration。
- cache-hit replay events/second。
- end-to-end backtest duration。
- peak resident memory。
- bytes read／written。
- opened stream count。

結構性驗收：

- cache hit 不重新解析 raw JSON。
- TWSE complete source 不因加入 TAIFEX 而重新下載。
- sentinel stream 不 open。
- peak event memory 不與總 event count 線性成長。
- 重複執行不因效能選項改變 domain bytes。

結果記錄於[效能驗證](../verification/performance.md)。

## 17. Acceptance criteria

| ID | 驗證項目 | Pass 條件 |
| --- | --- | --- |
| `M3-AC-01` | 三份 authorized TAIFEX fixtures 與 daily instruments | 三個 exact symbols 的 date/formats、checksums、provenance、secret scan 全部可追溯 |
| `M3-AC-02` | trading-date ownership | 兩個 after-hours profiles 的午夜前後及三個 regular records 全部歸屬正確 trading date |
| `M3-AC-03` | session plans | 兩個 after-hours、三個 regular windows 及 regular-only rejection 正確 |
| `M3-AC-04` | TAIFEX normalization | 三個 instruments 的 observed trades／books 轉為 atomic `TradeBatch`／`BookSnapshot` |
| `M3-AC-05` | unsupported records | close／stats 保存 raw 但不進 timeline；unknown format Strict failure |
| `M3-AC-06` | MarketState | book replacement、trade batch、segment boundary 與 state version 正確 |
| `M3-AC-07` | multi-stream ordering | shuffled discovery/input 仍產生相同 four-instrument event bytes 與 checksum |
| `M3-AC-08` | bounded/selective I/O | 只開 selected streams，sentinel 不 open，memory bounded |
| `M3-AC-09` | no-lookahead | callback 只看到目前及較早跨市場 events/states |
| `M3-AC-10` | fill isolation | order 只由同商品 subsequent eligible event fill |
| `M3-AC-11` | segment order lifecycle | pending SegmentDay order在 segment end deterministic cancel |
| `M3-AC-12` | multiplier provenance | 三個 futures 各有 verified/explicit value；missing/conflict 在 event 0 前失敗 |
| `M3-AC-13` | futures accounting | multiplier-based realized/unrealized P&L、fee 與 cash reconciliation exact |
| `M3-AC-14` | offline workflow | source/cache ready 後無 key、network denied 的 replay/backtest/inspect 成功 |
| `M3-AC-15` | determinism | 10 runs、3 discovery perturbations、cache hit/rebuild、debug/release bytes 相同 |
| `M3-AC-16` | inspection/corruption | inspect 顯示 per-instrument results，任一 attachment 竄改被拒絕 |
| `M3-AC-17` | performance baseline | 三種 profiles、TAIFEX 合併及 four-instrument metrics／identity 完整 |

acceptance evidence 必須同時包含：

- authorized live sync report。
- recorded normalizer／cursor／failure fixtures。
- network-disabled offline report。
- event/state/strategy/order/fill/ledger/result checksums。
- stream-open audit。
- multiplier/accounting trace。
- benchmark report。

## 18. Requirement traceability

| Requirement | M3 直接證據 |
| --- | --- |
| `DATA-01` | TAIFEX coverage、cursor、daily instrument、cross-day windows |
| `DATA-02` | multi-partition immutable source 與 atomic publish |
| `DATA-03` | per-partition completeness、Strict/degraded failure |
| `DATA-04` | per-symbol/date/segment cache、selective open、offline rebuild |
| `DATA-05` | TAIFEX trading date、expiry/multiplier provenance |
| `REPLAY-01` | TAIFEX `TradeBatch`／`BookSnapshot` fixture mapping |
| `REPLAY-02` | TWSE + TAIFEX deterministic ordering |
| `REPLAY-03` | per-instrument state 與 segment boundary |
| `REPLAY-04` | cross-market callback ordering與 no-lookahead |
| `REPLAY-05` | explicit universe、bounded k-way merge、sentinel audit |
| `REPLAY-06` | format/time/book/trading-date/multiplier errors |
| `STRAT-01` | read-only multi-instrument declaration、context、intent、feedback |
| `SIM-01` | same-instrument fills、book/trade evidence、segment cancellation |
| `SIM-02` | equity/futures accounting dispatch、multiplier P&L、reconciliation |
| `OPS-01` | multi-partition plan/sync/verify/replay/backtest/inspect/run |
| `OPS-02` | per-instrument lineage、economics、results、checksums |
| `NFR-01` | repeated/perturbed/cache/debug-release deterministic equality |
| `NFR-02` | actual TAIFEX 與 multi-market benchmark |
| `NFR-03` | offline boundary、secret safety、version/cache compatibility |

正式 evidence path 必須登錄於
[traceability matrix](../traceability.yaml)。本文件是 increment contract，不是
verification evidence。

## 19. Completion criteria

本 increment contract 的實作已涵蓋 shared-date TWSE `2330` 與三個 TAIFEX 商品；
formal acceptance 由 [`M3 acceptance`](../verification/m3-acceptance.md) 及
machine-readable report 記錄為四商品 `Passed`。TWSE fixture 的來源、抽取 predicate、
page checksums、daily instrument 與 redistribution approval 均保存在 fixture metadata。

M3 只有在下列條件全部成立時完成：

- 三個 exact TAIFEX symbols、共同 date 與 fixture approval gates 關閉。
- TAIFEX interface mapping 由 actual fixture 與官方文件 review。
- cross-day source、daily instrument 與 session/calendar provenance complete。
- `TradeBatch`／`BookSnapshot` normalizer 的 positive／negative fixture tests 通過。
- source/cache repository 支援多 partition，沒有固定 `[0]` 或 global-current 假設。
- bounded k-way merge 只開 selected streams。
- per-instrument MarketState、strategy view 及 no-lookahead tests 通過。
- same-instrument subsequent-event fill 與 segment cancellation tests 通過。
- futures multiplier、accounting、marking 與 reconciliation tests 通過。
- config version、CLI、operations 與 inspect 文件更新。
- debug、release、network-disabled、repeated、perturbation、cache rebuild suites 通過。
- 三種 TAIFEX profiles、TAIFEX 合併與 four-instrument performance baseline 完成。
- golden artifacts及 machine-readable acceptance report review。
- traceability 登錄實際 implementation paths 與 evidence。
- 沒有 secret、unexplained warning、required `NotRun`、`Blocked` 或 failed test。

## 20. 建議實作切分

每一步形成小型、可獨立 review 的 commit：

1. 取得三份 authorized TAIFEX samples，固定三個 exact symbols、共同 date、
   fixture provenance 與 redistribution gates。
2. 以 observed payload 完成 TAIFEX interface mapping 及 normalizer test catalog。
3. 定義 calendar/trading-date/session profile 與跨日 fixture tests。
4. 實作 TAIFEX wire parser、`TradeBatch` normalization 與 strict errors。
5. 實作完整五檔 `BookSnapshot` normalization 與 reducer profile。
6. 將 source repository、planner、cache descriptor 從 single current 擴充為
   instrument/date/segment keyed partitions。
7. 將 M2 config／CLI 從固定 reference resolver 擴充為 versioned explicit universe。
8. 實作 bounded k-way merge、stream-open audit 與 multi-market ordering tests。
9. 擴充 strategy context、same-instrument fill isolation 與 segment cancellation。
10. 實作 per-instrument economics binding 與 `FuturesRealizedPnlV1`。
11. 擴充 ledger、positions、performance、artifacts 與 inspect 的 per-instrument view。
12. 建立 offline、determinism、corruption、cache rebuild 與 performance harness。
13. 執行 live sync、formal M3 acceptance，更新 evidence、traceability 與 operations。

每一步先跑 focused checks，再跑：

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
```

若 fixture 顯示目前 domain schema 無法保持 source atomicity、TAIFEX trading-date
規則與 ADR 不相容，或 futures accounting 需要不同 contract，必須先更新本文件、
interface 或新增 ADR，不得用 hidden default、ad hoc JSON、calendar-date split 或
equity accounting 暫時繞過。
