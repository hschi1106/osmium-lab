# TWSE Teralion Interface

## 1. 文件目的

本文件定義 Teralion TWSE quote wire document 到 `osmium-lab` domain event 的
mapping、驗證與明確排除範圍。它是 normalizer 的 source contract，不是 Rust
type definition；具體 exact price／quantity type 與 canonical encoding 由
[market types 設計](../design/market-types.md)固定。

本文件依據：

- [Teralion Feed API](https://docs.teraliontech.com/feed/)對 quote envelope、
  two clocks、price 與 quantity 的說明。
- [Teralion Feed Archive API](https://docs.teraliontech.com/feed-archive/)。
- [TWSE 集中市場即時交易資訊傳輸規格書 B.12.13](https://dsp.twse.com.tw/public/static/downloads/computerPlanningOperationsDepartment/TWSE%E9%9B%86%E4%B8%AD%E5%B8%82%E5%A0%B4%E5%8D%B3%E6%99%82%E4%BA%A4%E6%98%93%E8%B3%87%E8%A8%8A%E5%82%B3%E8%BC%B8%E8%A6%8F%E6%A0%BC%E6%9B%B8%28B.12.13%29%28202612%29_20260515151841.pdf)。
- 2026-07-27 TWSE `2330` 08:55–13:35 的 77,213 筆實際 response。

適用 mapping：`TeralionTwseQuote`，`mapping_version = 4`。

## 2. 支援範圍

第一版只處理 TWSE 普通交易的整股 quote。實際下載中同時出現的 format 依下表
分類：

| `format` | 實測筆數 | 第一版行為 |
| --- | ---: | --- |
| `STOCK_SNAPSHOT` | 3,597 | M1 支援；一般 quote 為 `QuoteSnapshot`，試算揭示為 auction event |
| `STOCK_REALTIME` | 70,199 | M1 支援完整 book 與已驗證的 intermediate/final `1+1` group；見第 7 節 |
| `INTRADAY_ODDLOT_REALTIME` | 3,417 | 已知但不支援；保存 raw、計數略過、不產生 event |

第一版不支援：

- 盤中零股
- 盤後零股
- 盤後定價
- 鉅額交易
- 逐筆委託或 queue reconstruction
- 由 raw flags 猜測的獨立狀態事件

`kinds=quote` 無法排除 `INTRADAY_ODDLOT_REALTIME`；所以 source sync 可以保存它，
但 replay cache builder 必須依 format 排除，不能因它落在 regular session window
內就視為支援。

M1 committed fixture 必須保存全部 regular `STOCK_SNAPSHOT` 與
`STOCK_REALTIME` records。兩種 format 共同驗證完整 quote；試算揭示映射為
`IndicativeOpeningAuction`／`IndicativeClosingAuction`，一般 realtime
intermediate／final group 則維持 `TradeBatch -> QuoteSnapshot`。

## 3. TWSE session

TWSE 普通交易使用 `regular` session：

| 項目 | Asia/Taipei time |
| --- | --- |
| Official session | 09:00–13:30 |
| Download window by `received_at` | `[08:55, 13:35)` |
| Replay window by `match_time` | `[08:55, 13:35)` |
| `WarmUp` | `[08:55, 09:00)` |
| `Active` | `[09:00, 13:30]` |
| `CoolDown` phase | `(13:30, 13:35)` |

完整規則由 [ADR-0003](../architecture/decisions/0003-session-windows-and-strategy-activation.md)
定義。落在 margin 內不代表該 format 自動受支援；`CoolDown` 也不是來源 event，
平台不在 13:30 合成 observation。

2026-07-27 樣本的第一筆 source tick：

```text
received_at = 08:55:01.069708
match_time  = 08:54:56.982904
```

它符合 download window，但不符合 replay window。source 必須保留它並記錄
`outside_replay_window`，replayer 不得把 `received_at` 當成 `match_time` 將它
放進 timeline。

## 4. Wire envelope

每筆 TWSE quote 都先驗證：

| Field | 實測 wire type | Rule |
| --- | --- | --- |
| `type` | string | 必須為 `quote` |
| `market` | string | 必須為 `twse` |
| `format` | string | 必須由本文件明確分類 |
| `symbol` | string | 非空，且符合 source partition identity |
| `match_time` | string | 必須是含 offset 的有效 ISO-8601；唯一 replay time |
| `received_at` | string | 必須是含 offset 的有效 ISO-8601；只用於 acquisition |

adapter 不對 symbol 做數值轉換。`2330` 是 string，不得因看似數字而去除前導零、
改變大小寫或轉成 filesystem-derived identity。

`received_at` 與 `match_time` 是兩個獨立 clock。樣本中可觀察到
`received_at < match_time` 的 sub-second case；不得以兩者相減推論可靠 latency，
也不得要求 capture clock 一定晚於 exchange clock。

## 5. Quote body

### 5.1 共通欄位

`STOCK_SNAPSHOT` 與 `STOCK_REALTIME` 的共通實測欄位：

| Field | Wire type | Domain mapping |
| --- | --- | --- |
| `bids` | `{price, quantity}[]` | 完整 best-five bid observation |
| `asks` | `{price, quantity}[]` | 完整 best-five ask observation |
| `deal` | `{price, quantity}`／`null` | source deal observation；`null` 是 `NoObservation` |
| `cum_volume` | integer | session cumulative volume observation |
| `limit_flags` | integer | 無損保存 raw value |
| `status_flags` | integer | 無損保存 raw value |
| `intermediate_print` | boolean | 區分 final quote 與 intermediate print |

`STOCK_SNAPSHOT` 額外實測：

| Field | Wire type | 第一版行為 |
| --- | --- | --- |
| `open_price` | number | 保存在 raw source；不進 `QuoteSnapshot` |
| `high_price` | number | 保存在 raw source；不進 `QuoteSnapshot` |
| `low_price` | number | 保存在 raw source；不進 `QuoteSnapshot` |

這三個欄位在 pre-open 樣本可為 `0.0`。第一版不將 zero 猜成 absent、真實成交價或
策略可用的 session OHLC；若未來需要，必須以新 schema 與 fixture 定義語意。

### 5.2 Level

每個 level：

```json
{
  "price": 2320.0,
  "quantity": 99
}
```

normalizer 必須：

- 保留 array order；bids 由 best 到較差、asks 由 best 到較差。
- 每側最多五個 levels。
- price 必須是 finite、positive 且可轉成 exact domain price。
- quantity 必須是 positive integer。
- 不從相鄰 snapshot 差分推論 add、cancel、fill 或 queue position。
- 少於五個 levels 時以完整 snapshot 的 empty slots 表達，不沿用舊 level。

Teralion JSON number 不得直接以 binary floating-point 作 canonical event value。
decoder 必須保留足以無損轉成 domain decimal／tick representation 的 numeric
lexeme；exact Rust representation 留給 `market-types`。

TWSE B.12.13 與本地 fixture 已固定 quantity unit：

- regular `STOCK_SNAPSHOT`／`STOCK_REALTIME` 的成交量、五檔量與累計成交量，
  每一 source quantity unit 是一個 TWSE `TradingUnit`。
- `INTRADAY_ODDLOT_REALTIME` 的成交量與五檔量，每一 source quantity unit 是
  一股，因此 equity odd-lot 未來映射為 `Share`。

`TradingUnit` 不直接改寫成 `Share × 1,000`。B.12.13 的個股基本資料另有
「交易單位」欄位，記錄每交易單位所代表的股數／權證單位數／受益單位數，且可
不是 1,000。該 conversion factor 屬於 daily instrument metadata，不屬於 quote
event，也不能由 symbol type 猜測。

2026-07-27 的 2330 evidence 與規格一致：regular `STOCK_SNAPSHOT` 的最大
`cum_volume` 為 24,003 trading units；盤中零股的最大 `cum_volume` 為
1,511,292 shares。兩種 format 的 cumulative sequence 必須保持隔離。

### 5.3 Deal

`deal` 非 `null` 時：

```json
{
  "price": 2320.0,
  "quantity": 2077
}
```

price 使用 level 相同的 exact／positive 驗證；quantity 使用
`QuantityUnit::TradingUnit`。`deal=null` 表示本 source tick 沒有新的 deal
observation，不得：

- 將 recent trade 清空。
- 以 `0` price／quantity 代替。
- 沿用上一筆 deal 後宣稱本 tick 也觀察到成交。

WarmUp 期間的 `deal` 可能代表 pre-open 狀態下的來源 observation。只因欄位存在，
不得自動宣稱它是可供 fill model 使用的正式成交；fill eligibility 仍需 session
phase、format 與已確認的 status semantics。

### 5.4 Cumulative volume

`cum_volume` 是 `QuantityUnit::TradingUnit` 的 session cumulative
observation，`0` 是合法值而不是 absent。normalizer 不從 deal quantities重算或
修補 cumulative volume。

若同一 session 的 accepted event 造成 cumulative volume 倒退，必須依 mapping
shape 判斷：

- 已確認的 session reset：以明確語意處理。
- 不同且不支援的 format，例如盤中零股：不得混入整股 state。
- 無法解釋：依 `REPLAY-06` 停止，不以 `max(previous, current)` 靜默修正。

## 6. `QuoteSnapshot` mapping

### 6.1 Envelope

| Domain field | Source |
| --- | --- |
| `market` | constant validated `TWSE` from `market=twse` |
| `symbol` | `symbol` |
| `source_format` | exact `format` |
| `match_time` | parsed `match_time` |
| `event_kind` | non-trial `QuoteSnapshot`；trial 為對應 auction event |
| `source_sequence` | absent；TWSE quote sample 沒有可用 counter |

`received_at` 不進 replay clock，也不是 `OrderingRule` 的 source sequence。
source page、cursor、file line 或 ingestion ordinal 都不得補成 sequence。

### 6.2 Payload

對符合完整 book shape 的 `STOCK_SNAPSHOT` 或 `STOCK_REALTIME`：

| Domain observation | Mapping |
| --- | --- |
| complete bid slots | `bids` array，依序轉成最多五 slots |
| complete ask slots | `asks` array，依序轉成最多五 slots |
| trade observation | non-trial `deal` present → `Set`；`null` → `NoObservation` |
| cumulative volume | non-trial `Set(cum_volume)` |
| book／trade quantity unit | constant `TradingUnit` for regular `STOCK_*` formats |
| cumulative volume unit | constant `TradingUnit` for regular `STOCK_*` formats |
| limit annotations | 保存 raw `limit_flags`，並依第 8.2 節解碼四組 2-bit value |
| status annotations | 保存 raw `status_flags`，並依第 8.1 節解碼獨立 bits |
| standalone status event | 不產生；status 與 observation 留在同一 atomic event |

book、deal、cumulative volume 與 raw flags 必須組成單一 atomic event。event
accepted 後，一次完整取代 book、一次更新其他明確 observation，且 state version
只增加一次。

`STOCK_SNAPSHOT` 的 `open_price`、`high_price`、`low_price` 不在
`QuoteSnapshot` payload，因此不影響 event fingerprint；它們仍由 source
checksum 保護，未來加入 domain event 時必須更新 event schema／mapping version。

### 6.3 Complete book shape

第一版的 complete book shape 必須同時符合：

- `bids`、`asks` fields 都存在且為 arrays。
- 每側長度為 0 至 5。
- 所有存在的 levels 通過 price／quantity 驗證。
- `intermediate_print=false`。

array 少於五筆代表該側剩餘 slots 為 empty，不代表從前一 event merge。若來源
field 缺少、type 不符或大於五檔，視為 invalid payload。

### 6.4 Indicative auction events

`status_flags` 的 Bit 7 為 `1` 時，source record 是試算揭示，不得映射成
`QuoteSnapshot`、`TradeBatch` 或 actual cumulative volume。normalizer 依明確
opening／closing marker、delayed bit 或 session window 分類：

| Source condition | Domain event |
| --- | --- |
| 08:30–09:00 trial／opening marker／delayed open | `IndicativeOpeningAuction` |
| 13:25–13:30 trial／closing marker／delayed close | `IndicativeClosingAuction` |

auction payload 保留 indicative price／quantity、可用的 complete book、source
`cum_volume` observation 與 raw annotations；這些欄位不會更新 actual trade 或
fill evidence。無法唯一分類的 trial record 在 strict mode reject，不可猜測。

`STOCK_REALTIME` trial intermediate／final pair 仍須恰好一筆一筆；intermediate
auction event 沒有 book，final auction event 才可帶 complete book。兩者依 source
phase rank deterministic ordering。

## 7. `intermediate_print` edge case

2026-07-27 樣本有 3 筆：

```text
format             = STOCK_REALTIME
intermediate_print = true
deal               = present
bids               = []
asks               = []
```

每筆都在相同 `match_time` 附近另有 `intermediate_print=false`、完整五檔及下一個
deal 的 record。空 arrays 因此不能安全解讀為「當時完整 order book 為空」，否則
會短暫清除既有 MarketState；也不能沿用舊 book 後仍稱該 event 是完整
`QuoteSnapshot`。

TWSE B.12.13 的「揭示項目註記」進一步確認此 shape：Bit 0 為 `1` 時表示逐筆交易
產生的非最後一個成交價量，只揭示成交、不揭示最佳五檔；最後一個成交價量才會
同時揭示最佳五檔。Teralion 的 `intermediate_print=true` 與這項規則及實測
empty book 完全一致。因此 empty arrays 在此 case 是 `NoBookObservation`，不是
complete empty book。

三個 exact-`match_time` groups 的實測值：

| `match_time` | Intermediate deal／cum | Final deal／cum |
| --- | --- | --- |
| `09:28:49.274622` | `2340 × 1`／5,616 | `2345 × 7`／5,623 |
| `09:30:55.252155` | `2345 × 7`／6,055 | `2350 × 6`／6,061 |
| `10:29:59.907157` | `2360 × 2`／9,339 | `2365 × 1`／9,340 |

`TeralionTwseQuote` mapping version 3 保留 version 2 定義的 grouping，依
[ADR-0005](../architecture/decisions/0005-twse-intermediate-final-ordering.md)處理：

1. 以 market、trading date、symbol、format、`match_time` 建立 group，不能依 API
   page 或 input order 配對。
2. 已驗證的 group 必須恰好包含一筆 intermediate 與一筆具有 complete book 的
   final record。
3. intermediate 映射為 single-element `TradeBatch`，不修改或合成 book。
4. final 映射為 `QuoteSnapshot`。
5. `OrderingRule` version 2 以 source phase rank 固定
   `TradeBatch(intermediate) -> QuoteSnapshot(final)`。
6. final cumulative volume 必須等於 intermediate cumulative volume 加 final deal
   quantity。

若 group 有 missing final、multiple intermediate、multiple final、volume mismatch
或 invalid final book，default mode 以 `unsupported_realtime_match_group` 拒絕整個
group。explicit degraded mode 只能略過整個 group，並記錄 group count、first／last
`match_time` 及 degraded result；不得只略過 intermediate 後接受 final。

M1 必須使用 committed `STOCK_REALTIME` fixture 驗證全部三個實測 `1+1` groups；
不能把 intermediate record 略過後只接受 final。

## 8. Flags 與 quote annotations

整股 `STOCK_SNAPSHOT`／`STOCK_REALTIME` 在 2026-07-27 實測：

| Field | Observed values |
| --- | --- |
| `status_flags` | `4`、`8`、`16`、`128` |
| `limit_flags` | `0` |

Teralion 將兩者描述為 quote-header flags；其整數值與 TWSE B.12.13 的
「狀態註記」及「漲跌停註記」byte layout 一致。第一版同時保存 raw byte 與 decoded
annotations，避免未來規格新增 bit 時遺失來源資訊。

### 8.1 `status_flags`：狀態註記

`status_flags` 是 bit mask，不是互斥 enum：

| Bit | Mask | Decimal | `0` | `1` |
| ---: | ---: | ---: | --- | --- |
| 7 | `0x80` | 128 | 一般揭示 | 試算揭示 |
| 6 | `0x40` | 64 | 否 | 試算後延後開盤 |
| 5 | `0x20` | 32 | 否 | 試算後延後收盤 |
| 4 | `0x10` | 16 | 集合競價 | 逐筆撮合 |
| 3 | `0x08` | 8 | 非開盤揭示 | 開盤揭示 |
| 2 | `0x04` | 4 | 非收盤揭示 | 收盤揭示 |
| 1–0 | `0x03` | 3 | 保留，應為 zero | 未知／未來規格 |

Bit 6／5 只有 Bit 7 為 `1` 的試算揭示才具有語意；Bit 7 為 `0` 時，即使來源帶值
也不得解讀為延後開／收盤。Bit 3／2 是該筆行情的 opening／closing marker，不是
會一直維持到下一事件的 session phase。

本次 observed values 可直接解碼：

| Value | Hex／binary | Meaning | 2330 evidence |
| ---: | --- | --- | --- |
| 128 | `0x80`／`1000_0000` | 試算揭示 | 08:54:56 起的開盤前試算，以及 13:25 後的收盤前試算 |
| 8 | `0x08`／`0000_1000` | 開盤揭示 | 09:00:07.360140，共 2 筆 regular records |
| 16 | `0x10`／`0001_0000` | 逐筆撮合 | 09:00:07.385101–13:24:59.647541 |
| 4 | `0x04`／`0000_0100` | 收盤揭示 | 13:30:00，共 9 筆 regular records |

這個時間分布與 TWSE bit definition 相符，是 Teralion integer 對應原始 status byte
的 fixture 證據。未來若看到組合值，必須逐 bit 解碼，例如 `144 = 128 + 16`
表示「試算揭示」與「逐筆撮合註記」同時 set；不得把 `144` 當成新 enum variant。

`status_flags` annotations 隨原 quote 或 auction event 原子更新。replayer 不因
clock 穿越 09:00／13:30 合成另一個 status event，也不能把 `16` 簡化成永久的
`MarketOpen`。Bit 7 trial record 必須使用第 6.4 節的 auction event。

### 8.2 `limit_flags`：漲跌停註記

`limit_flags` 是四組 2-bit fields，必須分組解碼：

| Bits | Mask | Annotation | `00` | `01` | `10` | `11` |
| --- | ---: | --- | --- | --- | --- | --- |
| 7–6 | `0xC0` | 成交漲跌停 | 一般成交 | 跌停成交 | 漲停成交 | 保留 |
| 5–4 | `0x30` | 最佳一檔買進漲跌停 | 一般買進 | 跌停買進 | 漲停買進 | 保留 |
| 3–2 | `0x0C` | 最佳一檔賣出漲跌停 | 一般賣出 | 跌停賣出 | 漲停賣出 | 保留 |
| 1–0 | `0x03` | 瞬間價格趨勢 | 一般揭示 | 暫緩撮合且瞬間趨跌（緩跌） | 暫緩撮合且瞬間趨漲（緩漲） | 保留 |

因此本次唯一 observed value：

```text
limit_flags = 0 = 0x00 = 0000_0000
```

表示一般成交、最佳一檔買進非漲跌停、最佳一檔賣出非漲跌停，且沒有暫緩撮合的
瞬間趨漲／趨跌註記。買賣漲跌停註記只描述最佳一檔，不得套用到第二至第五檔。

non-zero layout 已由 TWSE 規格定義，但目前 2330 fixture 沒有 non-zero
`limit_flags`；在它影響 fill eligibility 前，仍須加入合法的漲停、跌停與暫緩撮合
fixture。`11` reserved code 必須保存 raw value、產生 warning，不能猜測。

### 8.3 緩跌／緩漲

市場常用的「緩跌／緩漲」是 `limit_flags` 最低兩 bits 的簡稱：

```text
instant_trend = limit_flags & 0x03

0 -> Normal
1 -> VolatilityInterruptionDown（緩跌）
2 -> VolatilityInterruptionUp（緩漲）
3 -> Reserved
```

它表示盤中瞬間價格穩定措施已觸發，該商品暫緩撮合，並指出觸發時試算價格的
方向。它不是：

- 跌停／漲停；漲跌停分別在同一 byte 的其他 2-bit fields。
- 整個 TWSE market halt；狀態屬於該 symbol。
- `status_flags` Bit 6／5 的試算後延後開盤／收盤。
- strategy session phase；盤中仍屬 `Active`。

TWSE 的瞬間價格穩定措施通常暫緩該商品撮合 2 分鐘，期間繼續接受符合交易所規則
的委託，結束後以集合競價撮合，再恢復逐筆交易。對第一版 backtest：

- `QuoteSnapshot` 仍更新可觀察的試算行情與 flags。
- strategy callback 仍執行，且 `Active` phase 仍可送出 order intent。
- `instant_trend` 為 `1` 或 `2` 的 event 不得作為實際 fill evidence。
- pending order 在暫緩期間不成交；只有後續明確回到可撮合狀態的 event 才重新
  依 fill model 判定。
- 不能只用本機 clock 自行假設 2 分鐘後恢復；必須等待來源 flags／可辨識的實際
  撮合 event。

目前 2330 樣本的 `limit_flags` 全為 `0`，所以這套行為只有 specification
evidence、尚無 local fixture evidence。正式啟用 fill rule 前必須新增至少一組
緩跌與緩漲 fixture。

### 8.4 Domain 與 warning policy

- raw `status_flags`／`limit_flags` 永遠保留。
- 已定義 bits 產生 typed annotations，不再對 4／8／16／128／0 發 unknown
  warning。
- reserved bits／codes 或不符合 interface version 的新組合使用
  `Unknown(raw)`，並 deterministic aggregate warning。
- flags 本身不改寫 planner 的 `WarmUp`／`Active`／`CoolDown` phase。
- fill model 若要使用 opening、closing、trial 或 limit annotations，必須以
  versioned rule 明確宣告；不能只因某個 bit 存在就成交。
- TradingContext 與 order／event eligibility 依
  [ADR-0004](../architecture/decisions/0004-trading-context-and-eligibility.md)建立。

## 9. 已知不支援 format

### 9.1 `INTRADAY_ODDLOT_REALTIME`

此 format 是實際存在的 Teralion `quote`，不是 unknown format；但產品需求明確
排除盤中零股。處理方式：

```text
source sync: preserve
source manifest: count by format
cache build: known unsupported, skip
timeline: no event
MarketState: no update
run summary: skipped count
```

default mode 不需要因已知且完整隔離的 odd-lot record 失敗，但若使用者 strategy
universe 明確要求 odd-lot session／instrument，planner 必須以 unsupported scope
拒絕，不能回退到整股行情。

其 `cum_volume`、book quantity、deal 或 flags 不得混入 `STOCK_*` state。未來加入
盤中零股 equity 時，quantity unit 必須是 `Share`，並使用新的 session／format
mapping 與獨立驗收。ETF／其他非 equity odd-lot instrument 仍須由各自 fixture
與 official unit definition 固定，不得一律套用 `Share`。

### 9.2 Unknown format

不在本文件 registry 的 TWSE format：

- raw source 可以保存。
- normalizer default mode 必須拒絕。
- 不可依欄位「看起來相同」套用 `STOCK_REALTIME` mapping。
- degraded mode 只能在已隔離、明確記錄 skipped count 且 policy 允許時略過。

## 10. 2026-07-27 實測摘要

來源：

```text
market       = twse
symbol       = 2330
trading_date = 2026-07-27
received_at  = [08:55, 13:35)
kinds        = quote
```

| Evidence | Value |
| --- | --- |
| Pages／ticks | 16／77,213 |
| `STOCK_REALTIME` | 70,199 |
| `STOCK_SNAPSHOT` | 3,597 |
| `INTRADAY_ODDLOT_REALTIME` | 3,417 |
| Missing `match_time` | 0 |
| `received_at` monotonic by downloaded page chain | yes |
| First／last `received_at` | 08:55:01.069708／13:33:11.007003 |
| First／last `match_time` | 08:54:56.982904／13:30:00 |
| `STOCK_SNAPSHOT` book depths | bids 5、asks 5 |
| `STOCK_REALTIME` book depths | bids／asks 5；3 筆 intermediate records 為 0／0 |
| `intermediate_print=true` | 3 |
| Regular `status_flags` | 4、8、16、128 |
| Regular `limit_flags` | 0 |
| Regular `STOCK_SNAPSHOT` maximum `cum_volume` | 24,003 `TradingUnit` |
| Odd-lot maximum `cum_volume` | 1,511,292 `Share` |

樣本另顯示：

- `deal` 可以是 object 或 `null`。
- `cum_volume=0` 與 non-null `deal` 可以同時出現，不能以 zero 判定 deal 無效。
- `STOCK_SNAPSHOT` 的 OHLC fields 在 pre-open 可以是 `0.0`。
- 相同 `match_time`／`received_at` 可以有多筆 records，不能自動去重。
- terminal cursor、coverage、bars 與 closing `match_time` 是互補 diagnostics，
  任一項都不是單獨的 completeness proof。

完整 raw download 位於本地
`raw/teralion/twse/2026-07-27/2330/complete`，目前未加入版本控制。M1 fixture
在確認資料授權後，保存實際 regular `STOCK_SNAPSHOT` 與 `STOCK_REALTIME`
records；不得手工重造 payload。

## 11. Validation matrix

| Case | Expected |
| --- | --- |
| 合法 `STOCK_SNAPSHOT` complete book | 一個 atomic `QuoteSnapshot` |
| 合法 non-intermediate `STOCK_REALTIME` | 一個 atomic `QuoteSnapshot` |
| trial quote in pre-open window | 一個 `IndicativeOpeningAuction` |
| trial quote in pre-close window | 一個 `IndicativeClosingAuction` |
| trial `STOCK_REALTIME` intermediate/final `1+1` | 兩個同 phase auction events，intermediate 先 |
| regular book／deal／cum quantity | `TradingUnit`；不乘以 1,000 |
| `deal=null` | trade `NoObservation`，不清除 recent trade |
| `cum_volume=0` | 保留合法 zero |
| 少於五檔但合法 arrays | empty slots 明確取代舊 slots |
| level price／quantity 非法 | reject，不更新 clock／state |
| 缺少／無效 `match_time` | reject |
| `received_at` 與 `match_time` 順序相反 | 接受兩 clock，不以差值判錯 |
| 合法 intermediate/final `1+1` group | `TradeBatch -> QuoteSnapshot`；intermediate 不清 book |
| missing／multiple／volume-mismatch intermediate group | reject 整個 group |
| `INTRADAY_ODDLOT_REALTIME` | preserve raw、known skip、no event |
| unknown format | default reject |
| `status_flags` 4／8／16／128 | raw preserved；解碼 close／open／continuous／trial |
| `limit_flags=0` | 四組 annotations 都是 normal |
| `limit_flags & 0x03 == 1` | 緩跌；state／strategy update，禁止以該 event fill |
| `limit_flags & 0x03 == 2` | 緩漲；state／strategy update，禁止以該 event fill |
| reserved status／limit bits | raw preserved、warning、no guessed semantics |
| replay window 外但 download window 內 | raw retained、no timeline event |

## 12. Traceability

- `DATA-01`：TWSE kind／format 與實測 acquisition scope。
- `DATA-02`：raw preservation、format counts 與 source identity。
- `DATA-03`：invalid／unsupported／outside-window 分類。
- `DATA-05`：TWSE symbol／market 與 daily instrument metadata boundary。
- `REPLAY-01`：`QuoteSnapshot` mapping、wire／domain 分離及 atomicity。
- `REPLAY-02`：intermediate/final source phase ordering。
- `REPLAY-03`：complete snapshot replacement、`NoObservation` 與 raw flags。
- `REPLAY-06`：invalid fields、unknown format 及 invalid match-group rejection。
- `NFR-01`：deterministic mapping 與 warning aggregation。
- `NFR-03`：`TeralionTwseQuote` 的 numeric／quantity-unit mapping version
  boundary。

## 13. M5 warrant profile

M5 增加一個由實際來源固定的 TWSE warrant profile：`03003T`、交易日
`2026-07-20`、`regular`。fixture 位於
[`fixtures/teralion/twse/03003T/2026-07-20`](../../fixtures/teralion/twse/03003T/2026-07-20)，
source market 仍是 `twse`；它不是把所有未知 symbol 自動判成 warrant。適用 mapping
為 `TeralionTwseWarrant`，`mapping_version = 1`。

### 13.1 實測 format 與 mapping

| `format` | 筆數 | 行為 |
| --- | ---: | --- |
| `WARRANT_REALTIME` | 60 | 依 quote body 驗證後產生 `QuoteSnapshot` 或 trial auction event |
| `WARRANT_SNAPSHOT` | 51 | 依同一個完整 quote body 產生 `QuoteSnapshot` 或 trial auction event |

兩種 warrant format 都必須驗證 `type=quote`、`market=twse`、exact symbol、兩個
clock、最多五檔 bids／asks、`deal`、`cum_volume`、`status_flags` 與 `limit_flags`。
Warrant fixture 的 111 筆 source 產生 99 個 `QuoteSnapshot` 與 12 個
`IndicativeClosingAuction`；沒有用 `received_at` 取代 `match_time`，也沒有用
snapshot 差分重建 queue position。status／limit raw values 繼續以
`TwseQuoteAnnotations` 隨 event atomic 保存；reserved bits 仍保留並 warning。

Warrant source quantity unit 固定為 `TradingUnit`；本次 reference 的
`units_per_trading_unit = 1000`、currency `TWD`、multiplier `1`。Teralion daily
沒有完整提供 underlying、put/call、expiry 或 multiplier 時，normalizer 不猜測；
這些 static fields 由 TWSE OpenAPI warrant reference 與 config provenance 綁定，詳見
[M5 source selection evidence](../verification/evidence/m5/source-selection-2026-08-01.yaml)。

### 13.2 Strict boundary

`WARRANT_*` 在 equity profile 會被拒絕；warrant profile 收到 `taifex_opt` 或錯誤
symbol 會拒絕。未知 format、invalid book、invalid match time 與不相容 identity 都
是 strict error；不會將 warrant quote 降級成普通 equity 或把缺少的 metadata 由
symbol 猜出來。fixture positive／negative test 位於
`crates/normalizer/twse/tests/m5_fixture.rs`。
