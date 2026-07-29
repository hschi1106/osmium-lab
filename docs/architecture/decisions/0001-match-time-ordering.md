# ADR-0001：以 `match_time` 與版本化內容鍵排序事件

- 狀態：Accepted
- 決策日期：2026-07-29
- 適用版本：`OrderingRuleV1`
- 主要需求：`REPLAY-02`、`REPLAY-04`、`NFR-01`、`NFR-03`

## 1. Context

`osmium-lab` 必須把多個商品、market 及 source format 的歷史事件合併成一條可重現
timeline。

Teralion 提供的 `match_time` 是唯一可用 replay time，但相同 `match_time` 可能
出現：

- 同一商品的多筆 source ticks。
- 不同商品的事件。
- 不同 market 或 format 的事件。
- 完全相同的重複事件。

來源資料不提供可宣稱為「全市場真實封包順序」的共同 sequence。檔案順序、API page
順序及 concurrency completion order 也不能成為穩定的 domain 語意。

因此需要一個：

- 第一鍵一定是 `match_time`。
- 對相同輸入產生固定 total ordering。
- 只使用事件本身的穩定資料。
- 不虛構交易所因果關係。
- 可以隨 schema 明確版本化的 tie-break。

## 2. Decision

採用 `OrderingRuleV1`。所有 accepted domain events 以完整 `OrderingKeyV1` 遞增
排序：

```text
OrderingKeyV1 = (
    match_time,
    market_rank,
    symbol,
    source_format,
    event_kind_rank,
    source_sequence,
    event_fingerprint
)
```

比較是欄位逐項的 typed lexicographic comparison；不是 locale-aware 字串排序，
也不依賴 Rust `Debug` 或任一 serializer 的預設輸出。

### 2.1 `match_time`

`match_time` 永遠是第一排序鍵。

- 使用經 market／format normalizer 驗證的 domain `MatchTime`。
- 必須保留來源能證實的精度，不得用 lossy floating-point 表示。
- domain type 必須提供跨平台相同的 total order。
- 缺少、無效或無法安全轉換的值不得進入 ordering。
- TAIFEX `trading_date` 決定 partition 歸屬，但不取代 `match_time` 的事件排序。

`MatchTime` 的 exact Rust representation 由
[market types 設計](../../design/market-types.md)根據真實 fixtures 定義。

### 2.2 `market_rank`

`OrderingRuleV1` 使用固定 rank：

| Market | Rank |
| --- | ---: |
| TWSE | 1 |
| TPEx | 2 |
| TAIFEX | 3 |

新增 market 必須分配新 rank 並 review ordering compatibility。既有 rank 不得重新
編號。unknown market 不能以臨時字串排序進入 timeline。

rank 只用於 deterministic tie-break，不表示 market 優先權或真實先後。

### 2.3 `symbol`

symbol 使用已驗證 domain identifier 的 canonical bytes 做 unsigned lexicographic
comparison：

- 不使用 locale collation。
- 不做未經 market interface 定義的大小寫轉換。
- 不以 filesystem path 或 display name 代替。
- canonicalization 規則是 domain schema 的一部分。

若不同 market 使用相同 symbol，`market_rank` 已先區分。

### 2.4 `source_format`

source format 使用 normalizer registry 的穩定 canonical format identifier bytes
排序。

identifier 必須：

- 從 source payload／manifest 可追溯。
- 在相同 mapping version 下保持不變。
- 不使用 Rust type name、module path 或 runtime registration order。

source format 排序同樣不代表來源封包優先權。

### 2.5 `event_kind_rank`

`OrderingRuleV1` 固定：

| Event kind | Rank |
| --- | ---: |
| `QuoteSnapshot` | 10 |
| `BookSnapshot` | 20 |
| `TradeBatch` | 30 |
| `MarketStat` | 40 |
| `MarketStatus` | 50 |

rank 保留間隔以便未來新增事件，但新增 kind 仍必須 review 並更新 event schema；
不能只插入數字後宣稱完全相容。

同一 source tick 內的 book、trade、volume 與 flags 依 `REPLAY-01` 保持單一原子
event，因此不能藉 event kind rank 拆開同一 tick。

