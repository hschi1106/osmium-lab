# ADR-0003：以商品 Session Plan 統一下載、回播與策略啟用

- 狀態：Accepted
- 決策日期：2026-07-30
- 最後修訂：2026-07-30（補充 TAIFEX 實際時段及允許 WarmUp 下單）
- 適用版本：`SessionWindowPolicyV1`、`StrategySessionPolicyV1`
- 主要需求：`DATA-01`、`DATA-05`、`REPLAY-04`、`REPLAY-05`、`STRAT-01`、
  `SIM-01`、`NFR-01`

## 1. Context

`osmium-lab` 必須支援具有不同交易時段的 market 與商品：

- TWSE／TPEx 普通交易具有自己的開盤與收盤時間。
- TAIFEX 商品的日盤時段可能不同於現貨市場。
- 只有部分 TAIFEX 商品具有盤後交易，且盤後時段可能跨越日曆日期。
- 盤後時段的資料仍必須歸屬正確的 exchange `trading_date`。

Teralion Feed Archive 以 `received_at` 篩選歷史 tick，但 replay 只能以
`match_time` 推進。兩個時間可能有延遲或跨越查詢邊界；直接使用官方開盤與收盤
時間查詢，可能漏掉剛好位於邊界附近、但稍晚才被來源接收的資料。

若讓每個 strategy 自行填寫絕對時間，會造成：

- 重複實作 market calendar 與 session 規則。
- strategy 時間與資料下載時間不一致。
- TAIFEX 盤後時段被錯誤切到日曆日期。
- 不同 strategy 對同一商品產生互不相容的本地來源 partition。
- strategy 縮短時間後，MarketState 缺少開盤前 warm-up 或收盤後 final observation。

因此需要一個跨 market 共用的 session model，同時明確分離：

- market／instrument 提供的交易時段事實。
- Teralion `received_at` acquisition window。
- `match_time` replay window。
- strategy 可以交易的 active phase。

## 2. Decision

採用版本化、資料驅動的 `SessionPlan`。所有 market 使用同一套 planning、
acquisition、replay 與 strategy phase 演算法；不同 market／instrument 只提供不同
的 session profile 與 calendar data。

每個被選取的交易 session segment 都必須套用固定五分鐘 margin：

```text
download window = [session open - 5 minutes, session close + 5 minutes)
replay window   = [session open - 5 minutes, session close + 5 minutes)
```

五分鐘 margin 是 `SessionWindowPolicyV1` 的固定 invariant：

- 同時套用於 download 與 replay。
- 不能由 strategy、run parameter 或 market adapter 任意縮短或放大。
- 各 segment 以自己的實際 open／close 計算，不能使用全市場共用的固定時鐘。
- 未來若要改變 margin，必須建立新的 policy version。

download window 以 `received_at` 套用；replay window 以 `match_time` 套用。
`received_at` 不得成為 replay clock。

## 3. Unified session model

### 3.1 Logical types

概念模型：

```text
SessionProfile {
    profile_id
    profile_version
    market
    instrument applicability
    calendar_id
    session segment templates
}

SessionPlan {
    market
    symbol
    trading_date
    timezone
    calendar_version
    profile_version
    materialized segments
}

SessionSegment {
    segment_id
    session_kind
    open_time
    close_time
    download_window
    replay_window
}
```

這是 logical contract，不固定 Rust struct 名稱、datetime library 或 serialization
format。

### 3.2 Session identity

`session_kind` 是語意 identifier，不是 strategy 自行輸入的時鐘。例如：

- `regular`
- `after_hours`

TAIFEX 日盤一律使用 `regular`，夜盤使用 `after_hours`；不另建立 `day` kind。

實際可用集合由 market／instrument session profile 定義。新增 kind 必須有明確的
calendar、trading-date 與驗證來源。

相同 kind 在不同 market 不代表相同 open／close。planner 必須先以
`market + instrument metadata + trading_date + session_kind` 解析 profile，再產生
absolute time windows。

### 3.3 Calendar resolution

