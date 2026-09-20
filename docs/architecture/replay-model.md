# 回播模型

## 1. Domain event

`market-types` 提供與來源格式分離的 exact types。`DomainEvent` 包含：

```text
instrument
trading_date
source_format
match_time
source_sequence?
payload
```

payload 支援 `QuoteSnapshot`、`BookSnapshot`、`TradeBatch`、`MarketStatus` 與單一 `IndicativeAuction`。
auction 以 provider-neutral `AuctionObservation` 表達 `AuctionPurpose::{Opening, Closing,
Periodic, VolatilityInterruption}` 的 partial evidence；purpose、`delayed`、`disposal` 與
volatility direction 各自可為 `Known`、`NoObservation` 或 `Unknown`。同一輪中
`NoObservation` 可沿用前態，`Unknown` 則使該欄位失效；兩者都不以零值代替。

每個 event 另可攜帶 `MarketSignal::{Continuous, AuctionCollecting, AuctionUncross, Closed}`。
`AuctionCollecting` 與 `AuctionUncross` 共用同一份 `AuctionObservation`；signal 是市場語義，
不把 provider status bits 暴露給 replay、strategy 或 simulation。

同一 source record 的不可分割成交、book 與 annotations 形成單一 atomic event。auction event 是試算觀察，不是實際成交。

## 2. 排序與時間

`match_time` 是 replay clock 與第一排序鍵。ordering rule version 4 的內容鍵依序為：

```text
match_time
market_rank
symbol
source_format
source_phase_rank
event_kind_rank
source_sequence
event_fingerprint
```

TWSE／TPEx `STOCK_REALTIME` 的 intermediate trade 使用較早的 `source_phase_rank`，確保同一撮合時間的 `TradeBatch` 先於 final `QuoteSnapshot`。其餘 tie-break 只保證 deterministic order，不代表交易所全域封包順序。

stream 必須符合相同 schema、ordering 與 canonical version；單一 stream 時間不得倒退。multi-stream merge 只保留每條 stream 的 head，因此記憶體用量與 stream 數量相關，而非事件總量。

## 3. Session plan

planner 依 instrument profile、exchange trading date 與 strategy 選擇的 `SessionKind` 產生 segment。每個官方 session 另加入前後五分鐘 margin：

- download window 以 `received_at` 查詢 source。
- replay window 以 `match_time` 接受 event。
- `WarmUp` 位於 open 前 margin。
- `Active` 覆蓋官方 session。
- `CoolDown` 位於 close 後 margin。

TAIFEX after-hours segment 可以跨日，但仍歸屬 planner 指定的 trading date。index futures／options 於 15:00 開盤，stock futures 於 17:25 開盤，皆於次一交易日 05:00 收盤。多個不連續 segment 不會用空檔資料補齊。phase 與 boundary 是 execution context，不會合成 `DomainEvent`。

內建 profile：

| Profile | Session |
| --- | --- |
| `twse_regular` | 09:00–13:30 |
| `tpex_regular` | 09:00–13:30 |
| `taifex_index_futures` | after-hours + regular |
| `taifex_stock_futures` | after-hours + regular |
| `taifex_stock_futures_regular_only` | regular |
| `taifex_index_options` | after-hours + regular |

實際時間由 `run-planner` 的版本化 profile 固定；設定只選 profile 與 semantic session kinds。

## 4. MarketState

每個 instrument 有獨立的 `MarketState`：

- 完整 book snapshot。
- 最近 trade／batch observation。
- cumulative volume。
- 最新 indicative auction observation；與 firm book／trade／volume 分開保存。
- 最新 `MarketSignal` 與由 reducer 唯一推導的 `MarketPhase::{Continuous, Auction, Closed}`。
- `last_match_time`、state version 與 applied event reference。

每側 book 將最多五筆 displayed entries 分成可定價的 `BookLevel` 與可選的
`market_order_quantity`。後者保存交易所用零 wire price 表達的市價委託聚合量，不是價格；
best bid／ask、mark、slippage 與 execution depth 只讀取 priced levels。

`Observation<T>` 的更新規則：

