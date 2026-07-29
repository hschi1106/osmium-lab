# 操作與非功能需求

## 1. 文件目的

本文件將[產品需求](../product-requirements.md)中的 `OPS-01`、`OPS-02` 及
`NFR-01` 至 `NFR-03` 細化為可設計、實作與驗證的系統需求。

本文件定義使用者工作流程、執行結果、可重現性、效能、安全與版本行為，不固定：

- CLI framework、subcommand spelling 或 argument syntax
- config serialization format
- run directory layout
- log framework
- checksum algorithm
- benchmark harness
- concurrency architecture

上述選擇由 operations、design 與 verification 文件記錄，但必須讓一份設定可以
完成常見工作、資料同步與回測可分離，且回測預設離線。

## 2. 使用者工作流程

平台必須支援下列概念流程：

```text
plan -> sync -> verify -> replay/backtest -> inspect
```

- `plan`：解析設定並比較本地資料狀態，產生 execution／sync plan。
- `sync`：取得缺少的 Teralion 資料並發布已驗證來源資料。
- `verify`：重新檢查本地來源資料及衍生 cache 的完整性與相容性。
- `replay/backtest`：離線回播事件，選擇性執行策略、fill 及帳務。
- `inspect`：檢查 plan、manifest、warning、order、fill、position 及結果。

實際命令可以整合或拆分，但資料準備與 backtest 必須能獨立執行。

## 3. 通用定義

### 3.1 Run configuration

`run configuration` 是使用者對日期、universe、策略、資料 policy、replay、
simulation 及輸出的明確設定。effective configuration 包含使用者值及套用後的
default。

### 3.2 Execution plan

`execution plan` 是執行前解析、驗證並固定的工作描述，至少包含：

- 工作類型
- market、symbol、trading date universe
- 每個資料 partition 的處置
- strategy identity 與參數
- replay／simulation model identity
- degraded policy；若啟用
- 預期輸入與輸出

### 3.3 Run manifest

`run manifest` 是一次 execution 的持久、可檢查記錄，描述實際使用的資料、版本、
設定、結果狀態及產出位置。

### 3.4 Successful、failed 與 degraded

- `successful`：所有必要資料與 invariant 通過，執行完成。
- `failed`：執行未完成或任一必要 invariant／reconciliation 失敗。
- `degraded`：使用者事前明確允許不完整資料，執行完成但結果不可視為完整資料
  backtest。

failed 或 degraded 不得以零 warning 的 successful 外觀呈現。

## 4. OPS-01：操作流程

### OPS-01.1 一份設定

常見工作必須能由一份 run configuration 表達，至少涵蓋：

- trading date 或 date range
- explicit market／symbol universe
- strategy identity 與參數
- data source／local data policy
- replay options
- fill、slippage、fee、tax 及 accounting options；若執行 backtest
- output／inspection options

設定可以引用 credential 的取得方式，但不得包含要寫入結果、log 或版本控制的
API key value。

effective configuration 必須在執行前可檢查。default value 必須明確，不能依
本機 locale、filesystem order 或未記錄環境狀態改變。

### OPS-01.2 頂層工作入口

平台必須提供一個明確頂層入口，使使用者不需手動串接內部 crates 即可執行常見
流程。

頂層入口必須能：

- 只建立 plan。
- 依 plan 同步及驗證資料。
- 在資料已備妥時直接 replay／backtest。
- 完成 plan、sync、verify、replay/backtest 的整合流程。
- inspect 既有 run，而不重新執行 backtest。

命令名稱與 subcommand layout 由 [CLI 操作](../operations/cli.md)決定。

### OPS-01.3 Plan

plan 必須在有副作用的下載或 backtest 前顯示至少：

- 可直接使用的 market／symbol／trading date partitions
- 需要下載的 partitions
- building、incomplete 或 corrupt partitions
- coverage 不包含或無法確認的 partitions
- 將使用或重建的 replay cache
- strategy universe
- 是否為 degraded run

只執行 plan 時不得下載 market data、建立成功 run result 或修改已驗證來源資料。
讀取 metadata、coverage cache 或執行必要的本地檢查可以允許，但任何網路需求都
必須清楚顯示。

