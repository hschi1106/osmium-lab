# Teralion Feed Archive 介面

本文件定義 Teralion REST API 與本地 source repository 之間的 adapter contract。wire response 只在 adapter／normalizer 邊界使用，不會成為 replay 或 strategy API。

## 1. 認證與網路

Base URL 為 `https://app.teraliontech.com`，資源位於 `/api/feed/`。request 使用：

```http
X-API-Key: <TERALION_API_KEY>
```

credential 只由 `data sync` 的 runtime provider 提供，不得寫入設定、query identity、source、manifest、cache、log、fixture 或 run artifacts。錯誤訊息中的 request header 必須先移除或遮蔽。

## 2. 使用的 endpoints

| Endpoint | 用途 |
| --- | --- |
| `GET /api/feed/coverage` | 查詢 market/date coverage，作為 discovery |
| `GET /api/feed/range/{symbol}` | 查詢 symbol 的可用時間範圍 |
| `GET /api/feed/ticks/{symbol}` | 取得 normalizer 使用的 raw tick document |
| `GET /api/feed/instruments/{symbol}` | 取得指定 trading date 的 instrument metadata |

其他衍生 endpoint 不在 sync transport contract 內，也不能取代 `/ticks` 或作為完整性證據。

## 3. Ticks query

第一頁：

```http
GET /api/feed/ticks/{symbol}?start=<ISO-8601>&end=<ISO-8601>&kinds=<csv>&limit=5000
```

後續頁保留完全相同的 symbol、start、end、kinds 與 limit，只加入前一頁回傳的 opaque `cursor`。cursor 不 decode、不修改、不自行產生，也不參與 replay ordering。

`start`／`end` 是 planner 依 session 產生的 `received_at` download window。server 的邊界行為不直接成為 replay 規則；source 保存回傳資料，normalizer 再以 `match_time` replay windows 分類。

## 4. Response envelope

分頁 response：

```json
{
  "items": [],
  "next_cursor": "opaque-or-null"
}
```

cache prepare 讀取 tick 時，normalizer 至少驗證：

| Field | 規則 |
| --- | --- |
| `type` | 符合 request kind 與 market interface |
| `market` | 符合 planned archive market |
| `format` | 由 market/profile registry 選擇 normalizer |
| `symbol` | 等於 frozen partition symbol |
| `match_time` | 保存；通過 normalizer 驗證後才可進 timeline |
| `received_at` | archive query 與 source diagnostics only |

sync 會保存 raw page 並驗證 page envelope、JSON、record count 與 checksums；market-specific identity、欄位及語意在 cache prepare 階段驗證。unknown field 不由 generic adapter 改名、補值或刪除。

daily instrument response 保存 symbol、market、kind、underlying、option side、strike、expiry、multiplier、currency、trading date 與 session reference 等可用欄位。`null`、空字串與 `0` 是不同值；缺少 metadata 不得自行推定。

## 5. Cursor 與發布

```text
Start(frozen query)
  -> receive and validate page
  -> persist compressed bytes + checksums
  -> checkpoint cursor
  -> next_cursor string: request next page
  -> next_cursor null: verify and publish
```

下列狀況拒絕發布：

- envelope 或 item schema 不合法。
- cursor 為空、重複、循環或無法前進。
- pagination 中 frozen query identity 改變。
- page 寫入、compression 或 checksum 失敗。
- HTTP error、timeout 或 parse error。
- terminal cursor、instrument metadata 或 source verification 未完成。

published manifest 記錄 interface version、endpoint、安全 query identity、page/item counts、observed type/format、checksums 與 terminal evidence。

## 6. 時間與完整性

| Clock | 用途 |
| --- | --- |
| `received_at` | archive filter、下載範圍與診斷 |
| `match_time` | 唯一 replay clock |

兩個 clock 不一定有固定先後，也不能以差值推定市場 latency。`match_time` 落在 replay window 外的 raw record仍保留 source，但不進 timeline；缺少或無效 `match_time` 的 timeline format 依 strict policy 拒絕。

coverage、range、非空第一頁、bars、terminal cursor 或觀察到 close 都不能單獨證明 partition 完整。完整性需同時通過 frozen query、closed trading date、cursor chain、instrument metadata、payload、manifest 與 checksum 驗證。

## 7. Source schema 變更

API 未提供可直接替代 normalizer mapping version 的 schema identity。遇到新的 required field、wire type、type/format pair 或 payload shape 時，adapter 保存 raw evidence，normalizer strict reject，直到介面文件、fixture、mapping version 與測試同步更新。

官方參考：[Teralion Feed Archive API](https://docs.teraliontech.com/feed-archive/)。本地生命週期見 [資料流程與儲存](../architecture/data-flow.md)。