Session plan 必須使用具版本的 exchange calendar：

- timezone 明確為 session profile 的 market timezone。
- holiday、補班交易日及非交易日由 calendar 決定。
- TAIFEX 盤後 segment 的 calendar date 由 trading-date 規則解析。
- 「前一日」必須表示前一個適用的 exchange calendar date，不得直接執行
  `trading_date - 24 hours`。
- 無法確認 calendar 或 session profile 時不得把 partition 發布為 complete。

calendar 決定 session 歸屬及邊界，但 event ordering 仍依
[ADR-0001](0001-match-time-ordering.md)使用 `match_time`。

## 4. Five-minute windows

### 4.1 Boundary convention

download 與 replay window 都使用 start-inclusive、end-exclusive：

```text
window_start <= time < window_end
```

對 session open `O`、close `C`：

```text
window_start = O - 5 minutes
window_end   = C + 5 minutes
```

官方 close 時間本身屬於 active phase。`C + 5 minutes` 邊界上的事件屬於下一個
範圍或 outside-window，不屬於本 segment。

若 source endpoint 對 `end` 使用不同 inclusive semantics，Teralion adapter 必須
保留 request provenance，並在本地驗證時依上述 half-open contract 分類；不得因此
縮短五分鐘 margin。

### 4.2 Download window

download window 只用於 Teralion request 與 source completeness：

- query clock 是 `received_at`。
- `start`、`end`、kind filter 及 opaque cursor 必須在每一頁 request 維持相同
  query identity。
- cursor 必須走到 `null`，但 terminal cursor 只證明該 query chain 結束，不單獨
  證明 exchange session 完整。
- raw source 保存 query 回傳且正規化可能需要的 payload。
- API key、request header 或 credential 不得進入 manifest。

同一商品有多個不相鄰 session segments 時，planner 產生多個 download windows。
不能為了方便而下載從第一個 segment 開始到最後一個 segment 結束之間的整段空白。

若兩個加上 margin 後的 download windows 重疊或相接，可以合併為一個實體 request，
但 manifest 必須保留原本 logical segment identities，且不得因此產生重複來源資料。

### 4.3 Replay window

replay window 決定哪些具有有效 `match_time` 的 domain events 進入該 segment：

- replay clock 仍只由 `match_time` 推進。
- `match_time` 位於 replay window 外的 source tick 不進入該 segment timeline。
- 因 `received_at` 與 `match_time` 差異落在 download window 內、replay window 外的
  tick 可以保留在 raw source，但必須列入 outside-window 摘要。
- 多 segment events 依 `match_time` 與 ADR-0001 的 tie-break 串流合併。
- session window 不改變 event atomicity 或 MarketState snapshot semantics。

Replay Engine 不因 clock 穿越 open／close 而製造 `MarketStatus`。session phase 是
execution context；只有來源 format／flags 明確支持的狀態才能正規化為
`MarketStatus` 或 known status。

## 5. Market-specific application

### 5.1 TWSE／TPEx

TWSE／TPEx 普通交易使用 `regular` session profile。其 open／close 由對應 market
calendar 與 interface 文件提供，不由 strategy hardcode。

對目前已確認的普通交易時段，概念結果為：

```text
official session: 09:00–13:30
download window:  08:55–13:35 by received_at
replay window:    08:55–13:35 by match_time
```

第一版不支援的盤中零股、盤後零股、盤後定價與鉅額交易，不會因落在 margin 內就
自動成為支援事件。source format mapping 仍依各 market interface 決定。

### 5.2 TAIFEX

TAIFEX 依 instrument session profile 解析：

- 只有日盤的商品只產生 `regular` segment。
- 具有盤後交易的商品可以產生 `after_hours` 與 `regular` segments。
- 每個 segment 分別依自己的 open／close 加前後五分鐘。
- 跨日盤後資料依 exchange trading-date 規則歸屬，不依午夜切割。

依 2026-07-30 查核的 TAIFEX 官方契約規格與盤後交易商品資料，目前主要 session
profiles 如下；所有時間均為 `Asia/Taipei`：