### OPS-01.4 Sync

sync 必須：

- 只下載 plan 判定缺少或需要明確修復的資料。
- 遵守 `DATA-01` cursor 完整性。
- 不靜默覆寫 complete source partition。
- 在中斷後留下 building／incomplete 而非 complete。
- 產生可供 verify 與 inspect 的資料摘要。

只需要 replay 已有完整資料時，不得強迫執行 sync。

### OPS-01.5 Verify

verify 必須能在不執行策略的情況下檢查：

- source partition manifest 與 checksum
- completeness status
- source schema 可識別性
- replay cache provenance、checksum 與版本相容性
- 必要商品 metadata

verify 不得用重建 cache 掩蓋 source corruption。若允許自動重建 derived cache，
plan 與結果必須清楚記錄該動作。

### OPS-01.6 Replay 與 backtest

資料準備完成後，replay／backtest 必須預設：

- 不存取網路。
- 不需要 Teralion API key。
- 只開啟 execution plan universe 的 streams。
- 在開始策略前驗證資料及版本相容性。
- 遇到不完整或 corrupt data 時停止；除非明確 degraded policy 允許。

`replay` 可以只產生事件／狀態結果；`backtest` 另包含 strategy、simulation 與
accounting。實際是否使用不同命令由 CLI design 決定。

### OPS-01.7 Inspect

inspect 必須能在不重跑 backtest 的情況下讀取並呈現：

- run status
- effective configuration
- data partitions 與 checksums
- schema／model versions
- event counts、warning 與 skipped data
- strategy outputs
- orders、fills、positions 及 P&L；若存在
- event／state／result checksums
- failure 或 degraded 原因

inspect 對 failed 或 partial run 必須清楚標示資料不完整，不得把缺少結果顯示成
合法零值。

### OPS-01.8 錯誤訊息

與市場資料相關的錯誤必須盡可能指出：

- market
- symbol
- trading date
- source format
- `match_time` 或無效原始值；若可用
- 失敗階段
- 原因
- 建議處理方式

與策略或 simulation 相關的錯誤必須另包含可用的 strategy、event、order 或 fill
identity。

錯誤訊息不得包含 API key、credential header 或其他 secret。多個 partition 失敗
時，可以提供摘要與詳細附件，但不得只回報最後一個錯誤而隱藏範圍。

### OPS-01.9 Exit status 與失敗可見性

頂層入口必須以 machine-detectable status 區分：

- 成功
- 明確 degraded success
- 使用者設定錯誤
- 資料缺少／不完整／損壞
- 外部服務或網路失敗
- 不相容版本
- strategy／simulation／reconciliation failure

實際 exit code taxonomy 由 CLI design 決定。任何 failed run 不得回傳與 successful
run 無法區分的狀態。

### OPS-01.10 驗收條件

`OPS-01` 至少必須由下列證據驗證：

- 一份設定建立完整 plan 的 acceptance test。
- plan 正確區分 reuse、download、incomplete 及 corrupt data 的測試。
- sync 與 backtest 可分開執行的測試。
- 第二次執行不重新下載 complete source data 的測試。
- 無網路及無 API key 的 replay／backtest 測試。
- universe 外 stream 不被開啟的測試。
- inspect 不重跑策略即可讀取結果的測試。
- failed／degraded／successful status 可由程式區分的測試。
- error context 與 secret redaction 測試。

M2 必須以 TWSE 2330 單日資料提供第一個完整
`plan -> sync -> verify -> backtest -> inspect` 證據。

## 5. OPS-02：執行結果

### OPS-02.1 Result identity 與狀態

每次 replay／backtest execution 必須具有穩定的 run identity，並保存：

- run status
- 開始及結束時間；作為操作 metadata，不可成為 replay time
- platform／build identity
- effective configuration
- execution plan identity 或內容

wall-clock timestamps 可以每次不同，因此不得納入要求跨執行相同的 domain result
checksum。

### OPS-02.2 Data provenance

每次結果必須記錄實際使用的：

- market／symbol／trading date partitions
- source data checksums
- source schema／format identities
- replay cache checksums；若使用
- 商品 metadata 及 multiplier provenance；若使用
- incomplete／skipped data 及原因

