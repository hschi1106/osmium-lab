# Data Sync 設計

## 1. 文件目的

本文件定義 M2 的 Teralion 資料規劃、cursor 下載、verified local source 發布、
完整性驗證及 replay cache 建置契約。設計目標是讓相同來源資料只下載一次，之後可在
無網路、無 API key 的環境重建 cache 並執行 replay／backtest。

```text
data_sync_design_version      = 1
source_manifest_version       = 1
source_revision_version       = 1
source_compression_policy     = ZstdPerPageV1
replay_cache_format_version   = 1
cache_descriptor_version      = 1
```

本文件固定 logical model、狀態轉移、持久化邊界與 deterministic identity，不固定
HTTP library、async runtime、Rust module 名稱或 YAML library。具體 Rust types 在
M2 step 2 定義，但不得改變本文語意。

依據：

- [產品需求](../product-requirements.md)
- [資料需求](../requirements/data.md)
- [M2 offline backtest](../increments/M2-offline-backtest.md)
- [Teralion interface](../interfaces/teralion.md)
- [Session window ADR](../architecture/decisions/0003-session-windows-and-strategy-activation.md)
- [Replay Engine](replay-engine.md)
- [Local data operations](../operations/local-data.md)

## 2. 責任與邊界

### 2.1 Data sync 負責

- 將 effective config 與 `SessionPlan` 解析成固定的 source partitions 及 queries。
- 讀取本地 catalog，規劃 reuse、download、resume、reject 或 repair。
- 只在 `sync` stage 建立 Teralion client 並走完 opaque cursor。
- durable 保存每頁 response、checkpoint 及每日商品資料。
- 驗證 query chain、payload、counts、schema、session ownership 與 checksum。
- 將成功 staging revision atomic publish 成 immutable `complete` source。
- 離線重新驗證 source。
- 由 complete source deterministic 建立、驗證及發布 replay cache。

### 2.2 Data sync 不負責

- 不解析 strategy 參數或執行 strategy。
- 不模擬 order、fill、fee、tax、position 或 P&L。
- 不把 `received_at` 當成 replay time。
- 不將 Teralion wire record 直接暴露給 Replay Engine。
- 不修改 complete source 來修補 normalizer 或 cache bug。
- 不在 `verify`、cache build、`replay` 或 `backtest` 隱式存取網路。
- 不將 Teralion `close`／`stats` 建立成 M2 domain event。

source data 與 derived cache 是不同信任邊界：

```text
Teralion response
-> untrusted staging bytes
-> verified immutable source revision
-> rebuildable replay cache
-> validated DomainEvent stream
```

## 3. Planning contract

### 3.1 `SourcePartitionKey`

M2 的 logical partition key 為：

```text
SourcePartitionKey {
    source_id
    market
    symbol
    trading_date
    selected_session_kinds
    session_plan_identity
}
```

`source_id` 在 M2 固定為 Teralion Feed Archive。`selected_session_kinds` 及
`session_plan_identity` 必須使用 canonical order。physical path 不是 identity，
相同 symbol 在不同 market、date 或 session plan 下不得互相覆寫。

immutable revision identity 另由已驗證內容產生：

```text
SourceRevisionIdentity {
    partition_key
    source_revision_version
    uncompressed_payload_set_checksum
    uncompressed_instrument_payload_checksum
    manifest_semantic_checksum
}
```

page size、retry 次數、worker count、staging path、下載時間及 cursor 值不得改變
partition key 或 revision identity。local compression level、zstd implementation
version及 compressed bytes也不得改變 semantic source revision identity。

### 3.2 `SourceQueryPlan`

每個 logical download window 產生：

```text
SourceQueryPlan {
    query_id
    endpoint_identity
    market
    symbol
    kinds
    received_at_start
    received_at_end
    expected_session_segments
    source_schema_identity
}
```

`query_id` 由 sanitized canonical fields 計算，不含 API key、header、cookie、
full cursor、signed URL、retry policy 或本機絕對路徑。所有 cursor pages 必須沿用
完全相同的 query plan；只有 opaque cursor 可以改變。

TWSE `2330 / 2026-07-27 / regular` 固定為：

```text
official session = 09:00–13:30 Asia/Taipei
download window  = [2026-07-27T08:55:00+08:00,
                    2026-07-27T13:35:00+08:00)
query clock      = received_at
replay window    = same absolute interval, evaluated by match_time
```

