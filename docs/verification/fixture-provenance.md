# M1 Fixture Provenance 與 Redistribution Gate

## 1. 文件目的

本文件記錄 M1 TWSE 2330 fixture 的來源、候選 records、完整性證據、預定最小化
方式與 repository commit gate。它不包含行情 payload，也不構成法律意見或
授權。

```text
provenance_record_version = 1
scope                     = M1 / TWSE / 2330
redistribution_status     = approved
```

本 approval 只涵蓋 private repository `hschi1106/osmium-lab` 的 internal use；
repository visibility 已於 2026-07-30 透過 GitHub repository metadata 驗證為
`private`。若 repository visibility、存取政策或使用範圍改變，必須在散布 fixture
前重新 review。

## 2. Source acquisition

| 欄位 | 值 |
| --- | --- |
| Provider | Teralion |
| Product／source label | `teralion-feed-archive` |
| Endpoint shape | `/api/feed/ticks/{symbol}` |
| Market | `twse` |
| Symbol | `2330` |
| Trading date | `2026-07-27` |
| Filter clock | `received_at` |
| Requested window | `[2026-07-27T08:55:00+08:00, 2026-07-27T13:35:00+08:00)` |
| Requested kind | `quote` |
| Pagination | 16 pages，terminal cursor reached |
| Local source path | `raw/teralion/twse/2026-07-27/2330/complete/`（gitignored） |
| Acquisition metadata SHA-256 | `563e727bbd20c9b5759c8504f0aad40f195895ced109d1e50be59cf57ecc9fc0` |

local acquisition 的觀察摘要：

- 77,213 ticks。
- `STOCK_SNAPSHOT` 3,597 筆。
- `STOCK_REALTIME` 70,199 筆。
- `INTRADAY_ODDLOT_REALTIME` 3,417 筆。
- observed `match_time`：
  `2026-07-27T08:54:56.982904+08:00` 至
  `2026-07-27T13:30:00+08:00`。
- observed regular stock `status_flags`：`4`、`8`、`16`、`128`。
- observed `limit_flags`：`0`。

完整 acquisition metadata 與 page files 留在 local gitignored source；不得因
本文件只列摘要就刪除原始 provenance。

## 3. Source integrity

候選 records 只來自：

| Page | SHA-256 |
| --- | --- |
| `pages/0001.json` | `a3569853550e42fcc5e4d54b7610b316d77ec039511da05eefd69c4eb84e3fe5` |
| `pages/0016.json` | `a9e1c4feb557e2fe0f62cbd2fec0380325470846b5feb9c61e804d32b19589b7` |

完整 16 頁 checksum manifest 位於 local：

```text
raw/teralion/twse/2026-07-27/2330/complete/checksums.sha256
```

fixture extraction 前必須先驗證整份 manifest；任一 source page checksum 不符時
停止，不可從已改變的資料產生同一 fixture identity。

## 4. Candidate record selectors

selector 使用 page 內 `items` array 的 zero-based `item_index`。下表只列 identity
與測試角色，不複製 price、quantity、book 或 deal payload。

| Page | item_index | `match_time` | `status_flags` | 角色 |
| --- | ---: | --- | ---: | --- |
| `0001.json` | 179 | `2026-07-27T08:59:57.927985+08:00` | 128 | pre-open trial，同時間第一 occurrence |
| `0001.json` | 180 | `2026-07-27T08:59:57.927985+08:00` | 128 | pre-open trial，同時間第二 occurrence |
| `0001.json` | 183 | `2026-07-27T09:00:07.360140+08:00` | 8 | opening marker |
| `0001.json` | 329 | `2026-07-27T09:00:10.962752+08:00` | 16 | continuous snapshot，cumulative volume 基準 |
| `0001.json` | 416 | `2026-07-27T09:00:15.931117+08:00` | 16 | continuous snapshot，book 與 cumulative volume 變化 |
| `0016.json` | 2201 | `2026-07-27T13:29:58.214261+08:00` | 128 | pre-close trial，同時間第一 occurrence |
| `0016.json` | 2202 | `2026-07-27T13:29:58.214261+08:00` | 128 | pre-close trial，同時間第二 occurrence |
| `0016.json` | 2204 | `2026-07-27T13:30:00+08:00` | 4 | closing marker |

這 8 筆候選涵蓋：

- 2 個以上 match times。
- pre-open／pre-close trial。
- opening／closing marker。
- continuous matching。
- book 與 cumulative volume 變化。
- real same-match-time occurrences。
- `STOCK_SNAPSHOT` 提供的 deal observation。

最終 fixture 可再縮小，但若移除任何 coverage，必須在 M1 acceptance 說明替代
record。不得為了湊 coverage 修改 source value；缺少的 negative branch 使用
明示的 `derived_negative` 或 `synthetic_domain`。

## 5. Fixture artifact

已建立：

```text
spec/fixtures/teralion/twse/2330/2026-07-27/stock-snapshot.jsonl
spec/fixtures/teralion/twse/2330/2026-07-27/metadata.yaml
```

