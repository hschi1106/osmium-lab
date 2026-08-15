# 資料流程與儲存

## 1. Artifact 類型

| Artifact | 權威來源 | 可否重建 | 執行中可否修改 |
| --- | --- | --- | --- |
| RunConfig | 使用者 | 是 | plan 建立後固定 |
| ExecutionPlan | effective config + 本地狀態 | 是 | 否 |
| Source staging | Teralion pages | 可重新下載或 resume | 只由 sync owner 修改 |
| Verified source revision | 已驗證 staging | 只能重新取得 | 否 |
| Replay cache | verified source + versions | 是 | 只在 prepare 階段建立 |
| MarketState | ordered events | 是 | 只由 reducer 更新 |
| Run artifacts | plan + execution | 可重跑但不覆寫 | 發布後否 |

## 2. 規劃

`RunConfig` 先經 schema、strategy registry、universe、session 與 economics 驗證，再 materialize 為 effective config。planner 為每個 instrument/date 建立 `SourcePartitionKey` 與 `SessionPlan`，檢查 source/cache catalog 後產生 action：

- reuse complete source/cache。
- download missing source。
- resume compatible staging。
- rebuild missing、stale 或 incompatible cache。
- reject incomplete／corrupt artifact。

plan 不寫入 source、cache 或 output，也不開啟 replay streams。

## 3. Source sync

```text
frozen query
  -> request page
  -> validate envelope and identity
  -> persist compressed page and checksums
  -> checkpoint opaque cursor
  -> repeat until terminal cursor
  -> verify manifest and payload
  -> atomic publish revision
```

source query 使用 planner 產生的 download window，以 `received_at` 篩選 Teralion archive。每頁保留 wire payload；cursor 只作 pagination，不參與 replay ordering。API key、authorization header 與 signed URL 不得進入 query identity 或持久化資料。

中斷時只有 frozen query identity 相容的 staging 可以 resume。HTTP error、parse error、重複 cursor、checksum mismatch 或未完成 cursor chain 都保持 `Building`／`Incomplete`，不會發布為 `Complete`。

## 4. Source verify

本地驗證至少包含：

- partition key、frozen query 與 session plan identity。
- `current.yaml` 指向已存在且 immutable 的 revision。
- manifest、page count、record count、compressed/raw checksums。
- terminal cursor evidence 與必要 instrument metadata。
- tick page envelope、JSON 可解析性與 record count 一致性。

coverage 或 range 只能作 discovery，不能單獨證明 partition 完整。`data verify` 確認 source artifact 的結構與內容完整性；market-specific item 語意由 cache prepare 階段的 normalizer 驗證。verify 是只讀操作，不自動修補 source。

## 5. Normalization 與 cache

cache builder 只接受 verified source。每筆 wire record 依 market interface分類：

```text
supported timeline format -> validated DomainEvent
known non-timeline format  -> KnownSkipped + reason
outside replay window      -> preserved source diagnostic
unknown / invalid shape    -> strict error or explicit degraded warning
```

normalizer 不使用 page order、line number、`received_at` 或 worker completion 補造市場順序。產生的 events 依 canonical ordering 排序並寫入 cache；descriptor 保存：

- cache format 與 event schema。
- source revision checksum。
- normalizer mapping identity。
- ordering、session 與 canonical encoding identity。
- event count、warning／skip summary 與 payload checksum。

cache 完成後以 atomic publish 加入 catalog。descriptor 不相容或 payload 損壞時拒絕讀取；由 verified source 重建即可。

## 6. Replay 與 backtest

replayer 根據 plan 只開啟 explicit universe 的 cache streams：

```text
validate bindings
  -> bounded k-way merge
  -> replay clock / MarketState
  -> optional Strategy
  -> optional Simulation / Accounting
  -> checksums and artifacts
```

`replay` 執行 event/state 路徑並輸出 deterministic summary。`backtest` 在相同 replay 路徑上加入 strategy、simulation、accounting 與 run publication。執行中不會下載 source、重建 cache 或擴張 universe。

## 7. Run publication

output 必須是尚不存在的目錄。writer 先建立 staging，依執行狀態寫入：

- run manifest 與 status。
- effective config checksum、execution plan identity 與 strategy metadata。
- source/cache provenance 與版本集合。
- warnings、strategy outputs、orders、fills、positions 與 performance。
- event stream、final state、ledger 等 canonical checksums。

成功結果完成所有 validation 與 reconciliation 後才 publish。失敗結果可保留診斷與 partial evidence，但必須明確標示 failed，不得偽裝成 successful run。`inspect` 驗證 artifacts 間的 identity 與 checksum，不重新執行回測。

## 8. 本地目錄

```text
<data_root>/
  source/teralion/<market>/<date>/<symbol>/
    partition.yaml
    current.yaml
    revisions/<source-revision>/
    staging/<attempt>/
  cache/replay/teralion/<market>/<date>/<symbol>/<cache-identity>/
```

run output 由 `--output` 或設定指定，可放在 `data_root` 外。實際檔案與復原方式見 [本地資料](../operations/local-data.md)。

## 9. 失敗處理

| 問題 | 行為 |
| --- | --- |
| 缺少 source | plan 要求 sync；離線執行拒絕 |
| staging 中斷 | 相容時 resume，否則建立新 attempt |
| published source checksum 錯誤 | 標記 corrupt 並拒絕使用 |
| cache 缺少或不相容 | 由 verified source rebuild |
| cache checksum 錯誤 | 拒絕讀取並 rebuild |
| event／state／accounting error | run failed，保留可追溯診斷 |
| output 已存在 | 拒絕，不覆寫 |

移除 cache 不影響 source。published source 或 run artifacts 不應手動就地修改。