endpoint 若使用不同 end-inclusive convention，adapter 必須記錄 transport mapping，
但 published manifest 仍以 start-inclusive、end-exclusive window 表達。

### 3.3 Planning actions

planner 對每個 partition 只能產生下列一個 source action：

| Action | 條件 | Network |
| --- | --- | --- |
| `ReuseCompleteSource` | published revision 通過 identity 與基本 manifest 檢查 | 不需要 |
| `DownloadMissingSource` | 沒有 published 或 resumable revision | 需要 |
| `ResumeOrRestartBuilding` | 有相同 query identity 的 staging checkpoint | 需要 |
| `RejectIncomplete` | published state 不完整且 policy 未授權 repair/degraded | 不需要 |
| `RejectCorrupt` | checksum 或 manifest 損壞 | 不需要 |
| `CoverageUnavailable` | coverage 明確不包含 requested partition | 視 coverage evidence 而定 |

cache action獨立為 `ReuseValidCache` 或
`RebuildCacheFromCompleteSource`。cache missing／stale 不得把 source action 改成
download。

## 4. Online boundary 與 credential

只有 `sync` 可以建立 Feed Archive HTTP client。credential 從 runtime secret source
取得，僅存在 request adapter scope：

```text
CLI sync
-> acquire runtime credential
-> create Teralion client
-> execute frozen SourceQueryPlan
-> drop client and credential
```

以下資料不得包含 credential 或可恢復 credential 的內容：

- config 與 effective config。
- plan、query identity、cursor checkpoint。
- source manifest、cache descriptor、run artifacts。
- log、error、metrics、HTTP recording。
- request URL query、header dump 或 panic message。

`verify`、cache build、`replay`、`backtest` 與 `inspect` 不讀
`TERALION_API_KEY`，也不因 local data 不足而 fallback 至 `sync`。

## 5. Source compression

### 5.1 `ZstdPerPageV1`

M2 不在磁碟保存未壓縮的原始 JSON。每個 Teralion page及 daily instrument response
各自保存為單一 Zstandard frame：

```text
encoding          = zstd
compression_level = 3
frame_checksum    = enabled
dictionary        = none
file_suffix       = .json.zst
```

「原始 response bytes」指 HTTP content encoding解開後、JSON parse／normalization
之前的 exact response body。這些 bytes只在串流 pipeline中出現，不建立 `.json`
temporary或 published file。

per-page frame提供：

- cursor resume只需驗證已 committed pages。
- corruption可以定位至單一 page。
- cache builder可依 manifest ordinal串流解壓及 parse。
- 不需要單一整日 seekable archive或將完整 trading day載入記憶體。

M2不使用 zstd dictionary。未來只有在真實 dataset benchmark證明收益，並定義
dictionary bytes、identity、版本及保存方式後，才能新增另一個 compression policy。

### 5.2 Dual checksum

每個 compressed object保存：

```text
SourceObjectDigest {
    uncompressed_bytes
    uncompressed_sha256
    compressed_bytes
    compressed_sha256
    record_count
    compression_policy
    zstd_implementation
    zstd_version
}
```

- `uncompressed_sha256`驗證 source semantics，參與 payload-set及 source revision
  identity。
- `compressed_sha256`驗證實際 storage object，參與 manifest storage inventory，
  但不參與 semantic source revision identity。
- zstd version或 encoder output改變時，只要解壓後 exact bytes相同，source revision
  identity保持不變；既有 immutable object仍不得就地重壓。
- frame內建 checksum提供解碼時的額外 corruption detection，不能取代兩個 manifest
  SHA-256。

source payload checksum的 concatenation依 manifest page ordinal，使用明確 framing
包含 object kind、ordinal、uncompressed byte length及 uncompressed SHA-256；不得把
compressed file concatenation直接當成 semantic source identity。

### 5.3 Streaming write

page寫入固定流程：

```text
HTTP decoded response body
-> count/hash uncompressed bytes
-> Zstd level-3 streaming encoder with frame checksum
-> <ordinal>.json.zst.tmp
-> finish encoder + flush/fsync
-> hash compressed file
-> streaming decode and verify uncompressed hash/count/JSON envelope
-> atomic rename to <ordinal>.json.zst
```

任一步驟失敗都不得發布 page metadata/checkpoint。retry不得從 partial zstd frame續寫；
應移除或隔離該 exact temporary object，再重新寫入該 page。

## 6. Cursor state machine

### 6.1 State

每個 paged query 使用：

