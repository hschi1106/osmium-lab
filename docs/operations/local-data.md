# Local Data 操作契約

## 1. 文件目的

本文件定義 release run config 的本地來源資料、staging、replay cache 與 run artifacts 的目錄布局、
操作流程、修復方式及安全邊界。它讓使用者能判斷哪些資料可重用、哪些衍生檔可刪除，
以及中斷或損壞後應如何恢復。

```text
local_data_contract_version = 2
current_scope               = v2 TWSE/TPEx/TAIFEX partitioned source and cache
```

資料內容及狀態演算法由 [Data Sync 設計](../design/data-sync.md)定義；CLI command
及 exit status 由 [CLI 操作契約](cli.md)定義。

## 2. 資料分類

| 類別 | 信任／生命週期 | 可否刪除重建 | 可否就地修改 |
| --- | --- | --- | --- |
| Published source revision | 已驗證、immutable、可跨 backtest 重用 | 只有重新下載才能取回 | 不可 |
| Partition `current.yaml` | 指向已發布 complete revision 的 atomic reference | 可由 catalog 修復 | 只能由 publisher atomic 更新 |
| Staging attempt | 未完成、未受信任、可 resume 或丟棄 | 可以 | 只由 sync owner 修改 |
| Replay cache | derived、source-bound | 可以，由 local source 離線重建 | 不可；建立新 cache |
| Run artifacts | 某次 execution 的 immutable evidence | 不可由 cache 重建相同策略結果以外的 operational metadata | 不可 |
| Log／temporary diagnostics | 非 domain result | 可以 | 可以 |

source revision 不是 cache，cache 也不能取代 source provenance。刪除 cache 不應造成
任何 Teralion request；刪除唯一 source revision 則可能需要重新下載。

## 3. Data root layout

Release 使用單一 user-configured `data_root`。推薦布局：

```text
<data_root>/
  catalog/
    catalog-version.yaml
  source/
    teralion/
      twse/
        2026-07-27/
          2330/
            partition.yaml
            current.yaml
            revisions/
              <source-revision-id>/
                manifest.yaml
                ticks/
                  pages/
                    00000000.json.zst
                    00000001.json.zst
                instrument/
                  daily.json.zst
            staging/
              <attempt-id>/
                attempt.yaml
                checkpoint.yaml
                ticks/pages/
                instrument/
                verification.yaml
  cache/
    replay/
      twse/
        2026-07-27/
          2330/
            <cache-identity>/
              descriptor.yaml
              events.bin
  runs/
    <user-selected-run-directory>/
```

規則：

- path component 使用 canonical source／market／date／symbol representation。
- symbol 必須先通過 market-specific identifier validation，不得含 path separator、
  `.`／`..` traversal 或 platform-specific reserved component。
- directory traversal order 不參與任何 checksum 或事件排序。
- manifest 與 descriptor 內保存 relative logical identity；machine-specific absolute
  path 只可出現在非 canonical diagnostics。
- Release 不使用 symlink 作 correctness boundary；`current.yaml` 保存 revision identity，
  reader 必須重新驗證它指向同 partition。

repository 的 `fixtures/` 是 committed acceptance fixture，不是使用者的
`data_root`，也不是 live source catalog。release CLI 不接受 `--fixture`，也不得自動
掃描或修改 user data root。

多商品 run 使用相同 contract 的 partitioned layout；每個 instrument/date/session selection
有獨立 source current pointer 與 cache identity：

```text
<data_root>/
  source/teralion/taifex/2026-07-20/TXFH6/
    partition.yaml
    current.yaml
    revisions/<source-revision>/manifest.yaml
  cache/replay/teralion/taifex/2026-07-20/TXFH6/<cache-identity>/
    descriptor.yaml
    events.bin
```

`TXFH6`、`CDFH6` 的 after-hours 與 regular source items 可以在同一 partition
revision 中保存；session ownership 由 `SessionPlan` 決定，不由 directory date 或
wire calendar date 猜測。`CAFH6` 只有 regular window。cache builder 依 market
normalizer mapping 與 partition identity 產生 source-bound cache；刪除 cache 不會
改變 source revision。

## 4. Partition files

### 4.1 `partition.yaml`

描述 stable partition identity 與 logical session scope，至少包含：

```text
local_data_contract_version
source_id
market
symbol
trading_date
selected_session_kinds
session_plan_identity
```

它不表示資料 complete，也不保存 API key、cursor 或結果 checksum。

### 4.2 `current.yaml`

只保存：

```text
reference_version
partition_identity
source_revision_identity
manifest_semantic_checksum
```

reader 不因 `current.yaml` 存在就信任資料；仍須開啟 revision manifest 並驗證
identity/checksum。reference 指向不存在、不相容或其他 partition 時為 `Corrupt`。

### 4.3 Revision directory

revision directory 只有 atomic publisher 可以建立。成功發布後：

- 內容 immutable。
- manifest state 固定為 `complete`。
- 頁面依 manifest ordinal 讀取，不依 filename discovery 推測。
- 手動新增、刪除或修改任何 file 都會使 verify 判為 `Corrupt`。
- 新內容必須建立新的 revision identity，不可覆寫舊 revision。