### 2.6 `source_sequence`

若 source event 具有經 interface 文件確認、會隨 payload 保存且跨重跑穩定的
sequence／counter，將它放入 `source_sequence`：

```text
None < Some(value)
```

只允許：

- source 明確提供的 counter。
- 已成為 event schema 一部分並由 fixture 證實語意的值。

不允許：

- API page number。
- download cursor。
- file line number。
- directory enumeration index。
- normalization worker completion index。
- 單純因本次 ingestion order 產生的 ordinal。

若來源沒有合法 sequence，使用 `None`，由 fingerprint 完成 tie-break。

### 2.7 `event_fingerprint`

`EventFingerprintV1` 定義為：

```text
BLAKE3-256(CanonicalEventV1(event))
```

比較使用 32-byte digest 的 unsigned lexicographic order。

選擇 BLAKE3-256 的理由：

- 對大量事件具良好 throughput。
- 跨平台輸出固定。
- 32-byte digest 使 collision 風險足以低於平台其他資料風險。
- 可同時重用於 fixture／golden diagnostics，但不取代 source checksum 的邊界。

fingerprint 不是安全簽章，不驗證資料來源身分；source integrity 仍由 source
checksum、manifest 與 provenance 負責。

## 3. `CanonicalEventV1`

fingerprint input 必須是 domain event 的 canonical encoding，不是 source JSON、
cache serializer、Rust memory layout 或 debug output。

`CanonicalEventV1` 遵守：

1. 開頭包含 canonical encoding version 及 event schema version。
2. event envelope 依 schema 固定欄位順序編碼。
3. payload 依 event kind 的 schema 固定欄位順序編碼。
4. enum 使用明確固定 discriminant。
5. integer 使用固定 width 與 byte order。
6. string／bytes 使用明確 length prefix 加原始 bytes。
7. optional value 使用 `absent`／`present` tag；absent 不等於零。
8. sequence 先編碼長度，再依 domain-defined order 編碼元素。
9. price、quantity 以 exact domain representation 編碼，不用 display string。
10. unknown raw flags／values 以其無損 domain representation 編碼。
11. 禁止 map iteration order、locale、timezone default、memory padding 及 NaN-dependent
    ordering。

每個 event 的所有 domain-significant 欄位都必須進入 canonical payload。欄位加入、
移除、重新解讀或 encoding 改變必須更新 event／canonical version。

具體 field layout 由 [market types 設計](../../design/market-types.md)列出並以 golden
encoding tests 固定。

## 4. Ordering 與 stream contract

### 4.1 單 stream

每個 cache／event stream 必須以與 `OrderingRuleV1` 相容的順序提供事件，或由 reader
在交給 merge 前建立該順序。

reader 必須偵測：

- `match_time` 倒退。
- 完整 OrderingKey 倒退。
- 不相容 ordering／event schema version。

發現錯誤依 `REPLAY-06` 停止；不得局部重排後隱藏 cache corruption，除非該操作是
明確的 cache rebuild。

### 4.2 多 stream merge

merge 比較各 stream head 的完整 `OrderingKeyV1`，取最小值：

```text
heads = selected_streams.current_events()
next = min_by(OrderingKeyV1, heads)
emit(next)
advance(next.stream)
```

stream discovery order、buffer size、I/O completion 或 worker count 不參與選擇。

### 4.3 完全重複事件

若兩個事件的：

- 完整 OrderingKeyV1 相同，且
- `CanonicalEventV1` bytes 相同，

則它們是 ordering-equivalent duplicates。

決策：

- 不在 ordering／merge 層自動去重。
- 每個 duplicate 都作為 accepted event 處理。
- 每個 duplicate 都造成 state version 依 ADR-0002 增加一次。
- 因事件內容完全相同，兩者互換不改變 canonical event sequence 的內容。

若需要偵測或拒絕來源重複，必須是另外的資料完整性 policy，不能由 sorter 靜默
刪除。

## 5. Replay clock 與 callback

排序完成後，每個事件依下列順序：

