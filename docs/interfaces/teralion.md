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

## 5. 試算資料與 endpoint 範圍

Teralion Feed Archive 文件指出，`/ticks` 回傳原始 client-document ticks，包含標示為試算的訊息，並保留 `status_flags` 與來源 `format`；文件舉例 TWSE／TPEx 以 `status_flags & 0x80` 辨識試算，TAIFEX 則以獨立的 `I022`／`I082`／`I064` format 辨識。相對地，文件所述 `/trades`、`/bars`、`/quotes` 為 firm data，不包含其描述的 indicative pre-auction prices。Normalizer 必須使用 `/ticks`，不可用衍生 endpoint 替代原始行情或用來找回被排除的試算。

該頁對 TWSE／TPEx indicative pre-auction 的說明範圍是開盤與收盤試算；它沒有說明盤中 stability trigger／試算期間的 Teralion JSON 欄位映射，也沒有定義該期間 `deal`、`bids`／`asks`、零數量或 `cum_volume` 的語意。故 `status_flags & 0x80` 的公開文件說明可作為 trial 標記依據，但不能單獨證明每筆 stability-period observation 的價量與 book 是模擬競價資料。2026-08-10 以 authenticated、read-only `/ticks` 查詢各觀察到一組 TWSE 3026 與 TPEx 2948 序列：trigger 為非 trial、非零 instant-trend、zero-quantity deal 與空簿；期間 observation 帶 `status_flags=128`、有 deal／book，累計量不變；約兩分鐘後出現非 trial 且累計量增加的正式撮合，接著恢復一般逐筆狀態。這提供 Teralion JSON 的實例證據，但因 raw records 未納入 repository，仍需 checked-in sanitized golden 或外部 user-owned source 驗證程序，不能稱為已完成可重播 fixture。

daily instrument response 保存 symbol、market、kind、underlying、option side、strike、expiry、multiplier、currency、trading date 與 session reference 等可用欄位。`null`、空字串與 `0` 是不同值；缺少 metadata 不得自行推定。

## 6. Cursor 與發布

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

## 7. 時間與完整性

| Clock | 用途 |
| --- | --- |
| `received_at` | archive filter、下載範圍與診斷 |
| `match_time` | 唯一 replay clock |

兩個 clock 不一定有固定先後，也不能以差值推定市場 latency。`match_time` 落在 replay window 外的 raw record仍保留 source，但不進 timeline；缺少或無效 `match_time` 的 timeline format 依 strict policy 拒絕。

coverage、range、非空第一頁、bars、terminal cursor 或觀察到 close 都不能單獨證明 partition 完整。完整性需同時通過 frozen query、closed trading date、cursor chain、instrument metadata、payload、manifest 與 checksum 驗證。

## 8. Source schema 變更

API 未提供可直接替代 normalizer mapping version 的 schema identity。遇到新的 required field、wire type、type/format pair 或 payload shape 時，adapter 保存 raw evidence，normalizer strict reject，直到介面文件、fixture、mapping version 與測試同步更新。

官方參考：[Teralion Feed Archive API](https://docs.teraliontech.com/feed-archive/)。本地生命週期見 [資料流程與儲存](../architecture/data-flow.md)。
