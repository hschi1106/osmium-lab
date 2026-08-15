# 本地資料

`data.data_root` 是使用者管理的資料目錄。repository fixture 不會自動複製到這個位置。

## 1. 目錄結構

```text
<data_root>/
  source/teralion/<market>/<date>/<symbol>/
    partition.yaml
    current.yaml
    revisions/<revision-id>/
      manifest.yaml
      ticks/pages/<ordinal>.json.zst
      ticks/pages/<ordinal>.yaml
      instrument/daily.yaml
    staging/<attempt-id>/
      checkpoint.json
  cache/replay/teralion/<market>/<date>/<symbol>/<cache-id>/
    descriptor.yaml
    events.bin
```

實際 identity 同時包含 source、session plan 與版本；上圖只顯示可讀路徑層級。run output 由 `--output` 指定，不一定位於 `data_root`。

## 2. Artifact 生命週期

| 類型 | 信任狀態 | 可否重建 | 操作規則 |
| --- | --- | --- | --- |
| Published source revision | verified、immutable | 需重新下載 | 不就地修改或覆寫 |
| `current.yaml` | atomic revision reference | 可由 catalog 檢查 | 只由 sync publish |
| Staging attempt | 未完成、未受信任 | 可 resume／捨棄 | 只由相同 query 的 sync owner 修改 |
| Replay cache | source-bound derived artifact | 可離線重建 | 不就地修改；發布新 identity |
| Run artifacts | 單次執行 evidence | 可重跑但不覆寫 | publication 後 immutable |

## 3. Source state

| State | 意義 | 建議處理 |
| --- | --- | --- |
| `Missing` | 沒有可用 source | `plan` 後執行 `data sync` |
| `Building` | 有未完成 staging | 以相同 config 重新 sync 以 resume |
| `Complete` | current revision 通過驗證 | reuse |
| `Incomplete` | cursor、metadata 或 completeness evidence 不足 | 檢查 staging／query，重新 sync |
| `Corrupt` | manifest、payload 或 checksum 不一致 | 停止使用，重新取得 source |

`data verify` 只讀 current revision。`Complete` 不只代表 terminal cursor，也要求 identity、manifest、payload、instrument metadata 與 checksums 一致。

## 4. Compression 與 checksum

source pages 使用 per-page zstd artifact，並保存 compressed 與 raw content identity。checksum 以實際 bytes 與版本化 canonical metadata 計算，不包含 credential 或不穩定的操作文字。

cache descriptor 另保存 source revision、normalizer mapping、event schema、ordering rule、session identity、event count 與 payload checksum。任何不相容都使 cache 不可讀，但不改變 source 完整性。

## 5. 安全操作

檢查狀態：

```sh
osmium plan --config config.yaml
osmium data verify --config config.yaml
```

重建 cache：

```sh
osmium cache prepare --config config.yaml
```

可直接移除明確的 cache identity 目錄後重建；不要移除整個 `data_root`、source revision 或無法確認範圍的路徑。published source 損壞時應保留診斷資訊並重新 sync，不要手動修正 checksum 或 manifest。

API key 不得出現在 `data_root`。分享錯誤資訊時，只提供 sanitized path、partition identity、revision/cache checksum 與 error category。

## 6. Backup 與搬移

需要保存不可重新取得的資料時，優先備份 `source/`；cache 可重建。搬移 `data_root` 後更新 config path，再執行 `data verify` 與 `plan`。不要只複製 `current.yaml` 而省略其 revision。

完整資料流見 [資料流程與儲存](../architecture/data-flow.md)。