只保存資料路徑不足以作為 provenance，因路徑內容可能改變。

### OPS-02.3 Version provenance

每次結果必須記錄：

- event schema version
- ordering rule version
- canonical encoding／checksum version；若分開管理
- strategy identity/version
- fill model/version
- fee、tax、slippage model/version
- accounting／marking policy version
- cache format 或 source mapping version；若影響結果

不適用的模型必須明確標示未使用，不能與版本遺失混淆。

### OPS-02.4 Strategy 與 simulation 設定

每次 backtest 結果必須保存：

- strategy name／identity
- effective strategy parameters
- explicit universe
- order／fill model 及參數
- slippage、fee、tax
- initial cash 及 accounting settings
- 每個商品 multiplier value 與來源
- random seed；若平台允許 randomness

secret value 不得成為 effective configuration 的可輸出內容。

### OPS-02.5 Processing summary

每次結果至少必須包含：

- source tick／domain event count；以可用層級分別記錄
- first／last `match_time`
- warning count 及詳細 warning reference
- rejected／skipped data count 及範圍
- strategy callback／output count
- order／fill count；若執行 simulation
- 執行時間
- peak memory 或 throughput；若該 run 為 benchmark

零值與 unavailable 必須區分。例如未執行 simulation 的 fill count 應標示不適用，
不能假裝執行後得到零 fill。

### OPS-02.6 Trading 與 accounting result

backtest 結果至少必須提供或可由 result artifact inspect：

- order records，包含 rejected／cancelled／unfilled
- fill records
- final cash
- final positions
- realized／unrealized P&L
- total fee
- total tax
- basic performance summary
- accounting reconciliation status

所有數值必須符合 [simulation requirements](simulation.md) 的 traceability 與
reconciliation 規則。

### OPS-02.7 Checksums

每次執行必須提供可用的：

- normalized event stream checksum
- final MarketState checksum
- strategy output checksum；若有
- order／fill／ledger 或整體 domain result checksum；若有

checksum 必須以具版本的 canonical content 計算，不得依賴：

- source JSON whitespace 或 object key order；除非明確是 source file checksum
- absolute local path
- wall-clock timestamp
- process ID
- debug formatting
- hash map iteration order

不同 checksum 的資料邊界及版本必須可檢查，不能只輸出一個無法判斷涵蓋內容的值。

### OPS-02.8 Failed 與 degraded result

failed run 可以保存 partial artifacts 供診斷，但必須：

- status 明確為 failed。
- 記錄最後成功階段與失敗原因。
- 將 partial outputs 與成功結果區分。
- 不產生或不發布看似完整的績效摘要。

degraded result 必須：

- status 明確為 degraded。
- 列出 incomplete partitions、略過範圍及 warning。
- 保存啟用 degraded mode 的 effective setting。
- 不使用與完整 successful result 相同但無標記的 presentation。

### OPS-02.9 可攜與可檢查

run manifest 必須有 machine-readable representation，並能由 inspect 提供人類可讀
摘要。

result artifact 不得要求連線 Teralion 才能檢查 provenance、設定或基本結果。
若大型明細另存，manifest 必須能定位、驗證 checksum 並指出缺少附件。

### OPS-02.10 驗收條件

`OPS-02` 至少必須由下列證據驗證：

- manifest 包含所有必要 data／version／strategy／simulation 欄位的 schema test。
- replay-only 與 full backtest 正確區分 not-applicable 欄位的測試。
- event／state／strategy／ledger checksum golden tests。
- failed result 不發布成功績效的測試。
- degraded result 列出不完整範圍的測試。
- inspect 可離線讀取 manifest 與 artifacts 的測試。
- manifest 及 artifact secret scan。

M1 先提供 event／state checksum 與策略輸出；M2 提供完整 run manifest、交易與 P&L。

## 6. NFR-01：可重現

### NFR-01.1 Reproducibility identity

可重現性比較至少固定：

- source data checksums
- event schema、normalizer mapping 與 ordering versions
- effective configuration
- strategy implementation identity 與參數
- fill、slippage、fee、tax、accounting 及 marking versions
- instrument metadata／multiplier values 與來源
- random seed；若使用

