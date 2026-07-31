# M4：TPEx regular equity

## 1. 目的與狀態

M4 是 M3 之後的第一個新 market vertical slice，只加入 TPEx 普通股票的
`regular` session。M4 不同時承接權證、選擇權或 TPEx 零股；那些範圍移至 M5，
避免在沒有實際 wire fixture 前猜測共用 mapping。

目前狀態為 `specification: complete`、`implementation: not_started`。正式實作前
必須先取得一份獲准的 Teralion TPEx fixture，固定 exact symbol、trading date、
source formats 與 session calendar；symbol/date 未經 fixture gate 確認前，不得在
config 或測試中填入 synthetic value。

## 2. 範圍

### 2.1 包含

- 一個 explicit TPEx equity symbol、一天已結束的交易日及 `regular` session。
- Teralion `market=tpex` 的完整 cursor source download、daily instrument metadata
  與 immutable source revision。
- TPEx quote wire 到 domain `QuoteSnapshot` 的 strict mapping。
- 若實際 fixture 包含已被 TPEx interface 核准的成交 format，加入 `TradeBatch`；
  沒有 fixture 證據時不宣稱成交支援。
- 完整五檔 snapshot replacement、累計成交量、成交價量與 raw status／limit flags
  的 preservation 或明確 `Unknown` policy。
- TPEx partition source、rebuildable replay cache、offline replay/backtest、
  deterministic ordering 與 per-instrument state。
- TPEx instrument economics、quantity unit、currency 與 provenance。

### 2.2 不包含

- TPEx odd-lot、warrant、option 或其他未由本 fixture 固定的 format。
- 逐筆委託簿、queue position、撮合引擎或交易所撮合重建。
- 改變 `match_time` replay clock、strategy read-only boundary、order origin event
  或 M3 已驗證的 multi-stream merge contract。
- 以 bars、quotes/trades derived endpoint 或 synthetic records 取代 raw ticks。

## 3. Source 與 fixture contract

### 3.1 Entry gates

M4 source gate 必須同時滿足：

1. coverage 確認 `(tpex, trading_date)`，range 與 daily instrument identity 可驗證。
2. `/api/feed/ticks/{symbol}` 使用固定 `received_at` window、`kinds=quote`、
   limit 與 opaque cursor，所有頁走到 terminal cursor。
3. 每頁 response、query identity、page checksum、record/format counts 與 raw
   market/symbol/date identity 均保存；API key 不進 artifact。
4. fixture redistribution scope、source checksum、extraction predicate 與 mapping
   version 由 metadata 固定，並通過 secret scan。
5. 官方 TPEx protocol／market rules 與實際 payload 的欄位、quantity unit、時間與
   flags review 完成。

### 3.2 Source boundary

下載 clock 與 replay clock 必須分離：source 以 `received_at` 選取，timeline 只使用
`match_time`。session window 使用 half-open interval，開收盤 margin、缺少或無效
時間、跨日日期與 out-of-window record 都必須由 normalizer 明確處理；不可用本地
檔案日期猜測 trading date。

Fixture extraction 只能移除 response envelope、cursor 及明確排除的 unsupported
formats；selected item 必須保留原始 JSON value bytes 與 source-page order。重新
抽取必須 byte-for-byte 一致。

## 4. Domain mapping

TPEx mapping 必須位於 TPEx interface／normalizer boundary，不能讓 Teralion wire
format 滲入 replay engine 或 strategy API。

- `QuoteSnapshot`：每次合法完整五檔取代同一 instrument 的舊 book，不以差分或
  `max(previous, current)` 修復缺欄位。
- `TradeBatch`：只有 fixture 與 interface registry 都確認成交 shape 時才啟用；
  intermediate／final semantics 必須由 TPEx payload 證明，不能沿用 TWSE-specific
  flags。
- `match_time` 相同時沿用既有 `OrderingKey` 與 TPEx market rank；若來源沒有可用
  source sequence，必須記錄 deterministic fallback 的限制。
- 未知 format、缺少 required field、非法數值、market/symbol mismatch 與不完整五檔
  必須 strict reject 或進入明確的 non-replayable error，不可靜默降級。
- raw status／limit flags 保留原值；只有官方文件與 fixture 共同支持的 bits 才能
  轉為 typed annotation。

## 5. Implementation slices

每個 slice 都要有獨立 commit 與 focused validation：

1. 固定 TPEx interface、fixture provenance、session/calendar 與 mapping registry。
2. 取得並驗證 authorized fixture，加入 raw-byte shards、daily instrument、metadata
   與 fixture-set checksum。
3. 實作 TPEx query identity、source validator、normalizer positive/negative tests。
4. 接入 partition source、cache builder、CLI plan/sync/verify/cache/replay path。
5. 加入 TPEx + M3 instruments 的 bounded k-way merge、state、strategy no-lookahead
   與 stream-open audit tests。
6. 固定 TPEx quantity/economics、order/fill eligibility 與 ledger reconciliation
   evidence；不修改 M1-M3 semantics。
7. 執行 network-disabled acceptance，完成 traceability、performance 與 operations
   文件。

## 6. Acceptance catalog

| ID | 驗收條件 |
| --- | --- |
| M4-AC-01 | authorized TPEx fixture、daily metadata、source page 與 artifact checksums 通過 |
| M4-AC-02 | TPEx market/symbol/date、cursor terminal、received_at window validation 通過 |
| M4-AC-03 | selected wire formats 的 quote mapping 與 unknown/invalid negative tests 通過 |
| M4-AC-04 | 完整五檔 replacement、trade/volume/raw flags preservation 通過；無 queue reconstruction |
| M4-AC-05 | TPEx source revision immutable、cache 可刪除並離線重建、second run 不重抓 source |
| M4-AC-06 | TPEx regular session boundary 與 `match_time` ordering golden 通過 |
| M4-AC-07 | TPEx + M3 multi-instrument merge、state isolation、no-lookahead 通過 |
| M4-AC-08 | strategy 只讀 market state，universe 外 stream 不被開啟 |
| M4-AC-09 | TPEx quantity/economics、fill、fee/tax 與 ledger reconciliation 有 provenance |
| M4-AC-10 | 10 次 rerun、discovery permutation、cache rebuild、debug/release 結果 byte-identical |
| M4-AC-11 | network-disabled/no-key plan、verify、replay、backtest、inspect 成功 |
| M4-AC-12 | corrupted source/cache/run、secret scan、traceability 與 performance evidence 通過 |

## 7. Completion criteria

M4 只有在 M4-AC-01..12 全部 `Passed`、formal report 沒有 `Blocked`／`Partial`／
`NotRun`，且 TPEx fixture 與 source/cache lineage 可由新 checkout 重建時完成。
完成後才可開始 M5；M5 不得把未完成的 TPEx work 隱含納入自身 scope。

## 8. References

- [TPEx interface](../interfaces/tpex.md)
- [data requirements](../requirements/data.md)
- [replay requirements](../requirements/replay.md)
- [data sync design](../design/data-sync.md)
- [traceability matrix](../traceability.yaml)