```text
NotStarted
-> Fetching(next_cursor)
-> PageStaged(page_ordinal, response_checksum, next_cursor)
-> Fetching(next_cursor)
-> Terminal(page_count)
-> Verified

任何非 terminal failure -> Interrupted(retry_state)
任何 invariant failure    -> Invalid(reason)
```

cursor 是 opaque transport value。程式只能保存、比較 exact bytes 並原樣傳回；
不得 decode、排序、截斷、正規化或自行產生 cursor。

### 6.2 Page commit

收到 page 後必須依序：

1. 確認 response 對應 frozen query identity。
2. 串流計算 uncompressed response checksum/byte count並壓縮成新的
   `.json.zst.tmp`。
3. 完成 encoder、flush/fsync，計算 compressed checksum，再串流解壓驗證。
4. atomic publish `.json.zst`，並寫入 page metadata：ordinal、record count、雙
   checksum、雙 byte counts、compression policy及 sanitized response identity。
5. durable 更新 checkpoint 的 next cursor。
6. 只有完成 checkpoint 後才允許要求下一頁。

checkpoint 不保存 full cursor 到 published manifest。staging 可以保存恢復所需的
opaque cursor，但必須限制權限、通過 secret scan，且 publish 時移除。

### 6.3 Cursor invariant

下列任一情況使 query `Invalid`：

- non-terminal cursor 未前進。
- cursor 重複或形成循環。
- page ordinal、page identity 或 exact response body 非法重複。
- query identity 在分頁期間改變。
- terminal 後仍出現 page。
- page 無法 parse 到 transport-level record envelope。
- retry 後同一 committed ordinal 解壓後的 exact response body不同。

合法 duplicate market records 必須保留。cursor/page duplicate detection 不能用
market record fingerprint 去重。

### 6.4 Retry 與 resume

只有 transport timeout、明確 retryable status 及暫時網路錯誤可以依 frozen policy
retry。schema error、cursor invariant、auth failure 或 query drift 不可無限 retry。

resume 時：

- 重新驗證所有已 committed page files 及 checkpoint。
- query identity 必須完全相同。
- 從最後 durable next cursor 繼續。
- 若服務無法安全 resume，建立新的 staging attempt；不得改寫舊 page。
- uninterrupted 與 resumed execution 必須具有相同 uncompressed source bytes、
  record order及 revision identity。compression implementation不同造成的 storage
  bytes差異不得改變 semantic revision。

## 7. Coverage、daily instrument 與 completeness

sync 在 ticks 前確認：

- market coverage 包含所需 trading date。
- symbol range 包含 `2330 / 2026-07-27`。
- trading date 已結束且由 exchange calendar 識別。
- daily instrument response 可以解析並關聯同一 market／symbol／trading date。

下列狀況不可合併：

| 狀況 | 結果 |
| --- | --- |
| coverage 不包含 | `CoverageUnavailable` |
| terminal cursor 且合法零筆 | 保存 `ZeroRecords` evidence，再由 completeness policy 判定 |
| cursor 未走完 | `incomplete` |
| request／auth／network failure | `building` 或 `incomplete`，附 retry action |
| payload checksum 不符 | `corrupt` |

terminal cursor 只證明 query chain 結束。`complete` 還需要 session plan、requested
windows、payload schema、daily instrument、counts、checksums 與 trading-date
ownership 全部通過。

## 8. Staging 與 atomic source publish

### 8.1 Staging revision

staging 是不可信且可恢復的工作區，包含：

```text
attempt.yaml
checkpoint.yaml
ticks/pages/<zero-padded-page-ordinal>.json.zst
ticks/pages/<ordinal>.yaml
instrument/daily.json.zst
verification.yaml
```

page ordinal 從零開始、固定寬度排序。程式不得依 filesystem enumeration 決定
concatenation order。

### 8.2 Published source revision

通過 verify 後，建立：

```text
manifest.yaml
ticks/pages/<zero-padded-page-ordinal>.json.zst
instrument/daily.json.zst
```

published manifest 至少包含：

- partition key、revision identity、state=`complete`。
- sanitized endpoint 與 query identities。
- logical session segments 及 requested download/replay windows。
- source formats、record/page counts及 uncompressed/compressed byte totals。
- 每頁 ordinal、record count、雙 checksum、雙 byte counts、compression policy、
  zstd implementation/version。
- terminal cursor reached evidence；不含 full cursor。
- daily instrument identity、checksum 與必要 economics provenance。
- calendar、session profile/window、source schema 及 adapter versions。
- acquisition／verification tool versions。
- unsupported／outside-window／zero-record summaries。
- checksum algorithms、semantic manifest checksum 及 publish protocol version。