### 4.4 Staging directory

staging 不可用於 replay/backtest。`attempt.yaml` 必須提供：

- partition/query identity。
- attempt identity 與狀態。
- safe resume capability。
- last committed page ordinal。
- failure category 及下一步；若失敗。

staging 內可以有恢復所需 opaque cursor，publish 時不得複製到 revision。其
filesystem permissions 應限制為 data-root owner 可讀寫。

### 4.5 Source compression

published/staging source payload只使用
[Data Sync](../design/data-sync.md)的 `ZstdPerPageV1`：

```text
ticks page       = <ordinal>.json.zst
daily instrument = daily.json.zst
zstd level       = 3
frame checksum   = enabled
dictionary       = none
```

data root不得保存解壓後的原始 `.json` source payload，包括 temporary files。
sync、verify及 cache builder都以 streaming decoder處理 `.json.zst`。

manifest對每個 object保存：

- uncompressed byte count、record count及 SHA-256。
- compressed byte count及 SHA-256。
- compression policy、zstd implementation/version。

uncompressed checksum決定 source semantic identity；compressed checksum只驗證目前
storage object。重新壓縮可能改變 compressed bytes，但不得就地重壓 immutable
revision，也不得因此改變解壓後相同資料的 source revision identity。

一般工具檢查可以使用：

```sh
zstd --test <page>.json.zst
zstd --decompress --stdout <page>.json.zst | <read-only-inspection-command>
```

檢查資料時只串流至 stdout/pipe，不應用 `zstd -d` 在 source revision旁產生
`.json`。上述 CLI工具只供人工診斷；formal verify使用平台實作及 manifest checksums。

## 5. State inspection

使用者看到的狀態：

| State | 意義 | 預設 backtest | 建議 action |
| --- | --- | --- | --- |
| `Missing` | 沒有 source/reference/staging | 拒絕 | `plan` 後 `sync` |
| `Building` | 有未 terminal attempt | 拒絕 | resume；query 不相容時明確 restart |
| `Complete` | published revision 全部驗證成功 | 允許 | reuse |
| `Incomplete` | bytes 可讀，但 cursor/completeness evidence 不足 | 拒絕 | inspect reason，建立 repair plan |
| `Corrupt` | checksum、identity、schema 或 reference 矛盾 | 拒絕 | 隔離，從其他 complete revision 恢復或重新 sync |

不能用 directory 非空、最後修改時間、檔案數大於零或 terminal cursor 單一條件推定
`Complete`。

`ExplicitDegraded` 只能針對 plan 已列出的 incomplete scope，不能接受 corrupt
revision。Reference acceptance 必須使用 `Strict`。

## 6. 常見操作

### 6.1 第一次準備資料

```text
osmium plan        --config <file>
osmium data sync   --config <file>
osmium data verify --config <file>
```

完成後檢查 plan／sync／verify summary：

- partition 為 `Complete`。
- terminal cursor reached。
- source revision、payload checksum、page/record count 可見。
- daily instrument 與 economics provenance 可見。
- secret scan 無 finding。

`sync` 成功應已發布 complete source；`verify` 是獨立離線重驗，不是補下載。

### 6.2 第二次執行

相同 config 再次 `plan`／`sync`：

- source action 是 `ReuseCompleteSource`。
- HTTP request count 是零。
- source revision/checksum 不變。
- valid cache 存在時 action 是 `ReuseValidCache`。

若第二次 `sync` 仍要求下載，應先停止並 inspect identity difference，不要直接刪除
complete source。

### 6.3 Cache rebuild

cache 可以安全刪除單一明確 target：

```text
<data_root>/cache/replay/<market>/<date>/<symbol>/<cache-identity>/
```

之後執行 `verify`/plan 應顯示 `RebuildCacheFromCompleteSource`，再由 cache prepare
或 `run` 的 cache stage 離線建立。重建必須：

- 不要求 API key。
- HTTP request count 為零。
- source revision/checksum 不變。
- 產生相同 cache payload checksum。

不要以 recursive broad target 刪除整個 `data_root`、`source/` 或使用 unresolved
glob。CLI 若未提供專用 cache-prune command，刪除由操作者對已 inspect 的完整
cache identity 執行。

### 6.4 Interrupted sync

中斷後：

1. `plan` 顯示 `ResumeOrRestartBuilding`。
2. inspect attempt 的 query identity、last committed page 與 failure category。
3. identity 相同且 checkpoint valid 時 resume。
4. identity 或 checkpoint 不相容時保留舊 attempt，建立新的 attempt。
5. 新 revision complete 並發布前，舊 complete revision 若存在仍保持可用。

不得把 staging page 手動移到 revision directory，或編輯 checkpoint 偽造 terminal。

### 6.5 Corrupt source

`verify` 判定 `Corrupt` 時：

1. 停止使用該 revision。
2. 保存 manifest、verify report、identity 與 mismatch diagnostics。
3. 檢查同 partition 是否有另一個完整 immutable revision。
4. 有合法 revision 時，以 atomic reference repair 指向該 revision。
5. 沒有時由明確 repair plan 重新 sync，發布新 revision。