- `NoObservation`：保留既有 field。
- `Set(value)`：以目前 event 與 value 取代。
- `Clear`：來源明確移除可用值，state 記錄 unavailable。
- `Unknown(raw)`：來源明確使既有 knowledge 失效並保存 raw reason，不推定 value。

因此 `NoObservation != Unknown != Known(false)`：沒有新觀察、知道舊知識已失效、以及明確觀察到
false 是三種不同事實。

`QuoteSnapshot` 更新完整 firm book，並依 observation 更新 firm trade 與 volume；`BookSnapshot`
取代 firm book。`TradeBatch` 更新最近正式成交與可用 cumulative volume，不修改 book。
`MarketStatus` 只更新實際攜帶的 cumulative volume 與 signal，不宣稱帶有完整 book 或正式
成交，因此保留既有 firm book／trade。`IndicativeAuction` 只更新獨立的試算欄位與
`AuctionCollecting` signal，不建立實際成交，也不覆寫 firm book／trade／volume。

### 4.1 Auction evidence 與 round lifecycle

`AuctionObservation` 的 `purpose`、`delayed`、`disposal`、`direction` 各自使用
`AuctionEvidence<T>`：

| Same-round input | Resolved evidence |
| --- | --- |
| `NoObservation` | retain previous field |
| `Known(value)` | replace/reassert field |
| `Unknown` | invalidate previous knowledge |

`AuctionCollecting` 更新同一輪 partial evidence。Repeated `delayed=Known(true)` 只是 reassertion，
不建立 round counter。Unclassified trial 不會被時間或頻率猜成 `Periodic`；purpose unknown 時，
direction 等獨立 evidence 仍可保留。

`AuctionUncross` 同時是「本次 round result event」與 lifecycle boundary。Reducer 必須先 resolve 本輪
evidence，讓本 event 的 matching/context 看見正確 call-auction result，再建立 post-event state。兩者
不能混為同一物件：

```text
round N state:
  purpose  = Periodic
  delayed  = true
  disposal = true
        ↓ AuctionUncross
current result event / matching context:
  purpose  = Periodic
  delayed  = true          # 本輪已 resolved 的結果
  matching = CallAuction
        ↓ reducer post-state
round N+1 state:
  purpose  = Periodic
  delayed  = false         # 新一輪預設尚未 delayed
  disposal = true          # 有效 instrument/day evidence 可延續
        ↓ explicit delay trigger
round N+1 state:
  delayed  = true
```

新一輪的 `delayed` 是 `Known(false)`，不是 `NoObservation`；上一輪 direction 等 transient evidence
不會無條件帶入。其他已知 purpose 的 boundary：

| Uncross purpose | Current event matching | Post-event `MarketPhase` |
| --- | --- | --- |
| `Opening` | `CallAuction` | `Continuous` |
| `Closing` | `CallAuction` | `Closed` |
| `Periodic` | `CallAuction` | next `Auction(Periodic, delayed=false)` |
| `VolatilityInterruption` | `CallAuction` | `Continuous` |
| unknown / no purpose evidence | auction event，但不可推定後態 | `Unknown` |

`Continuous` 結束既有 auction context；`Closed` 保留最後 firm observation。Session boundary 可依
`Carry` 或 `ResetObservableFields` 清除 observable/auction context；目前 CLI runner 在每個 planned
segment 使用 `ResetObservableFields`。Top-level market signal 的 `NoObservation` 保留前態，
`Unknown` 不推測為 continuous。

reducer 先驗證整個 transition，再一次提交。非法價格、數量單位、時間倒退或 cumulative-volume policy 違反時，state 不變。strategy 取得的 `MarketStateView` 沒有 mutation API。

reducer 支援 carry 與 reset boundary policy；目前 CLI runner 對每個 planned segment 使用 `ResetObservableFields`，在下一 segment 首個 event 前重設 observable fields。

### 4.2 Market background contract

回播不建立額外的 `MarketBackground` 或 instrument-day profile。各欄位的 production source 如下：

