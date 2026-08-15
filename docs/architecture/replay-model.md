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

payload 支援 `QuoteSnapshot`、`BookSnapshot`、`TradeBatch`、`IndicativeOpeningAuction` 與 `IndicativeClosingAuction`。價量使用 checked exact decimal 與具單位的 quantity；unknown 與 no-observation 不以零值代替。

同一 source record 的不可分割成交、book 與 annotations 形成單一 atomic event。auction event 是試算觀察，不是實際成交。

## 2. 排序與時間

`match_time` 是 replay clock 與第一排序鍵。ordering rule version 3 的內容鍵依序為：

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
- market-specific annotations。
- `last_match_time`、state version 與 applied event reference。

`Observation` 的更新規則：

- `NoObservation`：保留既有 field。
- `Set(value)`：以目前 event 與 value 取代。
- 明確 unknown：保存 unknown reason，不推定 value。

`QuoteSnapshot` 更新完整 book，並依 observation 更新 trade、volume 與 annotations。`BookSnapshot` 取代完整 book。`TradeBatch` 更新最近成交與可用 cumulative volume，不修改 book。indicative auction 更新可觀察試算資訊，但不建立實際成交。

reducer 先驗證整個 transition，再一次提交。非法價格、數量單位、時間倒退或 cumulative-volume policy 違反時，state 不變。strategy 取得的 `MarketStateView` 沒有 mutation API。

reducer 支援 carry 與 reset boundary policy；目前 CLI runner 對每個 planned segment 使用 `ResetObservableFields`，在下一 segment 首個 event 前重設 observable fields。

## 5. TradingContext

MarketState 保存 source-derived facts；`TradingContext` 保存目前 event 的決策投影。它分開表達：

- new order entry 是否允許。
- matching 是否可用，以及 continuous／call-auction 類型。
- pending order 對目前 event 的 fill eligibility。
- 穩定 reason code 與 policy version。

context 只使用 session phase、目前 event、更新後 state 與 market-specific annotations。WarmUp trial event 可更新 state 並呼叫 strategy，但不作正式 fill；CoolDown 不接受新 order 或 fill。origin event 永遠不能填入該 callback 新建的 order。

## 6. Strategy lifecycle

```text
resolve factory and parameters
  -> initialize
  -> session / timer / event callbacks
  -> commit callback output
  -> simulation feedback callbacks
  -> finalize
```

`StrategyEventContext` 提供目前 occurrence、event、selected state、所有 universe states、TradingContext、session context 與 decision time。所有 state 都是 post-event read-only view；API 不提供 next-event 或 future-state access。

`StrategyOutputSink` 的 indicator、order intent、scheduled request 與 timer 依 runner capability 開放。callback 成功才提交 output；error 或 panic 使 run failed。strategy parameter 與 output 使用 canonical encoding，以固定 identity 參與重現性檢查。

## 7. 版本 identity

目前公開 CLI 報告：

```text
cli_contract=4
config_schema=2
run_manifest=2
event_schema=3
cache_format=1
accounting=6
```

其他直接影響 replay 的 identity 包含 normalizer mapping、ordering rule、session calendar/profile/window、replay plan、MarketState reducer 與 canonical checksum version。任何不相容內容都需拒絕；cache 可由 compatible verified source 重建。

來源格式的具體 mapping 見 [介面文件](../README.md#資料介面)。