不得就地重寫 checksum、count 或 source JSON。corrupt revision 可以隔離或在證據
保留後移除，但這是 destructive operation，必須指定 exact revision identity。

### 6.6 Incomplete source

先區分：

- cursor 未 terminal。
- coverage／zero records 無法證明 completeness。
- daily instrument 缺少。
- unsupported format。
- session/calendar ownership 無法確認。

只有 cursor/transport 中斷可直接 resume。schema、calendar 或 metadata 問題應先修正
design/adapter/version，再建立新 revision。不得用 cache builder 忽略 source
incomplete。

## 7. Run artifacts

Run output 不得發布到 cache/source directory。`--output` 必須是不存在的新
directory，publisher 先在同一 parent 建 staging，成功後 atomic rename。

successful backtest 至少包含：

```text
effective-config.yaml
execution-plan.yaml
run-manifest.yaml
data-lineage.yaml
cache-lineage.yaml
strategy.json
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

failed run 可以發布明確標示的 diagnostic directory，但不得包含
`outcome: passed`、完整 performance 或 complete checksum 欄位。inspect 必須先讀
manifest status，再決定哪些 artifacts 合法。

run artifacts 可以備份或搬移；其 canonical lineage 依 checksum，不依 absolute
data-root path。搬移後 inspect 不需網路，但若要深度驗證外部 source/cache bytes，
必須由使用者提供可解析的新 data root。

## 8. Disk space 與 atomicity

sync/cache/run publisher 在開始前應估算或檢查可用空間，但預估不足不能取代寫入
錯誤處理。atomic publish 要求 staging 與 final target 位於支援 atomic rename 的同一
filesystem。

disk-full 或程序 crash 後：

- `current.yaml` 不得指向 partial revision。
- complete revision 保持不變。
- partial cache/run staging 不得被正常 discovery。
- cleanup 採 best effort；不能因 cleanup 失敗掩蓋原始 failure。

跨 filesystem copy 不能冒充 atomic rename。若 data root 不支援必要 durability，
preflight 應停止並給出 `Storage` error。

## 9. Concurrency 與 lock

同一 partition 同一時間只能有一個 publisher owner。logical lock 至少包含：

- partition identity。
- owner process/attempt diagnostics。
- lock protocol version。
- acquisition time 與 stale-lock recovery evidence。

lock 的 wall-clock/owner PID 不參與 source identity。reader 可以並行讀 immutable
complete revision；publisher 更新 `current.yaml` 不得破壞已開啟 reader 的 revision。

stale lock 不能只因超過固定時間就靜默刪除。操作者或 recovery logic 必須確認 owner
不存在、checkpoint durable，並記錄 takeover。

## 10. Security

- `data_root`、source、cache、runs、logs 及 staging 都納入 secret scan。
- API key、authorization、bearer、cookie、signed URL query 永不進 published data。
- full opaque cursor 只可存在受限 staging checkpoint，不進 published manifest/run。
- secret scan必須能串流解壓 `.json.zst`檢查 uncompressed payload；只掃描 compressed
  bytes不足。
- source payload 若服務本身回傳 credential-like field，publish 前必須依 interface
  contract 明確分類；不可在不留 provenance 的情況下任意 redaction。
- error/inspect 預設顯示 sanitized identity，不 dump raw HTTP request。
- acceptance config 可提交 repository，但不得引用個人絕對路徑或 secret value。

## 11. 備份、移轉與清理

備份優先順序：

1. published source revisions、partition metadata/current references。
2. run artifacts。
3. replay cache；可省略並在目的地重建。
4. staging；通常不備份，除非需保留失敗診斷。

移轉後先執行離線 `verify`，再 rebuild/reuse cache。不能只複製 `current.yaml` 而不複製
它指向的 revision。

安全清理規則：

- 先由 inspect/manifest resolve exact identity。
- 先清 staging 與 unreferenced cache。
- source revision 只有在確認非唯一 provenance、備份存在或接受重新下載後才移除。
- material deletion 後記錄刪除 identity、時間、操作者及是否可恢復。
- 不提供或建議對 workspace root、home directory、`data_root` root 使用 broad
  recursive delete。

## 12. 操作驗證

至少驗證：

- 第一次 sync 發布 complete revision。
- 相同 config 第二次 sync zero HTTP requests。
- interruption 後 resume 與 uninterrupted revision 相同。
- complete revision 不可靜默覆寫。
- source page及 daily instrument只以 per-object `.json.zst`保存，data root沒有
  uncompressed JSON payload。
- compressed/uncompressed checksum、frame checksum及 streaming decode驗證成功。
- compression implementation差異不改變 semantic source revision identity。
- 五種 state 與建議 action 正確。
- cache 刪除後在 network-disabled/no-key 環境重建。
- source corrupt 不被 cache rebuild 掩蓋。
- outside-universe cache stream 不開啟。
- run output existing path 拒絕，failed staging 不冒充 success。
- data root 搬移後 checksum identity 與 offline inspect 不變。
- manifest、checkpoint、cache、run artifact 及 log secret scan。

對應穩定 test IDs 見 [Verification Plan](../verification/plan.md)。
