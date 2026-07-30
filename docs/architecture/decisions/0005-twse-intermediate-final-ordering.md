# ADR-0005：TWSE intermediate trade 必須先於同撮合時間的 final quote

- 狀態：Accepted
- 決策日期：2026-07-30
- 適用契約：`TeralionTwseQuote`、`OrderingRule`
- TWSE mapping version：`2`
- ordering rule version：`2`
- 主要需求：`REPLAY-01`、`REPLAY-02`、`REPLAY-03`、`REPLAY-04`、`REPLAY-06`、
  `NFR-01`、`NFR-03`

## 1. Context

TWSE 逐筆交易的一次撮合可能揭示多個成交價量：

- 非最後一筆成交只揭示成交，不揭示最佳五檔。
- 最後一筆成交同時揭示最佳五檔。

Teralion `STOCK_REALTIME` 對應為：

```text
intermediate_print=true
-> deal present
-> bids=[]
-> asks=[]

intermediate_print=false
-> final deal
-> complete bids/asks
```

intermediate record 不能建構 `QuoteSnapshot`，因為 empty arrays 在此代表沒有 book
observation，不代表完整 order book 為空。domain schema 已能將它表達為不修改 book
的 `TradeBatch`。

原 `OrderingRule` version 1 在相同 `match_time` 時先比較 event kind：

```text
QuoteSnapshot rank = 10
TradeBatch rank    = 30
```

因此同一撮合結果可能錯排為：

```text
final QuoteSnapshot
-> intermediate TradeBatch
```

這會讓 final cumulative volume 被較小的 intermediate value 覆蓋，且 final
MarketState 的 recent trade 停在非最後一筆成交。

## 2. Fixture evidence

證據是本機 verified source：

```text
raw/teralion/twse/2026-07-27/2330/complete
```

完整資料含 16 pages、77,213 records；其中 `STOCK_REALTIME` 有 70,199 records。
三個包含 intermediate print 的 exact-`match_time` groups 為：

| `match_time` | Intermediate | Intermediate cumulative | Final | Final cumulative |
| --- | --- | ---: | --- | ---: |
| `09:28:49.274622` | `2340 × 1` | 5,616 | `2345 × 7` | 5,623 |
| `09:30:55.252155` | `2345 × 7` | 6,055 | `2350 × 6` | 6,061 |
| `10:29:59.907157` | `2360 × 2` | 9,339 | `2365 × 1` | 9,340 |

每個 group 都：

- 恰好有一筆 `intermediate_print=true`。
- 恰好有一筆 `intermediate_print=false`。
- 兩筆具有完全相同的 `match_time`。
- final record 具有完整五檔。
- final cumulative volume 等於 intermediate cumulative volume 加 final deal
  quantity。

前兩組的 `received_at` 完全相同；第三組相差 81 microseconds。這證明
`received_at` 不能成為一致的 replay ordering key。API page、array index 與
ingestion ordinal 同樣不具有可攜的 market semantics。

TWSE B.12.13「揭示項目註記」已定義 intermediate 是非最後一筆、final 是最後一筆，
所以兩個 phase 的相對順序有 exchange contract 支持，不是平台猜測。

## 3. Decision

### 3.1 保留兩個 source observations

不 coalesce、不刪除，也不合成 book：

```text
intermediate source record
-> TradeBatch {
       trades: [Intermediate trade]
       trade_order: SourceOrdered
       cumulative_volume: Set(intermediate cumulative volume)
       annotations
   }

final source record
-> QuoteSnapshot {
       complete book
       trade: Set(final trade)
       cumulative_volume: Set(final cumulative volume)
       annotations
   }
```

single-element `TradeBatch` 的內部順序是 trivially `SourceOrdered`。這不宣稱 API
page order 是 exchange sequence。

### 3.2 驗證 match group

`TeralionTwseQuote` mapping version 2 引入、version 3 保留的規則是在產生
domain events 前，以：

```text
(market, trading_date, symbol, source_format, match_time)
```

建立 source match group。grouping 必須跨 API page boundary，且不能依 page size、
worker completion 或 input discovery order 改變。

若 group 包含 `intermediate_print=true`，version 2 只接受已由 fixture 證實的 shape：

1. 恰好一筆 intermediate record。
2. 恰好一筆 final record。
3. intermediate 有 deal、沒有 book observation。
4. final 有 deal 及合法 complete book。
5. 兩筆的 quantity/cumulative-volume unit 相容。
6. `final.cum_volume = intermediate.cum_volume + final.deal.quantity`。

任何條件不成立都以 `UnsupportedRealtimeMatchGroup` 拒絕整個 group。不得只接受
final、只略過 intermediate、以 `max(cum_volume)` 修補，或利用 input order 配對。

若未來 fixture 出現多筆 intermediate，同樣先拒絕，再由新 mapping version 根據
新增證據定義 group semantics。

不含 intermediate 的普通 final records 繼續各自映射為 `QuoteSnapshot`；同
`match_time` 有多個普通 final records 不會被此規則錯誤 coalesce。