revision checksum 在 manifest 完成前由所有 semantic inputs 計算。manifest 的
`semantic projection` 明確排除 revision identity 本身、semantic checksum 欄位及
operational acquisition timestamp，也排除 compressed checksum/size、compression
level及 zstd implementation/version，避免 self-referential或 storage-specific
identity。完整 manifest可以保存這些排除欄位，但它們不參與 revision identity。

### 8.3 Publish protocol

1. 在與最終 revision 相同 filesystem 建立 staging directory。
2. 驗證所有 bytes、metadata 及 manifest candidate。
3. flush files 及必要 parent directory metadata。
4. 以 revision identity 建立全新 immutable revision directory。
5. 若相同 revision 已存在，逐項解壓並以 uncompressed semantics驗證後 reuse；
   uncompressed內容不同但 identity相同是 fatal error。不得用新 compression output
   覆寫既有 immutable object。
6. atomic 更新 partition 的 `current.yaml` reference。
7. publish 成功後才向 planner catalog 顯示 `complete`。

不得覆寫既有 revision。crash、disk-full 或 rename failure 只能留下 staging／
unreferenced revision，不得讓 `current.yaml` 指向未驗證內容。

## 9. Verify 與 source state

對外狀態為：

```text
Missing
Building
Complete
Incomplete
Corrupt
```

分類順序固定：

1. path/reference 不存在：`Missing`。
2. 只有 staging 或 attempt 未 terminal：`Building`。
3. published metadata 可讀，但 query/completeness evidence 未滿足：`Incomplete`。
4. bytes、checksum、identity 或 schema 互相矛盾：`Corrupt`。
5. 所有 invariant 成功：`Complete`。

`verify` 完全離線，至少檢查：

- manifest version、partition key、revision identity。
- `current.yaml` 只指向同 partition 的 immutable revision。
- 所有 `.json.zst` page/instrument files存在，compression policy可用，compressed
  size/checksum及 zstd frame checksum相符。
- 串流解壓後 uncompressed size/checksum、JSON envelope及 record count相符，且
  data root中沒有 published uncompressed `.json` source payload。
- page ordinals 連續且 terminal evidence 存在。
- formats 可由目前 normalizer mapping 識別。
- `received_at` query ownership、calendar/session/trading-date ownership。
- replay window 分類及 outside-window summary 可重算。
- 必要 instrument economics 具有 value、source、version 及 applicable date。
- cache 若存在，其 lineage、bounds、version 及 payload checksum 相符。

verify 不修改 source bytes。repair 必須先由新的 plan 明示，並建立新 staging／
revision；不得在 verify 中就地修檔。

`Strict` 只接受 `Complete`。`ExplicitDegraded` 只允許 plan 已列出的 incomplete
scope，且永遠不接受 corrupt bytes、ordering violation 或缺少必要 economics。

## 10. Replay cache

### 10.1 Identity 與 descriptor

cache identity 綁定：

```text
ReplayCacheIdentity {
    partition_key
    source_revision_identity
    uncompressed_source_payload_checksums
    replay_cache_format_version
    normalizer_mapping_versions
    event_schema_version
    canonical_event_version
    ordering_rule_version
    session/calendar/window_policy_versions
}
```

descriptor 另保存 event count、first/last `OrderingKey`、payload byte count、
payload BLAKE3-256、build tool version 及 unsupported/outside-window counts。
output checksum 不參與 pre-build cache identity，但必須在 cache publish 前驗證。

### 10.2 Canonical payload

M2 cache payload 使用：

```text
magic               = ASCII "OSMCACHE1"
cache_format        = u16(1)
event_count         = u64 big-endian
records             = repeated {
    event_byte_len   = u32 big-endian
    canonical_event = exact canonical event version 1 bytes
}
```

records 依完整 `OrderingKey` non-decreasing 排列；合法 duplicate occurrence 不可
消除。payload checksum 涵蓋 header 與全部 records。YAML descriptor 不屬於 payload
checksum，但 descriptor semantic checksum 及 payload checksum 都進 cache lineage。

### 10.3 Build 與 publish

cache builder：

1. 只讀 complete source revision。
2. 依 manifest page ordinal串流解壓 `.json.zst`、驗證 uncompressed checksum並
   parse；不得先展開成磁碟 `.json`。