| 商品 profile | `regular` 官方時段 | `regular` download／replay window | `after_hours` 官方時段 | `after_hours` download／replay window |
| --- | --- | --- | --- | --- |
| 國內股價指數、國外股價指數（TJF 除外）及原油類的盤後適用商品 | 08:45–13:45 | 08:40–13:50 | 15:00–次日 05:00 | 14:55–次日 05:05 |
| 匯率類及黃金類商品 | 08:45–16:15 | 08:40–16:20 | 17:25–次日 05:00 | 17:20–次日 05:05 |
| 日本東證期貨（TJF） | 08:00–16:15 | 07:55–16:20 | 17:25–次日 05:00 | 17:20–次日 05:05 |
| 股票／國內成分 ETF 期貨 | 08:45–13:45 | 08:40–13:50 | 僅適用商品為 17:25–次日 05:00 | 適用時為 17:20–次日 05:05 |
| 國外成分／境外 ETF 期貨 | 08:45–16:15 | 08:40–16:20 | 僅適用商品為 17:25–次日 05:00 | 適用時為 17:20–次日 05:05 |

M3 首個 TAIFEX futures profile 若使用臺股期貨（TX），具體為：

```text
regular official session:     08:45–13:45
regular download/replay:      08:40–13:50
after_hours official session: 15:00–次日 05:00
after_hours download/replay:  14:55–次日 05:05
```

TAIFEX 官方來源：

