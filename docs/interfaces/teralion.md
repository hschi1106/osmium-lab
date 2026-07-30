# Teralion Feed Archive Interface

## 1. 文件目的

本文件定義 `osmium-lab` 與 Teralion Feed Archive REST API 之間的 adapter contract。
它固定資料同步所需的 endpoint、認證、時間、pagination、response envelope、
完整性判定及安全邊界，但不讓 Teralion wire type 成為 domain event、replay 或
strategy API。

本文件依據：

- [Teralion Feed Archive API](https://docs.teraliontech.com/feed-archive/)。
- 2026-07-27 TWSE `2330` 的實際 API response 與完整 cursor 下載。
- [資料需求](../requirements/data.md)及
  [ADR-0003](../architecture/decisions/0003-session-windows-and-strategy-activation.md)。

適用 interface version：`TeralionFeedArchiveV1`。

## 2. 邊界與責任

Teralion adapter 只負責：

```text
safe request
-> Teralion response validation
-> opaque wire payload
-> source staging
```

adapter 不負責：

- 將 source tick 解讀為 `QuoteSnapshot`、`BookSnapshot` 或 `TradeBatch`；
  mapping 由 market interface 與 normalizer 負責。
- 使用 `received_at` 推進 replay clock。
- 決定 strategy session、fill eligibility 或帳務。
- 將 `bars`、`stats` 或 coverage counts 當成 tick completeness 的替代品。
- 在 replay／backtest 路徑自動存取網路。

Teralion response type 必須位於 source adapter 邊界；domain crate 不得依賴它。

## 3. 連線與認證

### 3.1 Base URL

```text
https://app.teraliontech.com
```

所有本文件使用的資源位於 `/api/feed/`。

### 3.2 Credential

每個 request 使用：

```http
X-API-Key: <credential>
```

credential 由 sync path 的 credential provider 在執行時提供。下列內容一律不得
包含 API key：

- source page 或 instrument payload
- staging／published manifest
- query identity
- checksum input
- log、error、run result 或 test fixture
- version control

錯誤輸出若包含完整 request，必須先移除 header。文件與測試只能使用 placeholder。

### 3.3 Online boundary

只有 `plan` 中需要遠端 discovery 的部分及 `sync` 可以使用 credential 與網路。
`verify`、cache build、replay、backtest 及 inspect 必須能只使用本地資料執行。

## 4. 使用的 endpoints

| Endpoint | 用途 | M2 source contract |
| --- | --- | --- |
| `GET /api/feed/coverage` | 查詢 store-wide `(market, date)` coverage | 必要 discovery |
| `GET /api/feed/range/{symbol}` | 查詢單一 symbol 可用的 `received_at`／date 範圍 | 必要 discovery |
| `GET /api/feed/ticks/{symbol}` | 取得完整 raw client tick document | 必要 payload |
| `GET /api/feed/instruments/{symbol}` | 取得指定 trading date 的商品資料 | 必要 metadata |
| `GET /api/feed/instruments` | 依 market／date 等條件分頁查詢商品 | 後續 universe discovery；explicit symbol 不必使用 |
| `GET /api/feed/bars/{symbol}` | 查詢衍生 OHLCV bars | 只可作診斷，不是完整性證據或 replay source |
| `GET /api/feed/quotes/{symbol}` | 查詢抽出的 book snapshots | 診斷用途；不取代 raw ticks |
| `GET /api/feed/trades/{symbol}` | 查詢抽出的 trade prints | 診斷用途；不取代 raw ticks |
| `GET /api/feed/stats/{symbol}` | 查詢 open-interest／settlement points | 第一版不下載、不正規化、不進 timeline |

第一版以 `/ticks/{symbol}` 作為 market normalizer 的來源，因為它保留 live feed 的
完整 client document envelope 與 market-specific body。`quotes`、`trades`、`bars`
或 `stats` 的衍生 response 不得與 raw tick 混合後宣稱是同一份完整來源。

## 5. Request contract

### 5.1 Coverage

```http
GET /api/feed/coverage?start=YYYY-MM-DD&end=YYYY-MM-DD
```

`start`、`end` 是 date。response 是整個 window 的單頁結果，
`next_cursor` 固定為 `null`。

coverage bucket 只表示該 `(market, date)` 至少有一筆 archived tick，以及服務端
觀察到的 symbol／tick counts。它不能證明指定 symbol、session window 或 cursor
chain 已完整。

### 5.2 Symbol range

```http
GET /api/feed/range/{symbol}
```

adapter 必須保留 symbol 的原始 domain identity，不得以 display name、root 或
filesystem-normalized value 代替。`available=false` 時所有 bounds 應為 `null`；
若 response 違反此組合，視為 schema error。

### 5.3 Ticks

第一頁：

```http
GET /api/feed/ticks/{symbol}?start=<ISO-8601>&end=<ISO-8601>&kinds=<csv>&limit=5000
```

後續頁：

```http
GET /api/feed/ticks/{symbol}?start=<same>&end=<same>&kinds=<same>&limit=5000&cursor=<opaque>
```

規則：

- `start`／`end` 使用含 offset 的 ISO-8601 timestamp。
- `kinds` 是 comma-separated source kind；M2 TWSE 使用 `quote`。
- `limit` 最大為 `5000`；page size 不是完整性條件。
- 每頁都重送同一組 frozen `symbol`、`start`、`end`、`kinds` 及 `limit`，只新增
  前一頁原樣回傳的 `cursor`。
- cursor 是 opaque value，不 decode、不修改、不自行產生，也不作 replay
  ordering key。
- server 對 `end` 的 inclusive semantics 不作本地假設；source 保存遠端結果，
  本地再依 ADR-0003 的 half-open window 驗證與分類。

若 `kinds` 無法排除同 kind 內不支援的 market format，例如 TWSE 盤中零股，
adapter 仍保存 raw response；normalizer 依 market interface 明確略過或拒絕。

### 5.4 Daily instrument

```http
GET /api/feed/instruments/{symbol}?date=YYYY-MM-DD
```

`date` 必須是 execution plan 的 exchange `trading_date`，不能以下載日或 process
local date 代替。response 是 bare `FeedInstrument`，不是 paged envelope。

若使用 collection endpoint：

```http
GET /api/feed/instruments?market=<slug>&date=YYYY-MM-DD&limit=5000
```

後續頁必須依 ticks 相同的 opaque cursor 規則走到 `null`。第一版 explicit symbol
universe 優先使用 single-symbol endpoint，避免下載無關商品。

### 5.5 Bars diagnostic

```http
GET /api/feed/bars/{symbol}?start=<ISO-8601>&end=<ISO-8601>&interval=1m
```

bars 由服務端即時計算，時間欄位 `t` 是 UTC bucket start。它只能用於人工檢查
範圍與活躍區段；不能用 bars 數量、第一根／最後一根 bar 或成交量推斷 raw ticks
沒有缺頁。

## 6. Response contract

### 6.1 Paged envelope

`ticks`、`quotes`、`trades`、`stats` 及 collection `instruments` 使用：

```json
{
  "items": [],
  "next_cursor": "opaque-or-null"
}
```

`items` 必須是 array。`next_cursor` 必須是 string 或 `null`。額外 top-level 欄位
在更新 interface version 前只能保存為 unknown，不得改變既有語意。

實際 2330 ticks response 只觀察到 `items` 與 `next_cursor`；symbol 不在 page
envelope 中，必須由 frozen request identity 與每筆 tick 共同驗證。

### 6.2 Tick envelope

`FeedTickPage.items[]` 是 opaque JSON object。所有已支援 tick 至少驗證：

| Field | Wire type | Adapter rule |
| --- | --- | --- |
| `type` | string | source kind；必須符合 request 及 market interface |
| `market` | string | 必須符合 planned market |
| `format` | string | 交由 `market + format` registry 選擇 normalizer |
| `symbol` | string | 必須等於 request symbol |
| `match_time` | string | 保存；由 normalizer 驗證後才可進 timeline |
| `received_at` | string | capture clock；用於 query／source validation |

kind-specific fields 保持 flattened top-level object，不由 generic adapter 改名、
補值或刪除。未知 field 必須保留在 raw source。

### 6.3 Coverage response

```json
{
  "items": [
    {
      "market": "twse",
      "date": "2026-07-27",
      "symbols": 31201,
      "ticks": 116944519
    }
  ],
  "next_cursor": null
}
```

`symbols`／`ticks` 是 discovery counts，不是指定 partition 的 expected count。
相鄰交易日出現異常低值可以觸發 warning，但不得以經驗門檻自動宣告完整。

### 6.4 Range response

```json
{
  "symbol": "2330",
  "available": true,
  "first_received_at": "2026-06-16T10:26:01.380645+08:00",
  "last_received_at": "2026-07-29T09:18:10.764437+08:00",
  "first_date": "2026-06-16",
  "last_date": "2026-07-29"
}
```

range 是低成本 availability probe。requested trading date 落在 date bounds 內，
仍不表示該日或指定 session window 完整。

### 6.5 Instrument response

第一版接受並保存：

| Field | Wire type | Domain use |
| --- | --- | --- |
| `symbol` | string | 必要 identity |
| `market` | string | 必要 identity |
| `exchange` | string／null | display metadata |
| `name` | string | display metadata；不作 identity |
| `root` | string | 原樣保存；空字串不是自行推論的 root |
| `kind` | string | 原樣保存；空字串保持 unknown／blank |
| `underlying` | string／null | 可選 metadata |
| `call_put` | string／null | 可選 metadata |
| `strike` | number／null | 可選 metadata |
| `expiry` | string／null | 可選 metadata |
| `multiplier` | number／null | 可選；缺少時不得猜測 |
| `currency` | string／null | 可選；缺少時不得猜測 |
| `trading_date` | string／null | 必須與 requested date 相容 |
| `session.reference` | number／null | 當日 reference price |
| `session.rise_limit` | number[] | 當日漲幅限制 tiers |
| `session.fall_limit` | number[] | 當日跌幅限制 tiers |

`0`、空字串與 `null` 是不同 wire values。normalizer 不得把 `strike: 0`、空
`kind` 或空 `root` 自行改成已知商品語意。

## 7. Cursor state machine

每個 paged request 使用下列 state machine：

```text
Start(frozen query)
-> receive page
-> validate envelope and every item
-> persist page bytes + safe metadata
-> next_cursor is string:
     verify non-empty and unseen
     request next page with exact opaque cursor
-> next_cursor is null:
     mark cursor_complete
```

必須拒絕：

- `next_cursor` type 不合法。
- non-terminal cursor 為空字串。
- cursor 重複、循環或無法前進。
- 後續頁的 frozen query identity 改變。
- page payload 無法保存或 checksum 失敗。
- HTTP error、timeout 或 parse error 被當成空 terminal page。

cursor 全值只可出現在需要恢復的 staging checkpoint；log 與 user-facing result
使用 redacted value 或 digest。published manifest 至少記錄 page count、
terminal cursor evidence、query identity 與每頁／整體 checksum。

## 8. 時間語意

### 8.1 Two clocks

| Clock | Owner | 用途 |
| --- | --- | --- |
| `received_at` | Teralion capture | archive filter、下載範圍、source diagnostics |
| `match_time` | exchange event | 唯一 replay time |

兩者使用含 `+08:00` offset 的 ISO-8601 timestamp，並可包含 microseconds。兩個
clock 由不同來源校時，差值不得當成可靠 market latency，也不得假設
`received_at >= match_time`。

### 8.2 Session window

adapter 接收 planner 已 materialize 的 download window：

```text
[session open - 5m, session close + 5m)
```

request 以 `received_at` 篩選。raw source 完成後：

- `received_at` 位於 request window 外：schema／service contract error。
- `match_time` 位於 replay window 內：可以進入 normalizer。
- `match_time` 位於 replay window 外：保留 raw，列入 `outside_replay_window`，
  不進 timeline。
- 缺少或無效 `match_time`：保留診斷資料，但依 `REPLAY-06` 拒絕正常 replay。

## 9. 完整性與錯誤

### 9.1 HTTP result

依官方 endpoint contract：

- `200`：仍須驗證 body、cursor 與 local invariants。
- `400`：request window、cursor、date、interval 或 kind 不合法；不得 retry 成
  成功空資料。
- `404`：symbol 或 window 無資料；不得直接標成 verified zero ticks。
- transport／service failure：維持 building／incomplete，由 retry policy 處理。

只有 coverage、range、query response、closed trading date、terminal cursor、
instrument metadata、payload validation 及 checksum 共同成立，partition 才可
發布為 complete。

### 9.2 Coverage is not completeness

下列任一項單獨都不足以證明完整：

- coverage 有 `(market, date)` bucket。
- range 包含 requested date。
- 第一頁非空。
- 某一分鐘有 bar。
- 最後一頁 `next_cursor=null`。
- 觀察到官方 close 的 `match_time`。

terminal cursor 只證明同一 frozen query 的 server pagination chain 已結束；
exchange session 是否完整仍需本地 manifest 與 market-specific invariants 判定。

### 9.3 Source schema change

Teralion tick body 是 opaque JSON，API response 未提供可直接當成 normalizer
compatibility 的 schema version。published source 因此必須記錄：

- `TeralionFeedArchiveV1`
- endpoint 與不含 credential 的 frozen query
- observed market／type／format field sets
- download time
- page／item counts
- source checksum algorithm 與 values

新增或改變 required field、wire type 或 format 時，normalizer registry 必須拒絕
未知 shape，直到 market interface、fixture 與 mapping version 更新。

## 10. 2026-07-27 TWSE 2330 實測證據

本地 acquisition 位於：

```text
raw/teralion/twse/2026-07-27/2330/complete
```

這份完整 raw download 是本地證據，不是已提交的 test fixture，也不構成資料授權
允許進入版本控制的證明。

| 項目 | 實測值 |
| --- | --- |
| Endpoint | `/api/feed/ticks/{symbol}` |
| Query clock | `received_at` |
| Window | `[2026-07-27 08:55, 13:35)` Asia/Taipei |
| Kinds | `quote` |
| Limit | `5000` |
| Pages | `16` |
| Terminal cursor pages | `1` |
| Total ticks | `77,213` |
| First／last `received_at` | `08:55:01.069708`／`13:33:11.007003` |
| First／last `match_time` | `08:54:56.982904`／`13:30:00` |
| Missing `match_time` | `0` |
| Observed formats | `STOCK_REALTIME` 70,199；`STOCK_SNAPSHOT` 3,597；`INTRADAY_ODDLOT_REALTIME` 3,417 |
| Page checksums | 16 個 SHA-256 均已保存並通過本地檢查 |

第一筆 `match_time` 早於 08:55，但其 `received_at` 位於 download window 內。這正是
download clock 與 replay clock 必須分離的實例：raw tick 保留，regular session
replay 依 half-open window 將它列為 outside-window。

此證據驗證 endpoint shape 與 pagination 行為；在 API integration tests、合法
fixture、atomic publish 及 second-run reuse 尚未實作前，不得將 `DATA-01` 或
`DATA-02` 的 verification lifecycle 標示為 complete。

## 11. 測試 contract

至少需要：

- coverage 及 range response schema tests。
- ticks 第一頁、multi-page 及 terminal cursor tests。
- cursor opaque round-trip、重複 cursor 及 malformed cursor negative tests。
- frozen query 在所有頁一致的 test。
- `400`／`404`／timeout 不被視為合法空頁的 tests。
- credential redaction test。
- tick envelope identity mismatch tests。
- download `received_at` 與 replay `match_time` window boundary tests。
- instrument null／empty／zero preservation tests。
- 16-page 2330 API integration test；live test 與 committed fixture test 分離。
- checksum verification、interruption recovery 及 second-run no-download tests。

## 12. Traceability

- `DATA-01`：coverage、range、ticks、instrument、cursor。
- `DATA-02`：safe query identity、source payload、checksum 與 offline boundary。
- `DATA-03`：完整性、HTTP failure 與 cursor terminal evidence。
- `DATA-05`：daily instrument response 與 missing metadata。
- `REPLAY-01`：wire／domain boundary。
- `REPLAY-06`：invalid time、identity、format 與 schema error。
- `NFR-03`：credential isolation 與 interface version。
