# TAIFEX Teralion Interface

## 1. 文件目的

本文件固定 Teralion Feed Archive 的 `taifex_fut` wire record 如何進入
`osmium-lab` 的 TAIFEX market adapter。它是 normalizer 的 source contract，不是
Rust type definition；Teralion JSON 與 domain event 必須維持在不同邊界。

本文件只涵蓋 M3 已授權並提交的三個 exact instruments：

| profile | symbol | trading date | segments |
| --- | --- | --- | --- |
| 股價指數期貨 | `TXFH6` | `2026-07-20` | `after_hours`、`regular` |
| 盤後適用股票期貨 | `CDFH6` | `2026-07-20` | `after_hours`、`regular` |
| 日盤限定股票期貨 | `CAFH6` | `2026-07-20` | `regular` |

來源證據為 [M3 source selection evidence](../verification/evidence/m3/source-selection-2026-07-31.yaml)，
提交 fixture 位於 [`fixtures/teralion/taifex`](../../fixtures/teralion/taifex)。
格式名稱與官方 message semantics 依
[TAIFEX Market Data Transmission Manual v2.31.0S](https://www.taifex.com.tw/file/taifex/eng/eng11/TechDocs/19/Market_Data_Transmission_Manual_v2.31.0S.pdf)。

適用 mapping：`TeralionTaifexFutures`，`mapping_version = 2`。

## 2. 邊界與 invariant

資料流程固定為：

```text
Teralion wire object
-> envelope／format validation
-> exact decimal／integer parsing
-> TAIFEX format mapping
-> validated domain event or KnownSkipped
-> replay cache／replayer
```

adapter／normalizer 負責：

- 驗證 `market`、partition symbol、`type`、`format`、必要欄位與 payload shape。
- 使用 exact decimal／integer 解析價格、數量與 counter；不得先轉成 `f32`／`f64`。
- 只把本文件列為 timeline 的 format 轉成 domain event。
- 保存 raw source、format、原始 numeric lexeme、`received_at` 及 non-timeline 記錄的
  skip reason，供 source lineage 與 verify 使用。
- 將 `match_time` 交給 domain `MatchTime`；它是唯一 replay clock。

adapter／normalizer 不負責：

- 以 `received_at` 排序、推算 latency 或代替 `match_time`。
- 以檔案列號、page index、cursor 或 ingestion order 補造 `source_sequence`。
- 由 book 差分推論委託、取消、成交方向、queue position 或 hidden liquidity。
- 由 `I021`／`I023`／`I030`／`I070`／`I072` 製造統計 event；`I022` 明確映射為
  opening indicative event。
- 以 TAIFEX wire type 讓 strategy、replayer、MarketState 或 simulation 直接依賴。

TAIFEX 的 `trading_date` 不是 wire field；它由已 materialize 的 source partition 與
session/calendar policy 提供。cross-midnight ownership 由 M3 session step 驗證，不
從單筆 tick 的 calendar date 猜測。

## 3. Wire envelope

每一個 extracted JSONL item 都必須是 object，且驗證下列共通欄位：

| Field | 實測 wire type | Rule | Domain use |
| --- | --- | --- | --- |
| `type` | string | 必須與 `format` pair 完全相符 | 選擇 format mapping |
| `market` | string | 必須為 `taifex_fut` | 映射 domain `MarketId::Taifex` |
| `format` | string | 必須是第 4 節列出的 known format | `SourceFormatId`；未知 strict failure |
| `symbol` | string | 必須等於 frozen partition symbol，保留原始 bytes | `InstrumentId` |
| `match_time` | string | 含 offset 的有效 ISO-8601；不可缺少 | 唯一 replay time |
| `received_at` | string | 含 offset 的有效 ISO-8601；不可缺少 | acquisition/source diagnostics only |

`match_time` 與 `received_at` 是兩個不同 clock。兩者可以有任意 sub-second
差異；不要求 `received_at >= match_time`。格式只保留 source 能證實的精度，並以
domain `MatchTime` 的 checked parser 驗證。

Teralion archive item 沒有可用的 TAIFEX `INFORMATION-SEQ`。`first_packet` 是
I020/I022 message 內的 packet/display 語意，不是全域或商品序號。因此：

- `source_sequence` 在 M3 reference event 一律為 `None`。
- 不得使用 page、cursor、file line、`received_at` 或 `first_packet` 補序號。
- 相同 `match_time` 的 deterministic tie-break 依 ADR-0001，以 source format、event
  kind 與 canonical fingerprint 完成；不宣稱是交易所真實封包順序。

## 4. Observed format registry

下表的 counts 是三份 committed fixture 合計 `74,214` 筆；它們是 mapping evidence，
不是未來所有日期的預期筆數。

| `type` / `format` | 合計筆數 | 官方語意 | M3 disposition |
| --- | ---: | --- | --- |
| `trade` / `I020` | 10,229 | matched prices and quantities | **timeline：`TradeBatch`** |
| `trade` / `I021` | 260 | intra-day high/low | `KnownSkipped(IntradayHighLow)` |
| `trade` / `I022` | 299 | calculated opening price and volume | **timeline：`IndicativeOpeningAuction`** |
| `trade` / `I023` | 15 | opening price and quantity | `KnownSkipped(OpeningReference)` |
| `stats` / `I030` | 4,914 | sum of order data | `KnownSkipped(OrderStatistics)` |
| `close` / `I070` | 8 | closing market data | `KnownSkipped(ClosingStatistics)` |
| `close` / `I072` | 15 | closing data with settlement/open interest | `KnownSkipped(ClosingStatistics)` |
| `book` / `I080` | 58,175 | order book information | **timeline：`BookSnapshot`** |
| `book` / `I082` | 299 | reference best five after calculated opening | **timeline：`BookSnapshot` (WarmUp reference)** |

Known skipped records remain in the immutable raw source and in normalization counts;
they are not an error and must never be silently treated as an empty page. An observed
known `type` with the wrong `format` is a schema error, not a known skip. A format not in
this registry is an unknown timeline format: Strict mode stops, and ExplicitDegraded may
skip the isolated record with a warning.

Observed composition by segment is recorded in the fixture metadata and source evidence;
the normalizer must not hard-code the aggregate counts above as a completeness rule.

## 5. I020：實際成交 `TradeBatch`

### 5.1 Wire shape

```json
{
  "aggregate": {
    "match_buy_cnt": 71,
    "match_sell_cnt": 110,
    "match_total_qty": 196,
    "status_code": 0
  },
  "first_packet": true,
  "format": "I020",
  "market": "taifex_fut",
  "match_time": "2026-07-20T08:45:00.068000+08:00",
  "received_at": "2026-07-20T08:45:00.074878+08:00",
  "symbol": "TXFH6",
  "trades": [{"price": 43500.0, "quantity": 196}],
  "type": "trade"
}
```

每筆 accepted I020 必須驗證：

| Field | Rule |
| --- | --- |
| `trades` | array，至少一筆；保留 array order；每筆 price 為 finite positive exact decimal，quantity 為 positive integer |
| `aggregate.match_total_qty` | non-negative integer；表示 source aggregate total，不宣稱等於本 item 的 batch sum |
| `aggregate.match_buy_cnt`／`match_sell_cnt` | non-negative integer；保存為 source counter，不當成成交筆數或 aggressor |
| `aggregate.status_code` | integer；官方 `0`、`1..60`、`98`、`99` 是 known values，其他值保留 raw 並 warning |
| `first_packet` | boolean；`true` 是本 fixture 唯一 observed shape |

fixture 中 `trades` 長度為 `1..16`，代表一個 source record 可以包含多筆成交。
`match_total_qty` 只有極少數與 `trades[].quantity` 總和相等；因此 normalizer 不得
以 batch sum 覆寫 aggregate，也不得由 aggregate 反推缺少的 trade。

M3 domain mapping：

```text
DomainEvent {
    instrument       = (TAIFEX, frozen symbol)
    trading_date     = planner-provided exchange date
    source_format    = I020
    match_time       = envelope.match_time
    source_sequence  = None
    payload          = TradeBatch(
        trades             = trades[].map(TradePrint::Regular),
        trade_order        = SourceOrdered,
        cumulative_volume  = Set(Volume(match_total_qty, Contract)),
        annotations        = TAIFEX source annotations, when schema support is present
    )
}
```

目前 `market-types` 的 common event model 還沒有 TAIFEX-specific annotation variant。
因此 `status_code` 與 buy/sell counters 必須至少由 source lineage／normalizer report
無損保存；若 strategy 或 simulation 要讀取它們，必須先新增 versioned TAIFEX
annotation schema，不能在 parser 中把它們丟棄或改名成 TWSE flags。I020 的
`match_total_qty` 可作為 `Contract` volume observation，但不作為 batch sum 的替代品。

### 5.2 Batch and packet policy

官方 I020 可以以多 packet 傳送同一 matched-time group；M3 Teralion fixture 已將每個
item 的 `trades` 保存為 observed batch，且全部 `first_packet=true`。在 source adapter
尚未取得穩定 continuation identity（packet sequence／group id）前：

- `first_packet=true` 的 item 依上表處理為一個 atomic `TradeBatch`。
- `first_packet=false`、缺少 `first_packet` 或出現無法配對的 continuation item，Strict
  mode 以 `unsupported_i020_continuation` 拒絕；不得把 continuation 當成另一個獨立
  batch 或依 input order 猜測合併。
- 不使用 `I023` 或相鄰 I080 來補 I020 的成交或累計量。

## 6. I080／I082：完整五檔 `BookSnapshot`

### 6.1 Common book shape

兩種 book format 的 `bids`、`asks` 都是 level array；M3 fixture 中每側均為五筆。
normalizer 應支援 `0..5` 個 level 以保留官方空槽語意，但必須拒絕超過五檔、空槽
後又出現 populated level、或 price/quantity 只有一側為 zero 的 pair。

每個 populated level：

```json
{"price": 43400.0, "quantity": 1}
```

規則：

- price 直接從 JSON numeric lexeme 轉 exact `Price`；不得經 binary float。
- quantity 是正整數，domain unit 固定為 `Contract`。
- bid array 由 best 到較差，價格嚴格遞減；ask array 由 best 到較差，價格嚴格遞增。
- 少於五筆只表達 trailing empty slots；不得沿用前一個 snapshot 的 level。
- 每一 accepted book 是完整替換；不推論 add/cancel、queue 或 hidden quantity。

### 6.2 I080 regular order book

I080 的 wire keys 固定為：`bids`、`asks`、`derived`、共通 envelope fields。
`derived` 的實測 shape 為：

```json
{
  "buy_price": 43364.0,
  "buy_quantity": 1,
  "sell_price": 0.0,
  "sell_quantity": 0
}
```

`derived` 是 TAIFEX source 提供的衍生 order observation，不是第六檔，也不是由本
平台重建的 queue。每一側的 `price=0` 與 `quantity=0` 表示該 derived side absent；
price／quantity 一正一零是 invalid shape。M3 event payload 只把 `bids`／`asks` 映射
為 `BookSnapshot`；`derived` 原 bytes 必須留在 source/diagnostic lineage，不能塞進
五檔 slots 或宣稱為一般 level。未來若 domain 要暴露它，需新增 versioned TAIFEX
annotation/schema。

### 6.3 I082 pre-open reference book

I082 的 wire keys 為 `bids`、`asks` 與共通 envelope fields，沒有 `derived`。官方語意
是集合競價試算後的 reference best five；M3 sample 只出現在各 segment 的 WarmUp
margin（regular 約 08:40 起，after-hours 約 14:55／17:20 起）。

I082 仍可映射為完整 `BookSnapshot`，但：

- source format 必須保留為 `I082`，不能改名為 `I080`。
- session policy 將其視為 WarmUp reference；它不能單獨使 order eligible for fill。
- 若 I082 出現在不相容的 session phase，Strict mode 拒絕，不能把它當成一般盤中 book。

### 6.4 Book event mapping

```text
DomainEvent {
    instrument       = (TAIFEX, frozen symbol)
    trading_date     = planner-provided exchange date
    source_format    = I080 or I082
    match_time       = envelope.match_time
    source_sequence  = None
    payload          = BookSnapshot(
        complete_five_level_book = validated bids/asks,
        annotations              = TAIFEX source annotations, when schema support is present
    )
}
```

I080／I082 同一 source item 只有 book；不得從相鄰 I020 配對成交、補 cumulative
volume 或合成 QuoteSnapshot。若未來某一 wire shape 同時包含不可分割的 book 與實際
成交，必須先更新 event schema，不能在 normalizer 內任意拆成兩個時間點。

## 7. I022：開盤試算 `IndicativeOpeningAuction`

`I022` 是 TAIFEX 集合競價的 calculated opening price／volume observation，不是已成交
的 `TradeBatch`。每一個 accepted `I022` source item 產生一個 atomic
`IndicativeOpeningAuction` event：

- `trades` 必須恰有一筆；price 與 quantity 必須同時為 `0/0` 或同時為正值。
- `0/0` 映射為 `price=NoObservation`、`quantity=NoObservation`。
- 正值映射為 `price=Set(Price)`、`quantity=Set(Quantity<Contract>)`。
- TAIFEX fixture 沒有 `book` 或 cumulative volume 可用，因此兩者保留為
  `NoObservation`；I022 永遠不會更新實際成交或成交量狀態，也不能產生 fill evidence。
- I022 不建立 `IndicativeClosingAuction`。官方 closing/stat records `I070`／`I072`
  在本 mapping 仍是 `KnownSkipped(ClosingStatistics)`。

## 8. Known non-timeline formats

這些 formats 的 raw record 必須可 verify、計數與 inspect，但本 mapping 不產生 domain event。
`received_at`、`show_time`、close/stat time 或 open reference 都不能被用作 replay
clock。

| Format | Observed fields | Validation／preservation | Skip reason |
| --- | --- | --- | --- |
| `I021` | `day_high`、`day_low`、`show_time` | prices exact；`show_time` 只保存原始 `HH:MM:SS...` text，不補 date/offset | `IntradayHighLow` |
| `I023` | `open_price`、`open_quantity` | exact price／integer quantity；同一開盤可重複傳送，不得與 I020 合併或重複計成交 | `OpeningReference` |
| `I030` | `buy_order`、`buy_quantity`、`sell_order`、`sell_quantity` | non-negative integer；數量單位為 contracts；不得當成 book 或 trade volume | `OrderStatistics` |
| `I070` | `prices[]`、`stats[]`、nullable `settlement`／`open_interest` | arrays、null 與 numeric lexeme 原樣保存；不產生 close event | `ClosingStatistics` |
| `I072` | I070 fields plus `block_trade_qnty` | non-negative contract quantity；不產生 close event | `ClosingStatistics` |

I023 是 opening reference，且 fixture 中同一 open match time 出現三次；它不會與 I020
合併，也不會重複計成交。

I070／I072 的 settlement、open interest、close price 與 order counters 暫不進
MarketState。若未來需要 settlement/accounting input，必須先定義 timing、trading
period semantics、event schema 與 cache compatibility。

## 9. Status、counter、quantity 與 precision rules

### 9.1 Status and raw source values

I020／I022 `aggregate.status_code` 依官方 message definition：`0` 為 normal，
`1..60` 為 abnormal duration，`98` 表示 abnormal removed，`99` 表示超過 60 分鐘。
值必須原樣保留；目前 fixture 全部為 `0`。未來不在此集合的值產生 warning，不能
猜測成新 market phase，也不能建立 standalone status event。

I080 `derived` 是否存在及其四個欄位也是 source fact。未知額外 fields 一律留在 raw
source，並在 normalizer report 計數；generic adapter 不刪欄位、不重命名。

### 9.2 Quantity units

M3 三個 futures instruments 的交易價量、五檔量、I020 aggregate quantity、I022／I023
opening quantity、I030 order quantity、I070／I072 statistics quantity 都以
`QuantityUnit::Contract` 解讀。這不等於 futures multiplier；multiplier 是另外由
explicit economics provenance 綁定的 accounting input。

`match_buy_cnt`／`match_sell_cnt` 是 order counts，不是 contracts；不能與 quantity
相加或當成 trade count。I020 `trades[].quantity` 是 batch 內的 contracts。

### 9.3 Numeric handling

- JSON number 的原始 lexical representation 必須可無損轉成 domain Decimal；禁止先經
  binary floating-point。
- accepted timeline price 必須 finite、positive、checked；zero 只在明確 documented
  sentinel（I022 `0/0`、I080 derived absent side）出現。
- negative price、NaN、infinity、fractional quantity、overflow、quantity pair
  mismatch 或無法解析的 timestamp 都是 invalid payload。
- `null` 只在 I070/I072 的 observed nullable fields 依 wire shape 保留；不得把 null
  改成 zero 或 unknown event value。

## 10. Atomicity and ordering

一個 accepted I020、I022、I080 或 I082 source item 產生一個、且只產生一個 atomic domain
event；一個 I021/I023/I030/I070/I072 item 產生一個 `KnownSkipped` source
diagnostic，不產生 event。

- I020 batch 內的 trades 不拆成多個 event。
- I080/I082 的 bid、ask、derived source facts 不拆成多個 event。
- 不依 input order 將 I020 與同 `match_time` 的 I080/I082 配對。
- accepted event 先更新該 instrument 的 state，再呼叫 strategy；不同 instrument 的
  state 不互相補值。
- 全域排序第一鍵是 normalized `match_time`，後續使用 ADR-0001 ordering version 3。
- `received_at`、source page、cursor、JSON line number 與 discovery order 不參與
  replay ordering。

## 11. Strict／ExplicitDegraded errors

Strict mode 至少必須拒絕：

- invalid JSON、非 object、缺少共通 envelope field。
- `market`、symbol、type/format pair 與 frozen partition identity 不相符。
- missing/invalid `match_time` 或 timestamp precision overflow。
- unknown format、known format 的 wrong payload shape 或 unsupported I020 continuation。
- book 超過五檔、level 順序錯誤、空槽不連續、price/quantity zero mismatch。
- I020 empty batch、non-positive populated trade、counter negative/overflow。
- I022 只出現一個 zero、I080 derived side 只出現一個 zero。
- source partition 的 trading-date/session ownership 無法由 planner 證明。

ExplicitDegraded 可以逐筆略過已隔離的 known unsupported/invalid scope，但必須留下
record number、instrument、format、match_time、reason 與 completion quality；不得把
invalid record 改成 zero、沿用前一個 book 或以 `received_at` 補時間。

## 12. Normalizer test catalog

下列測試以 committed fixtures 為 positive evidence，另以 synthetic domain fixture
覆蓋 source 未出現的 negative shape；測試不得修改已提交的 raw bytes。

| ID | Case | Expected result |
| --- | --- | --- |
| `TAIFEX-W01` | common envelope identity/time validation | exact market/symbol accepted；mismatch、missing time rejected |
| `TAIFEX-W02` | observed format registry | 三份 fixture 的九種 type/format 全部分類，count 與 metadata 對得上 |
| `TAIFEX-W03` | I020 single and multi-trade | 一筆 atomic `TradeBatch`；保留 source order、Contract unit、aggregate volume |
| `TAIFEX-W04` | I020 aggregate semantics | batch sum 與 `match_total_qty` 不相等的 fixture accepted；不得重算或覆寫 aggregate |
| `TAIFEX-W05` | I020 continuation | `first_packet=false`／missing 在目前 source boundary 以 stable unsupported error 拒絕 |
| `TAIFEX-W06` | I022 calculated opening | `0/0` 產生 no-observation opening event；non-zero 產生 indicative event；兩者都不是 regular trade |
| `TAIFEX-W07` | I023 repeated opening reference | 同 match time 的三次 raw record 都可保存，但不重複產生 trade |
| `TAIFEX-W08` | I080 complete book | 五檔 bid/ask、strict price order、derived zero/non-zero side 全部可驗證 |
| `TAIFEX-W09` | I082 WarmUp reference book | 五檔映射為 `BookSnapshot`，source format 保持 I082，fill eligibility 由 session policy 限制 |
| `TAIFEX-W10` | malformed book | >5 levels、order reversal、hole、zero mismatch、non-positive level rejected |
| `TAIFEX-W11` | non-timeline records | I021/I023/I030/I070/I072 只產生 KnownSkipped 與 counts；I022 不在 skip set |
| `TAIFEX-W12` | close/stat payload | nullable fields、array shape、raw values 不進 timeline 且不被當成空頁 |
| `TAIFEX-W13` | status/counter preservation | known status accepted；unknown status warning；raw counters 不遺失 |
| `TAIFEX-W14` | source sequence boundary | `source_sequence=None`；不因 line/page/received_at 改變 event order |
| `TAIFEX-W15` | cross-midnight partition | session/calendar layer 驗證 `2026-07-17`/`2026-07-18` ticks 仍屬 `2026-07-20` trading date |
| `TAIFEX-W16` | deterministic normalization | shuffled input 產生相同 accepted events、skip counts、warnings 與 canonical bytes |

## 13. Phase boundary

本文件完成 M3 Phase 2 的 interface mapping 與 normalizer test catalog；目前已交付
`taifex-normalizer` 的 I020／I022／I080／I082 mapping 與 TAIFEX MarketState profile。
cache builder、multi-stream replay、session/calendar 與 TAIFEX-specific annotations 仍
屬後續 M3 work，不得把 I022 重新降級成 skip 或把 I070／I072 猜成 closing auction。

## 14. M5 index option profile

M5 增加一個由實際來源固定的 TAIFEX index option profile：`TXO24000U6`、trading
date `2026-07-28`，source market 明確為 `taifex_opt`，domain market 仍為
`MarketId::Taifex`。fixture 位於
[`fixtures/teralion/taifex/TXO24000U6/2026-07-28`](../../fixtures/teralion/taifex/TXO24000U6/2026-07-28)，
適用 mapping 為 `TeralionTaifexOptions`，`mapping_version = 1`。query identity 會
把 `ArchiveMarket::TaifexOptions` 編入 canonical bytes，不能把 option response
當成 `taifex_fut`。

### 14.1 Contract 與 session

| 欄位 | M5 固定值 | Provenance |
| --- | --- | --- |
| underlying | `TAIEX` | TAIFEX TXO product |
| option side | `Put` | symbol `U` convention 與 daily identity |
| strike | `24000` | symbol／daily contract reference |
| expiry | `2026-09-16` | daily expiry month；third Wednesday calendar |
| currency | `TWD` | TXO contract specification |
| multiplier | `50` | TWD per index point |
| quantity unit | `Contract` | TAIFEX wire quantity semantics |

官方 session 是 regular `08:45–13:45`、after-hours `15:00–次日 05:00`。下載 query
保留五分鐘 margin，實際 query window 為
`[2026-07-27T14:55, 2026-07-28T13:50)`；normalizer 以兩個不連續的
`match_time` replay windows 判定跨日 ownership：after-hours
`[14:55, 05:05)` 與 regular `[08:40, 13:50)`。因此 05:00–08:40 的 raw records
會保留在 source，但不會被誤放進 timeline。

### 14.2 Observed format disposition

| Format | Fixture 筆數 | M5 行為 |
| --- | ---: | --- |
| `I020` | 2 | atomic `TradeBatch`，quantity 為 `Contract` |
| `I022` | 177 | 在 replay window 內的 60 筆為 `IndicativeOpeningAuction`；`0/0` 保持 `NoObservation` |
| `I080` | 85 | atomic `BookSnapshot` |
| `I082` | 177 | replay window 內的 60 筆為 `BookSnapshot`，保留 source format |
| `I021`／`I023`／`I030`／`I070`／`I072` | 99 | `KnownSkipped`，保留 raw 與 reason，不產生 domain event |

整個 fixture 為 540 筆，normalizer 在 source replay windows 產生 207 個 events、95
個 known-skipped、238 個 outside-window records。book 是 snapshot replacement；一側
為 empty 是合法空槽，不以前一筆補值，也不從 I020 與 I080/I082 猜 aggressor 或
queue。`close`／`stats` 不會被改寫成 closing event 或成交量。

### 14.3 Accounting 與 strict boundary

Option 使用 `AccountingModel::OptionsV1`：買進／賣出先按
`price × economic_quantity × multiplier` 移動 premium cash，再以 average cost
計算 realized P&L；同一 M5 config 中的 `TXFH6` 保持 `FuturesV1`，兩者在 positions
與 reconciliation 中分開。fee、tax、slippage 仍由 config 明示，fixture acceptance
使用 zero charge 以隔離 multiplier／unit 行為。

`taifex_opt` query 收到 `taifex_fut` payload、錯誤 symbol、wrong type/format pair、
unknown format、I020 continuation、超過五檔或 invalid numeric shape 時 strict reject。
I021/I023/I030/I070/I072 只有在 identity/time/shape 通過後才進 known-skipped 分類；
不會把 unknown 或 malformed record 當成空頁。positive／negative fixture test 位於
`crates/normalizer/taifex/tests/m5_option_fixture.rs`。
