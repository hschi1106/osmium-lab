# M5：warrant 與 option market expansion

## 1. 目的與目前狀態

M5 承接已完成的 M4 TPEx vertical slice，加入產品需求中的 warrants 與 options。
M5 不是把所有 market format 做成未驗證的 generic plugin；每一個新 market、
instrument kind 與 source format 都必須先由實際 Teralion fixture 固定，再加入
對應 interface、normalizer、session rule 與 acceptance evidence。

截至 2026-08-03，目前狀態如下：

| 項目 | 狀態 | 說明 |
| --- | --- | --- |
| M4 prerequisite | `complete` | [M4 formal acceptance report](../verification/evidence/m4/formal-2026-08-01/acceptance-report.yaml) 為 `Passed`。 |
| M5 specification | `complete` | 本文件已固定 M5-W／M5-O 的範圍、entry gates 與 acceptance catalog。 |
| M5 entry gate | `complete` | M4 prerequisite、實際 fixture、授權範圍、official protocol review 與 no-secret boundary 均已記錄。 |
| M5-W fixture／provenance | `complete` | TWSE `03003T`／`2026-07-20` regular fixture，111 筆；underlying／put／strike／expiry／unit provenance 已固定。 |
| M5-O fixture／provenance | `complete` | TAIFEX `TXO24000U6`／`2026-07-28` cross-day fixture，540 筆；TAIEX／put／strike／expiry／multiplier provenance 已固定。 |
| M5 implementation | `complete` | 完成 explicit `taifex_opt` query、warrant／option normalizer、session、state、source/cache、config reference 與 `OptionsV1` accounting。 |
| M5 formal acceptance | `complete` | [M5 formal acceptance report](../verification/evidence/m5/formal-2026-08-01/acceptance-report.yaml) 的 M5-AC-01～10 全部 `Passed`。 |
| TPEx warrant extension | `complete` | `72328U`／`2026-07-20` 的 real fixture、dedicated profile、offline replay 與 focused acceptance 已通過；不改寫 2026-08-01 的 M5-W formal scope。 |

M5-W 與 M5-O 都已在實際 fixture 上完成 source、mapping、session、state、cache、
offline replay、simulation、accounting 與 determinism 驗證；未宣稱 fixture 未覆蓋的
其他 warrant／option format 自動支援。來源授權與欄位 provenance 見
[M5 source selection evidence](../verification/evidence/m5/source-selection-2026-08-01.yaml)。

本次交付的關鍵結果：

- M5-W：`WARRANT_REALTIME` 60 筆、`WARRANT_SNAPSHOT` 51 筆；全部正常化為 99 個
  `QuoteSnapshot` 與 12 個 closing indicative events，TWSE raw status／limit flags
  維持 atomic annotation。
- M5-O：`I020`／`I022`／`I080`／`I082` 進入 timeline；`I021`／`I023`／`I030`／
  `I070`／`I072` 保留為 known-skipped diagnostics；跨日 after-hours 與 regular
  window 以 `match_time` 判定，未把 close／stats 假造為 domain event。
- formal run 在 network disabled sandbox 中完成：warrant 181,854 events／6 fills，
  option 500,304 events／4 fills；兩者均通過 10 次 byte-identical rerun、3 次
  universe permutation、cache rebuild、debug/release comparison 與 corruption rejection。

## 2. Scope and sequence

### 2.1 M5-W：warrants

- 先選定一個 explicit TWSE 或 TPEx warrant symbol 與交易日。
- 固定 underlying identity、warrant kind、call/put、strike、expiry、currency、
  multiplier 與 quantity unit；missing metadata 必須保持 unknown 或使 source
  不可發布為 complete。
- 以實際 quote/trade formats 建立 warrant interface 與 regular-session fixture。
- 驗證 warrant-specific price precision、unit、reference/limit fields、session
  boundary 與 order/fill economics。

### 2.2 M5-O：options

- 選定一個 TAIFEX option symbol，另行固定 trading date、underlying、expiry、
  strike、call/put、multiplier 與 regular／after-hours applicability。
- 以真實 `book`、`trade`、`close`、`stats` source kinds 中實際被納入 scope 的
  formats 建立 options interface；`close`／`stats` 不因下載存在就自動成為 timeline
  event。
