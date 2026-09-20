# 資料流程、identity 與 artifacts

本頁是 acquisition、source/cache lifecycle、lineage/checksum 與 run publication 的 canonical
文件。Provider boundary 與各市場 wire mapping 入口見[Teralion 介面](../interfaces/teralion.md)。

## Artifact lifecycle

| Artifact | Owner / authority | Rebuildable | Publication rule |
| --- | --- | --- | --- |
| RunConfig | 使用者 | 是 | plan 後視為固定 input |
| ExecutionPlan | validated config + local catalog | 是 | deterministic identity，不寫 source |
| Source staging | provider sync attempt | 可 resume/re-download | 只有 sync owner 可修改 |
| Verified source revision | provider evidence + verification | 只能重新取得 | immutable + atomic publish |
| Replay cache | source + mapping/schema/ordering versions | 是，離線 | derived + atomic publish |
| MarketState | ordered DomainEvents | 是 | memory state，只由 reducer 更新 |
| Run artifacts | plan + replay + strategy/execution/accounting | 可用新 output 重跑 | create-new，發布後 immutable |

最重要的區分：**verified source 是可重用的事實來源；replay cache 是可刪除、可重建的衍生物。**
Cache stale 不代表 source 要重新下載。

## Planning

RunConfig 經 schema、strategy registry、universe、session、contract 與 economics validation 後物化為
effective config。Planner 為每個 instrument/date 建立 `SessionPlan` 與 `SourcePartitionKey`，檢查
source/cache catalog，再建立 frozen `ExecutionPlan`：

- complete/compatible artifact → reuse；
- missing source → download；
- compatible staging → resume；
- missing/stale cache → rebuild；
- incomplete/corrupt artifact → reject。

CLI 只在 composition root 注入 provider mapping resolver。Runner 消費 `PlannedPartition` 已物化的
session、contract、economics 與 identity，不再從 YAML 重推。

## Online acquisition 與 credential boundary

```text
frozen query
  → request page
  → validate envelope / partition identity
  → persist compressed page + checksums
  → checkpoint opaque cursor
  → repeat to terminal cursor
  → verify manifest / payload / completeness
  → atomic publish source revision
```

目前 Teralion adapter 依 planner download window 以 `received_at` 查 archive；它只是 source selection
欄位，不是 replay time。API key、authorization header 與 signed URL 不得進入 query identity、disk
artifact 或 log。Cursor 只作 pagination，不參與 event ordering。

中斷時，只有 frozen query/partition identity 相容的 staging 可 resume。HTTP、JSON、cursor loop、
record count 或 checksum failure 都不會發布 `Complete` revision。

## Source verification

`data verify` 是離線且只讀，至少檢查：

- partition、query 與 session-plan identity；
- `current.yaml` 指向存在的 immutable revision；
- manifest/page/record counts 與 compressed/raw checksums；
- terminal cursor evidence、必要 metadata 與 wire JSON 可解析性。

Coverage/range discovery 不能單獨證明 partition 完整。Market-specific item semantics 在 normalization
階段驗證；repository integrity 不列舉 provider wire formats。

## Normalization 與 cache publication

Cache builder 只接受 verified source。Selected normalizer 對每筆 wire record產生：

```text
supported timeline format → validated DomainEvent
known non-timeline format → KnownSkipped + reason
outside replay window     → source diagnostic
unknown / invalid shape   → strict error or explicit degraded warning
```

Normalizer 不使用 page order、line number、`received_at` 或 worker completion 補造市場順序。Events 依
canonical ordering 排序後寫入 cache；unknown format 沒有 generic fallback。

### Cache identity

`CacheDescriptor` 保存 cache identity、source revision、instrument/date、partition identity、event count/
time range、payload SHA-256，以及 mapping/schema/ordering versions。Identity 明確包含：

- cache format version；
- verified source revision、instrument、trading date 與 partition identity；
- normalizer mapping **name + version**；
- `MARKET_TYPES_VERSION`；
- `EVENT_SCHEMA_VERSION`；
- `CANONICAL_EVENT_VERSION`；
- `ORDERING_RULE_VERSION`；
- event count 與 canonical payload checksum。

因此 provider mapping、合法 domain value set、canonical encoding 或 ordering semantics 只要改變，舊
cache 就不會被 current descriptor 認為 current。Current code truth：

```text
cache_format=3
market_types=11
event_schema=9
canonical_event=9
TWSE quote/warrant mapping=11/7
TPEx quote/warrant mapping=9/8
TAIFEX future/spread/option mapping=5/2/4
```

Compatibility failure 或 payload corruption 會拒絕讀取；由 verified source 重建，不提供舊 cache
migration reader。

## Selective replay 與 lineage

```text
ExecutionPlan
  → validate cache bindings
  → open explicit-universe streams only
  → bounded deterministic merge
  → ReplayClock / MarketState
  → optional Strategy / Execution / Accounting
  → checksums / artifacts
```

Replay 不下載 source、不 normalize、不 rebuild cache、不擴張 universe。Binding 將 plan、partition、
source revision 與 cache identity連起來；event stream/final state checksum 證明實際 replay 結果。

## Run publication

Output 必須不存在。Writer 在 sibling staging directory 建立完整檔案，為 artifacts 計算 BLAKE3，將
checksum index 寫入 `run-manifest.yaml`，sync 後 rename publish。只有完成 validation、finalization 與
accounting reconciliation 的 successful run 走此 publication path；錯誤不會偽裝為成功 run。

Artifact 分成：

- **Identity**：run manifest、effective-config checksum、plan identity、strategy metadata、version set；
- **Lineage**：source revision、cache identity；
- **Replay evidence**：event-stream 與 final-state checksum、replay summary；
- **Execution evidence**：strategy output、orders、fills；scheduled mode 另有 execution trace；
- **Accounting result**：ledger、positions、performance；scheduled mode另有 fill costs/cash charges；
- **Diagnostics**：warnings 與 run summary。

完整檔名與 inspect 行為見[使用指南](../user-guide.md#backtest-artifacts)。`inspect` 驗證 manifest version
與列出的 artifact checksum，只讀既有 run，不重跑 strategy。

## Local layout 與 recovery

```text
<data_root>/
  source/<source>/<market>/<date>/<symbol>/
    partition.yaml
    current.yaml
    revisions/<source-revision>/
    staging/<attempt>/
  cache/replay/<source>/<market>/<date>/<symbol>/<cache-identity>/
```

Source namespace 目前通常是 `teralion`，但 layout/cache codec 不解讀 Teralion fields。不要手改
`current.yaml`、revision、descriptor 或 manifest；精確狀態與安全 recovery 見
[本地資料](../operations/local-data.md)。