| 欄位 | production source | 缺少時的行為 | 是否進 run identity |
| --- | --- | --- | --- |
| intraday matching | 當前 `DomainEvent` 的 `MarketSignal`，再由 reducer 產生 `MarketPhase` | `NoObservation` 保留既有 state；初始或 `Unknown` 時 `TradingContext` 回傳 `MatchingState::Unknown`，不猜 `Continuous` | event/cache identity；不另存背景 |
| disposal | `AuctionObservation` 的 neutral observation；provider 只使用有來源證據的屬性 | 沒有來源證據就不載入處置名單，也不改一般商品 fill/accounting 規則；synthetic neutral event 仍可測試 `disposal=true` | event/cache identity；不另存今日名單 |
| auction post-state | reducer 依已知 `AuctionPurpose` 與 `AuctionUncross` 推導 | opening、closing、periodic 各依固定 transition 處理；purpose 不明時保留 unknown，不以時間或背景補完 | reducer/event version |
| order-entry policy | `TradingContext` 讀取 event signal、post-event state 與 session phase | signal/state 未知時回傳 `Unknown`，不放行 order | strategy/execution version |
| execution fill policy | effective `simulation`、instrument economics 與已驗證 contract 設定 | config 缺失或矛盾時在 config/planner 階段拒絕 | effective config／execution plan checksum |
| strategy-visible metadata | event、`MarketStateView`、`TradingContext` 與 session context | 只呈現已觀察或已驗證值；不查 provider API | event、strategy 與 run artifact identity |

production flow 是 YAML 經 `RunConfig`、effective values 與 frozen `ExecutionPlan` 的既有
contract/session/economics 初始化，加上 provider normalizer 輸出的 neutral event，經由
`ReplayCore -> MarketState -> TradingContext -> execution` 完成。replay 與 backtest 直接使用
`PlannedPartition` 已物化的 `SessionPlan` 與 contract；不在 runner path 重新從 YAML 推導。
reserved 或無法解讀的 provider evidence 會映射為 `Unknown` 並保留 warning；profile、metadata DB、
crawler 與 web lookup 不在 core path。這讓缺少 market background 的商品仍能回播無關事件，同時不把
unknown 靜默降級成 false、`Continuous` 或可執行 order。

## 5. TradingContext

MarketState 保存 source-derived facts；`TradingContext` 保存目前 event 的決策投影。它分開表達：

- new order entry 是否允許。
- matching 是否可用，以及 `Continuous`／`CallAuction` 類型。
- 目前 event 的 auction observation 與 new-order entry restriction。
- provider-neutral policy version。

context 只使用 session phase、目前 event、更新後 `MarketStateView` 與 neutral market signal。
本次 `AuctionUncross` 的 matching 仍是 `CallAuction`，不能用 uncross 後的 state 把本次
fill eligibility 改成 continuous；下一個 event 才讀取 post-state。WarmUp auction event 可
更新 state 並呼叫 strategy，但不作正式 fill；CoolDown 不接受新 order 或 fill。origin event
永遠不能填入該 callback 新建的 order。

## 6. Strategy visibility boundary

Replay 只保證 callback 看見目前 event 已 atomic commit 後的 read-only state，以及 deterministic
universe state order；API 不提供 next event 或 future state。Strategy lifecycle、output capabilities、
orders 與 feedback 的 canonical contract 見[執行與帳務模型](execution-model.md#2-strategy-lifecycle)。

## 7. 版本 identity

目前版本 identity（`event_schema` 亦由公開 CLI 報告）：

```text
cli_contract=4
config_schema=3
run_manifest=4
market_types=11
event_schema=9
canonical_event=9
cache_format=3
accounting=8
```

run manifest v4 的 `versions` 保存 event／canonical event、MarketState／reducer、ordering、replay、strategy API 與 execution／fill model 的實際版本，讓撮合語意變更可追溯。

其他直接影響 replay 的 identity 包含 normalizer mapping、ordering rule、session calendar/profile/window、replay plan、MarketState reducer 與 canonical checksum version。任何不相容內容都需拒絕；cache 可由 compatible verified source 重建。

來源格式的具體 mapping 見[Teralion 介面](../interfaces/teralion.md)及其市場文件連結。