上述 identity 相同時，domain result 必須相同。

### NFR-01.2 必須相同的結果

相同 reproducibility identity 必須得到相同：

- normalized events 及順序
- warning／skipped data 集合
- final MarketState
- strategy callback sequence 及 outputs
- order／fill sequence
- cash、positions、fee、tax 及 P&L
- domain checksums

wall-clock runtime、CPU scheduling、log timestamps 或實體檔案路徑可以不同，但不得
改變 domain result。

### NFR-01.3 Optimization invariance

下列變更不得在相同版本語意下改變結果：

- worker count
- thread scheduling
- bounded buffer size
- stream discovery order
- cache hit／offline rebuild path
- 合法的 I/O prefetch
- 不改變需求語意的效能最佳化

若最佳化必須改變 event、fill 或 accounting 語意，必須更新對應版本並視為明確
行為變更，不能仍聲稱相同 reproducibility identity。

### NFR-01.4 Golden result policy

golden result 只能因下列原因更新：

- 經 review 的需求變更。
- 經 source fixture 或官方格式文件證實的 mapping 修正。
- 明確的 schema、ordering、model 或 canonical encoding version 變更。

更新必須同時說明原因、受影響範圍及預期 checksum／行為差異。

### NFR-01.5 驗收條件

`NFR-01` 至少必須由下列證據驗證：

- 同一 run 重複執行的 checksum test。
- 打亂 input／stream discovery order 的測試。
- cache hit 與 cache rebuild 結果比較。
- 不同允許 worker count 的結果比較；加入並行時。
- golden result update review rule。
- domain checksum 排除 wall-clock metadata 的測試。

## 7. NFR-02：效能

### NFR-02.1 優先改善方向

平台應優先減少：

- 已完整資料的重複下載
- 每次 backtest 重複解析全部 source JSON
- universe 外商品 I/O
- 將完整市場或期間載入記憶體
- 不必要的 replay cache rebuild

效能設計不得犧牲完整性、determinism、offline execution 或錯誤可見性。

### NFR-02.2 Benchmark dataset

首版效能門檻必須在取得實際資料後決定，不得在需求證據不足時任意指定。

benchmark 至少分階段使用：

- TWSE 2330 實際單日資料
- TAIFEX futures 實際交易日資料，包含跨日 session；M3
- 2330 與 futures 多商品 replay；M3

資料集必須記錄 checksum、market、symbol、trading date、事件數及使用限制，使
結果可比較。

### NFR-02.3 Metrics

benchmark 至少量測：

- sync throughput；適用時
- normalization throughput
- replay events per second
- end-to-end backtest duration
- peak memory
- bytes read／written 或可比較的 I/O 指標
- cache build 與 cache hit duration

量測環境、build profile、platform、worker count、設定及版本必須記錄。

### NFR-02.4 Acceptance threshold

正式數值 threshold 只有在 baseline benchmark 完成並由
[效能驗證](../verification/performance.md)記錄後才成為驗收條件。

在 threshold 尚未建立前，仍必須驗證結構性要求：

- 第二次執行不重新下載 complete source data。
- cache hit 不重新解析全部 source JSON。
- universe 外 stream 不開啟。
- replay 使用 streaming／bounded memory。

### NFR-02.5 Regression

建立 baseline 後，效能變更必須在相同 dataset 與可比較環境下評估。若 regression
超過已核准 tolerance，必須說明原因及 correctness trade-off。

任何 performance result 都不得省略 correctness checksum；速度較快但 domain
checksum 不同的執行不能與 baseline 視為同一行為。

### NFR-02.6 驗收條件

`NFR-02` 至少必須由下列證據驗證：

- complete source data 的第二次執行不重新下載。
- cache hit 不重新解析全部 source JSON。
- universe 外商品 stream 不開啟。
- 大於 buffer 的資料集以 bounded memory 回播。
- benchmark 記錄 dataset checksum、環境、設定、metrics 及 correctness checksum。
- 建立 threshold 後，使用相同 dataset 的 regression comparison。

## 8. NFR-03：安全與版本

### NFR-03.1 Secret handling

API key 及其他 credential 不得寫入：