### 3.3 Source phase rank

`OrderingRule` version 2 在 `source_format` 後、`event_kind_rank` 前加入
`source_phase_rank`：

```text
OrderingKey = (
    match_time,
    market_rank,
    symbol,
    source_format,
    source_phase_rank,
    event_kind_rank,
    source_sequence,
    event_fingerprint
)
```

rank 固定為：

| Event shape | Rank |
| --- | ---: |
| 非 TWSE `STOCK_REALTIME` event | `0` |
| TWSE `STOCK_REALTIME` `TradeBatch`，且所有 prints 都是 `Intermediate` | `10` |
| TWSE `STOCK_REALTIME` `QuoteSnapshot` final record | `20` |

phase rank 必須由 canonical event 的 `market`、`source_format`、event kind 與 trade
kind 純函式重建，不加入新的 mutable metadata，也不進 `CanonicalEvent` bytes。

不符合表中合法 shape 的 TWSE `STOCK_REALTIME` domain event 是 validation error，
不能臨時指定 rank。

## 4. Resulting replay

第一組 fixture 的 replay 必須是：

```text
TradeBatch(2340 × 1, cumulative=5616)
-> QuoteSnapshot(2345 × 7, cumulative=5623, complete book)
```

因此：

```text
after intermediate:
    recent_trade      = 2340 × 1
    cumulative_volume = 5616
    book              = previous complete book

after final:
    recent_trade      = 2345 × 7
    cumulative_volume = 5623
    book              = final complete book
```

每個 source record 仍形成一個 atomic transition、state version 與 strategy
callback。final state 不依 raw API order、normalization concurrency 或 event kind
rank 的偶然結果。

## 5. Versioning and compatibility

- `TeralionTwseQuote.mapping_version` 從 `1` 提高為 `2`。
- `OrderingRule` 從 `1` 提高為 `2`。
- `event_schema_version` 與 `canonical_event_version` 維持 `1`；domain event
  fields、discriminants 與 canonical bytes 沒有改變。
- 依賴 ordering 的 version 1 replay cache 與 result identity 不相容，必須由
  verified local source rebuild。
- source partition 不需重新下載。

mapping version 1 的 default rejection 不再是 current behavior。version 2 對三個
已驗證 groups 正常產生 events；未知 group shape 走一般 schema-change rejection。

## 6. Rejected alternatives

### 6.1 只交換全域 event kind rank

拒絕。把所有 markets 的 `TradeBatch` 排在 `QuoteSnapshot` 前，會把 TWSE-specific
證據誤套到 TAIFEX／TPEx 及其他 formats。

### 6.2 使用 `received_at`

拒絕。它是 capture clock，不是 replay time；fixture 中相同 phase pair 有時相同、
有時不同，也可能受 capture path 影響。

### 6.3 使用 API page／array order

拒絕。page size、cursor、retry 與 cache rebuild 不能成為 market semantics。

### 6.4 以 cumulative volume 排序所有同時間 events

拒絕作為 generic rule。cumulative volume 不是 source sequence，跨 market／session
可能有不同 reset semantics。本決策只用它驗證已配對 group 的一致性。

### 6.5 Coalesce 為一個 QuoteSnapshot

拒絕。這會合併兩個 source observations、減少 callback/state transition 數，且需要
改變 `QuoteSnapshot` trade schema 才能無損保存兩筆成交。

### 6.6 忽略 intermediate

拒絕。它是來源明確提供的成交 observation；靜默刪除會改變 trade history、event
checksum 與策略可見資料。

## 7. Verification

至少需要：

1. 三個 fixture groups 各自產生 `TradeBatch -> QuoteSnapshot`。
2. shuffled raw input、不同 page boundary 與 worker count 產生相同 events/order。
3. intermediate 的 book 保留，final 的 complete book 完整 replacement。
4. cumulative volume 依 `5616 -> 5623`、`6055 -> 6061`、`9339 -> 9340` 前進。
5. final MarketState recent trade 分別為 `2345 × 7`、`2350 × 6`、`2365 × 1`。
6. missing final、multiple intermediate、multiple final、volume mismatch 與 invalid
   book 都以 `UnsupportedRealtimeMatchGroup` 拒絕整個 group。
7. `OrderingRule` version 1 cache 被拒絕並可由既有 source rebuild。
8. `STOCK_SNAPSHOT` canonical events 與 ordering relative order 不變。

## 8. Traceability

- [產品需求](../../product-requirements.md)：`REPLAY-01`、`REPLAY-02`、
  `REPLAY-03`、`REPLAY-04`
- [Replay requirements](../../requirements/replay.md)：event atomicity、ordering、
  state replacement、failure policy
- [TWSE interface](../../interfaces/twse.md)：`TeralionTwseQuote` mapping version
  3；沿用本 ADR 在 version 2 引入的 ordering
- [Market types](../../design/market-types.md)：`TradeBatch`、`QuoteSnapshot`、
  `TradePrintKind`
- [MarketState design](../../design/market-state.md)：atomic reducer 與 cumulative
  volume validation