- 驗證跨日 trading-date、after-hours boundary、五檔 book、trade batch、contract
  economics 與 futures/options accounting isolation。

M5-W 與 M5-O 可以分開 commit、fixture 與 formal report；先完成的子範圍不能把另
 一個子範圍標為 complete。

## 3. Shared architecture contract

M5 必須沿用 M1-M4 已固定的邊界：

- verified local source 可重用；replay cache 可刪除、失效與重建。
- Teralion wire documents 與 domain events 分離，normalizer 之外不能依賴 wire type。
- `match_time` 是唯一 replay time，tie-break deterministic 且不虛構交易所全域順序。
- market state 只由合法 trade 與完整五檔 snapshot 更新，不重建 queue position。
- strategy 只能讀取 state/context，不得修改 market state。
- replayer 只開啟 explicit strategy universe 所需的 streams。
- order origin event 不可 fill；跨 instrument 不可誤填；accounting 依 instrument
  economics dispatch。

## 4. Entry gates

每個 M5 子範圍都必須具備：

1. coverage、range、daily instrument 與 ticks 的完整 cursor acquisition。
2. source market、instrument kind、symbol、trading date、session、format、quantity
   unit 與 metadata identity 的 strict validation。
3. official market protocol／contract specification review 與 provenance。
4. raw page、selected fixture、daily metadata、source/cache lineage、redistribution
   scope、checksums 與 secret scan。
5. mapping version、canonical event version、cache compatibility 與 invalidation policy。
6. network-disabled acceptance；API key 不進 source、manifest、log、run 或 commit。

上述六項 entry gate 的實際結果與 checksum 集中於
[source selection evidence](../verification/evidence/m5/source-selection-2026-08-01.yaml)
及 [formal acceptance evidence](../verification/evidence/m5/formal-2026-08-01/acceptance-report.yaml)。

## 5. Delivery slices

### M5-W

1. warrant fixture/provenance 與 instrument economics spec。
2. warrant wire parser、quote/trade normalizer 與 strict error tests。
3. source partition/cache、regular session replay、simulation/accounting integration。
4. TWSE/TPEx + warrant multi-instrument determinism、stream audit 與 formal evidence。

### M5-O

1. option calendar、trading-date、contract identity 與 session profile spec。
2. option book/trade normalizer、cross-day boundary 與 unsupported-format tests。
3. source partition/cache、option accounting、fee/tax/multiplier integration。
4. TAIFEX futures + option multi-instrument determinism、reconciliation 與 formal evidence。

## 6. Acceptance catalog

| ID | 驗收條件 | 結果／證據 |
| --- | --- | --- |
| M5-AC-01 | M4 complete、fixture authorization 與 official protocol review | `Passed`；source-selection evidence、M4 report |
| M5-AC-02 | instrument identity、underlying、expiry、strike、call/put、multiplier、quantity provenance | `Passed`；fixture metadata、M5 configs、official reference |
| M5-AC-03 | source cursor、session、trading-date、format、checksum integrity | `Passed`；fixture-integrity、fixture-data、plan/verify logs |
| M5-AC-04 | warrant／option mapping positive、negative、golden tests | `Passed`；兩個 M5 fixture tests 與 artifact checksums |
| M5-AC-05 | book/trade state atomicity、raw flags、unsupported format strict handling | `Passed`；normalizer tests、state profiles、replay logs |
| M5-AC-06 | source/cache reuse、offline rebuild、cache compatibility、no-network boundary | `Passed`；cache-rebuild logs、sandbox report |
| M5-AC-07 | mixed-market ordering、state isolation、strategy no-lookahead、stream selection | `Passed`；stream-open audit、replay/backtest logs |
| M5-AC-08 | multiplier、unit conversion、fee/tax、fill、ledger reconciliation | `Passed`；OptionsV1 test、accounting isolation、positions |
| M5-AC-09 | 10 reruns、3 discovery permutations、cache rebuild、debug/release byte identity | `Passed`；formal determinism logs |
| M5-AC-10 | corruption、secret scan、performance、traceability、formal report | `Passed`；corruption logs、fixture scan、performance YAML、report |

## 7. 完成 M5 的實際步驟

1. 先確認 M4 formal acceptance，再為 warrant 與 option 各選定 exact symbol、trading
   date、session 與 source market；禁止以 synthetic payload 代替來源證據。