`metadata.yaml` 至少包含：

```text
fixture_schema_version
provider
source_product
market
symbol
trading_date
source_acquisition_checksum
source_page_checksums
record selectors
record count
extraction tool identity
extraction command／algorithm version
removed fields
redacted fields
fixture byte SHA-256
redistribution approval reference
approved by／at
secret scan result
```

fixture content：

```text
record_count = 8
sha256      = ff1474c9a77223c42d416facb04c070aec5af6f166a68e1cee237616c55ec84c
```

第二次 extraction 與 committed candidate byte-for-byte 相同。

## 6. Extraction policy

approval 後 extraction 必須：

1. 驗證 `checksums.sha256`。
2. 只選上表明示 selectors。
3. 保留每個 selected `items[]` object 的 source field names 與 exact JSON
   values。
4. 只移除 response envelope、pagination cursor 與 request transport metadata。
5. 不以 `f64` parse／rewrite numeric lexeme。
6. 不改寫 `match_time`、`received_at`、format、flags、deal、book 或 cumulative
   volume。
7. 以 selector order 寫出 JSONL；ordering tests 可在 memory 另行 shuffle。
8. 產生 exact fixture SHA-256 與 record count。
9. 執行 secret scan。

若 source JSON library 無法保證 numeric lexeme fidelity，extraction 必須採用
保留原始 record bytes 的 parser 或先固定一個經 review 的 canonicalization
version；不得無聲改變數值表示。

## 7. Removed、redacted 與 forbidden content

目前預定：

| 類別 | Policy |
| --- | --- |
| API key／authorization header／cookie | forbidden；不得出現在 source fixture 或 metadata |
| request headers | 移除 |
| cursor／pagination envelope | 移除 |
| unrelated market／symbol／format records | 移除 |
| selected record market payload | 原樣保留，approval 後才可 commit |
| `received_at` | 保留於 source fixture 作 provenance；normalizer 不得當 replay time |
| personal data | acquisition 未預期含有；secret scan 與人工 review 仍必須執行 |

若需要 redaction selected record 的任何 market field，該 record 不再算
`approved_source`，必須重新評估 acceptance suitability。

## 8. Redistribution review

### 8.1 已完成的公開資料檢查

2026-07-30 檢查：

- TeralionTech 公開公司／產品網站。
- repository 現有 Teralion Feed／Feed Archive interface references。
- 可公開搜尋到的 Teralion terms／license／redistribution 資訊。

找到的官方公開頁面只有公司與產品資訊及一般 copyright notice；沒有找到明確
允許將 Feed Archive 行情 payload 放進 public version control 或再散布的條款。
「未找到允許」不等於判定使用一定被禁止；它只代表 repository 沒有足以關閉
gate 的 permission evidence。

公開參考：

- [TeralionTech official site](https://teraliontech.com/index-en.html)

### 8.2 Required approval

只有有權代表資料訂閱者／契約持有人判斷的人，可以填寫：

```text
redistribution_status: approved
approval_scope: exact planned fixture path and selected records
approval_basis: contract clause / written provider permission / applicable license
approval_reference: non-secret stable reference
approved_by: name or accountable role
approved_at: ISO-8601 timestamp
expiry_or_review_date: timestamp or null
```

不能使用：

- 「資料可以透過 API 下載」作為 redistribution permission。
- assistant、developer 或 test runner 的推測。
- 沒有可回溯 reference 的口頭「應該可以」。
- 將 repository 設為 private 的假設，除非 approval scope 明確只涵蓋該 private
  repository 且 access policy 已記錄。

### 8.3 Current gate

```text
gate_id: TERALION_FIXTURE_REDISTRIBUTION_APPROVAL
status: closed
blocks: []
does_not_block:
  - domain type implementation
  - normalizer implementation against local/private source
  - synthetic reducer/order/strategy tests
closure_evidence:
  - explicit approval fields recorded
  - repository visibility verified private
  - source checksum manifest verified
  - extracted fixture checksum recorded
  - deterministic re-extraction passed
  - fixture secret scan passed
```

## 9. Approval record

目前 record：

```yaml
redistribution_status: approved
approval_scope: selected M1 fixture records committed only to private repository hschi1106/osmium-lab for internal use
approval_basis: permitted internal/private-repository use
approval_reference: repository-owner authorization recorded in this approval record on 2026-07-30
approved_by: repository owner/admin
approved_at: 2026-07-30T09:12:56Z
expiry_or_review_date: null
fixture_content_sha256: ff1474c9a77223c42d416facb04c070aec5af6f166a68e1cee237616c55ec84c
secret_scan_status: passed_field_allowlist_and_forbidden_pattern_scan
```

本 approval 不授權公開散布，也不因 repository 目前為 private 而自動延伸至 fork、
artifact、package、release 或其他 export。fixture extraction 與 metadata 以後續
獨立 commit 記錄；normalizer 行為變更不得混入。