3. 將 wire record 交給 market normalizer。
4. 依 replay window 及 deterministic ordering 產生 canonical events。
5. 使用 bounded buffer 寫入新 staging payload。
6. 驗證 count、ordering bounds、EOF、payload checksum。
7. atomic publish 全新的 descriptor/payload directory。

invalid cache 不得在 replay 中逐 event fallback 到 source。planner 必須在 stream
open 前選擇 reuse、offline rebuild 或 fail。

### 10.4 Reader contract

reader 在產生第一個 event 前驗證 descriptor 與版本，之後：

- 只開啟 frozen universe/date 的 cache。
- bounded 讀取 length-prefixed record。
- 驗證每筆 canonical event、ordering monotonicity 與 expected instrument/date。
- 到 EOF 時確認 count、first/last key 及完整 payload checksum。
- checksum／EOF failure 使 run failed；不得把已讀 prefix 當成 complete result。

## 11. Failure taxonomy

| Category | 例子 | 可否自動 retry |
| --- | --- | --- |
| `Configuration` | 無法解析 session、日期未結束 | 否 |
| `Credential` | key 缺少或 auth rejected | 否 |
| `Coverage` | market/symbol/date 不可用 | 否 |
| `Transport` | timeout、retryable service error | 依 frozen policy |
| `CursorInvariant` | cursor loop、query drift | 否 |
| `SourceSchema` | envelope/format 不支援 | 否 |
| `Integrity` | checksum、count、identity 不符 | 否 |
| `Storage` | disk full、fsync、rename failure | 操作者處理後重試 |
| `VersionIncompatible` | manifest/cache/normalizer 不相容 | source 可驗證時 rebuild cache |

error 必須包含 stage、partition/query identity、market、symbol、trading date 及建議
action；不得包含 credential、full cursor 或 secret-bearing URL。

## 12. Determinism 與安全不變條件

- 同一 query response set 不受 page size、retry、resume 或 worker order 影響。
- complete source revision 永不就地修改。
- source checksum 與 cache checksum 是不同 identity。
- source 有效但 cache stale 時只離線重建 cache。
- backtest path 不建立 HTTP client。
- path、wall clock、random attempt ID 與 log 不進 domain identity。
- source JSON object key order只影響解壓後 exact source payload checksum；
  normalization後 canonical event不依賴 serializer order。
- zstd compression level/implementation/version及 compressed bytes不改變 semantic
  source revision；compressed object checksum仍必須精確符合 manifest。
- cache builder及 verify只串流解壓，不在 data root建立 uncompressed source file。
- filesystem permissions 至少避免其他使用者讀取 staging cursor checkpoint；API key
  永遠不落地。

## 13. Verification contract

至少需要：

- multi-page terminal cursor、cursor loop、cursor stall、query drift tests。
- retry/resume 與 uninterrupted publish uncompressed-semantics-identical test。
- per-page/daily-instrument zstd level 3、frame checksum、no-dictionary contract test。
- compressed corruption、frame checksum、compressed/uncompressed SHA-256 tests。
- 不產生 published或 temporary uncompressed `.json` source file test。
- 不同合法 zstd output仍得到相同 semantic source revision identity test。
- zero-record、coverage unavailable、partial page、disk-full／rename failure tests。
- `Missing`／`Building`／`Complete`／`Incomplete`／`Corrupt` classification tests。
- source atomic publish、immutable revision 及 second-sync zero-request tests。
- manifest count/checksum/provenance/secret-redaction tests。
- cache hit、stale/corrupt reject、delete-and-offline-rebuild tests。
- cache reader count/bounds/EOF/checksum 及 outside-universe sentinel tests。
- `2026-07-27 TWSE 2330` received-at/replay window boundary tests。
- network-disabled/no-key verify、cache build、replay 與 backtest tests。

穩定 test IDs 與 evidence schema 由
[Verification Plan](../verification/plan.md)固定。

## 14. Increment boundary

| Milestone | Scope |
| --- | --- |
| M2 | Teralion TWSE 2330 單日 cursor sync、verified immutable source、rebuildable cache、offline reuse |
| M3 | TAIFEX multi-segment、跨日 trading date、multi-instrument cache streams |
| M4 | TPEx、權證、選擇權 source/mapping extensions |

M2 不因未來 market 需求建立通用 data lake、分散式 scheduler 或任意 plugin source。
新增市場應沿用 partition、revision、state、publish 與 cache boundary，再增加
market-specific interface 及 fixtures。