2. 以 coverage、symbol range、daily、ticks cursor 完整取得來源，保留 raw page、daily
   metadata、cursor identity、received_at 與 match_time，並執行 secret scan。
3. 依 official protocol 與 instrument reference 固定 underlying、expiry、strike、
   call/put、currency、multiplier、quantity unit 及 provenance；缺失欄位維持 unknown。
4. 將 wire market 與 domain market 分離：warrant 使用既有 TWSE quote shape 的專用
   mapping；option 使用明確 `taifex_opt` query 與 `TeralionTaifexOptions` mapping。
5. 實作 strict normalizer、session/calendar、MarketState profile、source/cache descriptor
   與 invalidation，明確列出 timeline formats 和 known-skipped formats。
6. 將 instrument kind／reference 綁進 effective config identity，依 unit、multiplier、
   fee/tax 與 fill model 選擇 accounting；option 使用 `OptionsV1`，future 使用
   `FuturesV1`。
7. 以真實 fixture 執行 plan、verify、replay、backtest，檢查 stream audit、state
   isolation、reconciliation、inspect 與 corruption rejection。
8. 在禁止網路且沒有 API key 的環境跑完整 formal harness：10 次重跑、3 次 universe
   permutation、cache rebuild、debug/release byte comparison，最後更新 traceability。

## 8. Out of scope

- 未有實際 fixture 的 market、instrument kind 或 Teralion format。
- 泛用衍生品定價、volatility surface、risk engine、portfolio margin 或交易所撮合
  模擬。
- queue position、逐筆 order book reconstruction、distributed source lake 或 plugin
  marketplace。

## 9. Completion criteria

M5 只有在 M5-W 與 M5-O 各自的 fixture、mapping、source/cache、simulation、determinism
與 formal acceptance 全部通過後完成。任何一個子範圍 `Blocked` 或 `Partial` 時，
M5 overall 必須保持 `partial` 或 `blocked`，不得用另一子範圍的證據升格。

## 10. References

- [product requirements](../product-requirements.md)
- [TWSE interface](../interfaces/twse.md)
- [TPEx interface](../interfaces/tpex.md)
- [TAIFEX interface](../interfaces/taifex.md)
- [data requirements](../requirements/data.md)
- [replay requirements](../requirements/replay.md)
- [traceability matrix](../traceability.yaml)

## 11. TPEx warrant extension（completed focused scope）

本次 extension 補上 TPEx warrant 的 explicit `InstrumentProfile::Warrant`、
`TeralionTpexWarrant` cache mapping、M2／M3 routing、dedicated market-state reducer、
real fixture 與可重現的 acceptance harness。它只接受實際 fixture 證實的 TPEx
`WARRANT_REALTIME`／`WARRANT_SNAPSHOT` quote formats；未確認的 TPEx warrant wire format
仍由 strict parser 拒絕。

fixture 是 [`72328U/2026-07-20`](../../fixtures/teralion/tpex/72328U/2026-07-20)，包含
4 筆 `WARRANT_REALTIME`、7 筆 `WARRANT_SNAPSHOT`，沒有成交資料。underlying、put、strike、
expiry、currency、multiplier 與 trading-unit provenance 由 fixture metadata、Teralion
daily response、TPEx warrant issue reference 與官方交易規則共同記錄。

focused acceptance 使用
[`tools/run_tpex_warrant_acceptance.sh`](../../tools/run_tpex_warrant_acceptance.sh)，在
network disabled 且 credentials absent 的環境完成 `plan`、`verify`、`replay`、`backtest`、
`inspect`、10 次 byte-identical rerun、cache rebuild、debug/release comparison 與
corruption rejection。11 筆 source records 正規化為 3 個普通 quote、2 個 opening auction
與 6 個 closing auction events；單一 strategy universe 只開啟 `72328U` 一條 stream。

這是 TPEx warrant 的 exact-symbol/date extension acceptance，不把其他 TPEx warrants、
成交 `TradeBatch`、零股或未被 fixture 固定的 formats 一併宣稱為支援。證據見
[`TPEx warrant acceptance report`](../verification/evidence/m5/tpex-warrant-2026-08-03/acceptance-report.yaml)。