- [臺股期貨契約規格](https://www.taifex.com.tw/enl/eng2/tX)：`regular`
  08:45–13:45、`after_hours` 15:00–次日 05:00。
- [盤後交易介紹](https://www.taifex.com.tw/cht/4/aHIntroduction)：15:00 與 17:25
  兩組盤後時段、適用商品及次一 `regular` session 歸屬原則。
- [日本東證期貨契約規格](https://www.taifex.com.tw/enl/eng2/tJF)：`regular`
  08:00–16:15、`after_hours` 17:25–次日 05:00。
- [股票／ETF 期貨契約規格](https://www.taifex.com.tw/cht/2/sTF)及
  [適用標的一覽](https://www.taifex.com.tw/cht/2/stockLists)：依 underlying
  決定 13:45／16:15 收盤，且只有公告商品具有 `after_hours`。
- [外匯期貨契約範例](https://www.taifex.com.tw/enl/eng2/xEF?menuid1=12)：`regular`
  08:45–16:15、`after_hours` 17:25–次日 05:00。

上述表格是 profile family，不取代個別契約及 trading-date 規則。到期月份契約最後
交易日可能提早收盤、取消 `after_hours`，部分國外指數或商品契約也有
daylight-saving／到期日例外。planner 必須以
`instrument + contract + trading_date` materialize 當日實際時段，再套用前後五分鐘；
不得只複製一般日的 family table。

不得只用 `market = TAIFEX` 就假設所有 futures／options 具有相同盤後時段，也不得
依 symbol pattern 猜測 session profile。缺少可驗證 metadata 或明確設定時必須停止。

## 6. Strategy contract

### 6.1 Strategy 宣告 session，不宣告時鐘

Strategy 在 execution plan 建立前宣告：

- explicit market／symbol universe。
- 每個商品要參與的 semantic session kinds。

Strategy 不宣告：

- Teralion `start`／`end`。
- `received_at` window。
- 絕對日期時間形式的 replay window。
- 五分鐘 margin。

例如 strategy 可以選擇：

```text
TWSE 2330 -> regular
TAIFEX instrument A -> regular
TAIFEX instrument B -> after_hours + regular
```

Planner 驗證選取的 session kind 確實存在於該商品 profile。unknown 或不適用的
session kind 必須在任何 stream 開啟前失敗。

Strategy 可以在自己的決策邏輯中選擇不於某些 active events 產生輸出，但這不能
縮短 acquisition／replay window，也不能改變其他 strategy 或 run 可重用的 source
partition。

### 6.2 Three strategy phases

每個 materialized segment 具有三個由平台計算的 phase：

| Phase | `match_time` 範圍 | MarketState update | Strategy callback | 新 order intent | 既有 order fill |
| --- | --- | --- | --- | --- | --- |
| `WarmUp` | `[O - 5m, O)` | 是 | 是 | 允許 | 依 fill model 判定 |
| `Active` | `[O, C]` | 是 | 是 | 允許 | 依 fill model 判定 |
| `CoolDown` | `(C, C + 5m)` | 是 | 是 | 不允許 | 不允許 |

exactly at close `C` 的 event 屬於 `Active`。沒有 event 時不產生虛構 callback。

每次 callback 的 read-only context 至少能識別：

- trading date
- instrument
- session kind／segment identity
- current phase
- 是否允許產生新 order intent

WarmUp 用於以已發生資料建立 strategy-local indicator 與 MarketState，也允許
strategy 提前送出 limit order intent，模擬開盤前掛單。intent 是否 accepted、何時
成為 fill-eligible 及是否可使用 WarmUp event 成交，由版本化 fill model 與該 market
的 session rules 判定；允許 intent 不代表可在官方開盤前成交。

CoolDown 用於接收來源確實提供的 final observations、處理既有 order feedback 及
完成 session 摘要，不允許新的 order intent 或 fill。

第一版若需要處理 session close 時尚未完成的 order，其 cancel／carry policy 由
simulation design 明確定義；CoolDown event 不得成為 fill-eligible event，strategy
也不能以擴張自己的 active window 繞過。

### 6.3 Multi-market strategy

多 market strategy 的 planner 為每個 instrument 建立獨立 SessionPlan，再取所有
selected segments 的 event union 依 `match_time` merge。

Strategy context 可以讀取 universe 內各商品截至目前的 session phase 與
MarketState，但不能因某商品為 `Active` 就假設其他商品也已開盤。跨市場同時可交易
條件由 strategy 使用平台提供的 phase context 判斷，不得自行複製 exchange hours。

## 7. Plan、manifest 與 cache identity

Execution／sync plan 至少保存：

- session profile identifier／version
- calendar identifier／version
- selected session kinds
- materialized open／close
- 固定五分鐘 policy version
- 每個 download／replay window

Source manifest 至少保存：

- `received_at` query windows
- 每個 window 的 cursor completion
- observed first／last `received_at`
- outside-window 與 unsupported-format counts

Replay cache identity 至少綁定：

- source checksum
- session profile／calendar version
- `SessionWindowPolicyV1`
- replay windows
- event schema／normalizer／ordering versions

session profile、calendar 或 window policy 不相容時，cache 必須失效並由本地 source
重建；完整且相容的 source 不應因此自動重新下載。

## 8. Completeness and failure

每個 selected segment 必須分別驗證：

- market／instrument／trading date 與 session profile 可解析。
- download／replay windows 正確套用前後五分鐘。
- 所有 opaque cursor 頁面走到終點。
- page query identity 未在 cursor 過程改變。
- payload market、symbol、kind 與 `received_at` 位於 request contract。
- accepted domain events 的 `match_time` 位於 replay window。
- 必要商品 metadata、payload、筆數與 checksum 已保存。

Teralion coverage bucket 有資料或 cursor 為 `null`，都不足以單獨證明完整 session。
若實際資料無法支持某 selected segment 的完整性，partition 必須標示 incomplete，
不得因 strategy 只使用其中一小段就發布為 complete。

## 9. Consequences

### 9.1 正面結果

- 所有 market 使用同一 planning 與 replay phase 模型。
- 不同商品仍保留自己的 session 時段與 trading-date 語意。
- download 與 replay 都具有一致、版本化的五分鐘 boundary margin。
- strategy 不重複實作 exchange calendar 或 Teralion query。
- WarmUp 可以建立狀態及預掛 order；CoolDown 只更新 final state，不接受新 order
  或 fill。
- 完整本地來源資料可供不同 strategy 重用，不因策略時間偏好切成多個版本。
- TAIFEX 盤後與日盤可以在同一 trading date 下正確 merge。

### 9.2 成本與限制

- 必須維護具版本的 market calendar 與 instrument session profile。
- 五分鐘 margin 會讀取部分 strategy 不交易的事件。
- Source 與 cache manifest 必須保存更多 session provenance。
- `received_at`／`match_time` 差異需要 outside-window 分類。
- market-specific format／status 語意仍需各 interface fixture 驗證，不能由通用
  session model 解決。

## 10. Considered alternatives

### 10.1 Strategy 自行指定絕對時間

拒絕。會重複 market calendar、破壞 source reuse，並容易錯切 TAIFEX 跨日時段。

### 10.2 所有 market 使用同一組固定開收盤時間

拒絕。TWSE、TPEx 與 TAIFEX session 不同，且 TAIFEX 商品是否具有盤後交易也不同。

### 10.3 只下載及回播官方 open 到 close

拒絕。`received_at` 與 `match_time` 的邊界差異可能漏資料，也無法提供固定的
WarmUp／CoolDown。

### 10.4 只對 download 加 margin

拒絕。會讓 raw source 與 replay eligibility 使用不同 session scope，增加 manifest
與策略行為的歧義。

### 10.5 將多個 session 取最早到最晚的單一 envelope

拒絕。會下載中間不需要的長時間空白或其他交易 session，尤其不適合 TAIFEX
日盤與盤後。

### 10.6 以 session boundary 產生 synthetic `MarketStatus`

拒絕。calendar 只能證明 planned phase，不能證明交易所實際發布的 market status。

## 11. Verification

至少需要：

- 各 session segment 前後正好五分鐘的 boundary unit tests。
- start-inclusive／end-exclusive edge tests。
- TWSE／TPEx `regular` profile materialization tests。
- TAIFEX `regular`-only 與 `regular`-plus-`after_hours` profile tests。
- TAIFEX 08:45／13:45、08:45／16:15、08:00／16:15 profiles 及各自五分鐘
  window tests。
- TAIFEX 15:00／17:25 `after_hours` profiles 及跨日至 05:00 的五分鐘 window
  tests。
- TAIFEX 到期月份最後交易日縮短或取消 `after_hours` 的 profile test。
- TAIFEX weekend／holiday 前一 exchange date 測試。
- multi-segment window overlap／merge test。
- 每頁 cursor request 維持相同 query identity 的 integration test。
- `received_at` 在 download window、`match_time` 在 replay window 的分類測試。
- WarmUp／Active／CoolDown boundary callback tests。
- WarmUp limit order intent acceptance 及 fill-model eligibility tests。
- CoolDown 新 order intent／fill rejection tests。
- strategy unknown session kind preflight failure test。
- multi-market 不同 phase 的 deterministic replay test。
- session／calendar version 改變造成 cache invalidation 的測試。

M1 以 TWSE 2330 `regular` session 驗證固定 window 與三 phase callback；M3 以實際
TAIFEX futures 驗證日盤、盤後、跨日 trading date 與 multi-market merge。

## 12. Traceability

- [產品需求](../../product-requirements.md)：`DATA-01`、`DATA-05`、`REPLAY-04`、
  `REPLAY-05`、`STRAT-01`、`SIM-01`、`NFR-01`
- [資料需求](../../requirements/data.md)：同步範圍、cursor、TAIFEX trading date
- [回播需求](../../requirements/replay.md)：`match_time`、事件處理、selective streams
- [策略需求](../../requirements/strategy.md)：universe、callback、無前視
- [模擬需求](../../requirements/simulation.md)：後續 eligible event 與 fill boundary
- [系統架構總覽](../overview.md)：planner、replayer、strategy runtime
- [資料與執行流程](../data-flow.md)：sync、replay、strategy callback
- [排序決策](0001-match-time-ordering.md)：跨 segment deterministic ordering
- [市場狀態決策](0002-snapshot-market-state.md)：snapshot reducer 與 read-only state