- source data 或 replay cache
- manifest 或 run result
- log、warning 或 error
- fixture 或 snapshot
- version control

credential 必須透過不進入持久 artifact 的安全管道提供。第一版可以使用 environment
或其他明確 secret source；實際方式由 operations design 決定。

### NFR-03.2 最小權限與離線邊界

只有需要 Teralion 的 plan／sync 操作可以要求 API key。verify、replay、backtest
及 inspect 本地完整資料時不得要求 credential 或網路權限。

錯誤、debug mode 及 HTTP tracing 仍必須 redaction authorization header、query
secret 及可能的 credential body。

### NFR-03.3 必要版本

至少下列內容必須具有穩定 identity／version：

- source data schema
- source normalizer／market-format mapping
- standard event schema
- deterministic ordering rule
- canonical encoding／checksum
- replay cache format
- fill model
- fee、tax、slippage model
- accounting／marking policy；若其變更會影響結果

strategy implementation 與 instrument reference data 也必須可識別，以完成
`NFR-01` reproducibility identity。

### NFR-03.4 Compatibility behavior

讀取 artifact 或執行 run 時，系統必須判定版本：

- compatible：可以使用。
- rebuildable incompatible：拒絕舊 derived cache，從完整 source data 重建。
- non-rebuildable incompatible：拒絕執行並指示需要重新同步、遷移或更換設定。
- unknown：預設拒絕，不猜測相容。

不相容 event schema、ordering 或 fill model 不得被靜默當成目前版本。

### NFR-03.5 驗收條件

`NFR-03` 至少必須由下列證據驗證：

- repository／fixture secret scan。
- manifest、result、log 及 error redaction tests。
- 無 API key 的 offline backtest test。
- compatible／rebuildable incompatible／unknown version tests。
- cache version 失效只重建 derived artifact 的測試。
- run manifest 完整記錄影響結果的版本 identity。

## 9. 跨需求不變條件

任何 CLI、結果、效能或版本設計都必須維持：

1. 一份設定可建立完整 execution plan。
2. sync 與 backtest 可以分開執行。
3. backtest 預設離線且只讀取 strategy universe。
4. successful、failed 與 degraded 結果清楚區分。
5. 每次結果記錄資料、策略、模型及版本 provenance。
6. 相同 reproducibility identity 產生相同 domain result。
7. 並行與效能最佳化不改變 correctness。
8. benchmark threshold 由實際 2330／TAIFEX dataset 建立。
9. API key 不進入任何持久 artifact、log 或版本控制。
10. 不相容 derived cache 可重建；無法安全相容時拒絕執行。

## 10. 驗證與里程碑摘要

| Requirement | 主要驗證層級 | 首次完整里程碑 |
| --- | --- | --- |
| OPS-01 | CLI／workflow acceptance tests | M2 |
| OPS-02 | manifest schema／result golden tests | M2 |
| NFR-01 | repeated-run／property／system tests | M1 baseline；M2 full backtest |
| NFR-02 | integration tests／benchmarks | M2 TWSE baseline；M3 multi-market |
| NFR-03 | security／compatibility tests | M2 |

正式 requirement、design、implementation 與 test mapping 由
[traceability matrix](../traceability.yaml)維護。

## 11. 待下游文件決定的事項

| 議題 | 文件 |
| --- | --- |
| Command、config schema、exit codes 與 display | [CLI 操作](../operations/cli.md) |
| Local data inspection、repair 與 cache management | [本地資料操作](../operations/local-data.md) |
| Data manifest 與 publish workflow | [data sync 設計](../design/data-sync.md) |
| Run artifact schema 與 replay orchestration | [replay engine 設計](../design/replay-engine.md) |
| Fill／ledger result details | [execution simulation 設計](../design/execution-sim.md) |
| Acceptance workflow | [驗收規格](../verification/acceptance.md) |
| Dataset、metrics、baseline 與 threshold | [效能驗證](../verification/performance.md) |

下游文件不得以 CLI 簡化、效能或相容性為由隱藏資料缺漏、混淆失敗結果、存取
未授權網路、洩漏 secret，或讓相同版本 identity 產生不同 domain result。