```text
select minimum OrderingKeyV1
-> advance clock to event.match_time
-> atomically update MarketState
-> strategy sees current event and updated state
-> process strategy output
```

相同 `match_time` 的 clock value 可以維持不變，但 event/state version 繼續逐一
推進。strategy 在某事件 callback 中只能看到 ordering 中較早及目前事件。

## 6. Versioning 與 compatibility

執行結果與依賴排序的 replay cache 必須記錄：

- `OrderingRuleV1`
- event schema version
- `CanonicalEventV1`
- `EventFingerprintV1`／BLAKE3-256

下列變更需要新的 ordering rule 或明確 compatibility review：

- key 欄位順序改變。
- market／event kind rank 改變。
- source sequence presence／comparison 改變。
- fingerprint algorithm 改變。
- canonical encoding 改變而可能改變 fingerprint。
- `MatchTime` comparison 語意改變。

不相容 cache 必須拒絕並由 complete source data 重建。不得將舊、新 rule 的 streams
混合在同一 execution。

## 7. Consequences

### 7.1 正面結果

- 相同資料與版本產生相同 event order。
- shuffled input、parallel normalization 及 stream discovery 不影響結果。
- 多 market／symbol 可以用同一 merge contract。
- ordering provenance 可寫入 run manifest。
- 不需要虛構 source global sequence。

### 7.2 成本與限制

- 每個 event 需要 canonical encoding 與 fingerprint；cache build 有額外 CPU 成本。
- rank 與 encoding 變更需要版本管理及 cache rebuild。
- 平台順序可能與實際 exchange packet order 不同。
- 完全重複 events 不自動去重，可能重複 callback；這是保留來源事實的刻意選擇。

### 7.3 不代表的語意

本決策不代表：

- market rank 較小的事件真實較早。
- event kind rank 是 exchange priority。
- fingerprint order 具有市場意義。
- 相同 `match_time` 間存在真實因果關係。
- source counter 是跨商品的全域 sequence。

## 8. Considered alternatives

### 8.1 只使用 `match_time`

拒絕。相同時間事件的順序會受 input／sort stability 影響，無法保證重跑一致。

### 8.2 保留 source file／API page order

拒絕。page size、retry、cache rebuild、filesystem order 或 download concurrency
都可能改變順序，且不代表全市場語意。

### 8.3 只用 ingestion ordinal

拒絕。ordinal 是本次處理副作用；打亂輸入或 parallel normalization 會改變結果。

### 8.4 只用 event fingerprint

拒絕作為完整 key。它雖 deterministic，但會失去可診斷的 market、symbol、format
及 event kind grouping。保留 fingerprint 作最後 collision-resistant tie-break。

### 8.5 猜測 exchange priority

拒絕。來源精度不足，會把平台假設誤標為市場事實。

### 8.6 自動刪除 duplicates

拒絕。平台無法只憑內容判斷重複是來源錯誤或真實重複紀錄，靜默刪除會改變事件
數與策略 callback。

## 9. Verification

至少需要：

- 同一 events 以多種 input order 產生相同 ordered canonical bytes。
- 相同 `match_time` 跨 market、symbol、format、kind 的 golden order test。
- `None`／`Some(source_sequence)` comparison test。
- fingerprint golden vectors。
- 不同 locale、timezone、worker count 的結果一致測試。
- 完全 duplicate 不被去重的測試。
- ordering version mismatch 拒絕／cache rebuild test。
- 多 stream discovery order 不影響 event checksum 的 test。

M1 對應 `M1-AC-03` 與 `M1-AC-04`；M3 補多商品 merge evidence。

## 10. Traceability

- [產品需求](../../product-requirements.md)：`REPLAY-02`、`NFR-01`、`NFR-03`
- [回播需求](../../requirements/replay.md)：`REPLAY-02`、`REPLAY-04`
- [操作與非功能需求](../../requirements/operations.md)：`NFR-01`、`NFR-03`
- [M1 增量](../../increments/M1-twse-replay.md)：ordering 與 checksum 驗收
- [資料與執行流程](../data-flow.md)：single／multi-stream merge
