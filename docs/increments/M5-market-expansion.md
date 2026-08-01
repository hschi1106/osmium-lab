# M5：warrant 與 option market expansion

## 1. 目的與目前狀態

M5 承接已完成的 M4 TPEx vertical slice，加入產品需求中的 warrants 與 options。
M5 不是把所有 market format 做成未驗證的 generic plugin；每一個新 market、
instrument kind 與 source format 都必須先由實際 Teralion fixture 固定，再加入
對應 interface、normalizer、session rule 與 acceptance evidence。

截至 2026-08-01，目前狀態如下：

| 項目 | 狀態 | 說明 |
| --- | --- | --- |
| M4 prerequisite | `complete` | [M4 formal acceptance report](../verification/evidence/m4/formal-2026-08-01/acceptance-report.yaml) 為 `Passed`。 |
| M5 specification | `complete` | 本文件已固定 M5-W／M5-O 的範圍、entry gates 與 acceptance catalog。 |
| M5 entry gate | `partial` | M4 prerequisite 已完成；M5-W／M5-O 的 fixture acquisition plan 尚未 review。 |
| M5-W fixture／provenance | `not_started` | 尚無已 review 的 warrant exact symbol、trading date 與 authorized fixture acquisition record。 |
| M5-O fixture／provenance | `not_started` | 尚無已 review 的 option exact symbol、trading date 與 authorized fixture acquisition record。 |
| M5 implementation | `not_started` | 尚無 warrant／option-specific interface、normalizer 或 contract accounting implementation。 |
| M5 formal acceptance | `not_started` | 尚無 M5 formal evidence。 |

因此 M4 prerequisite 已關閉，但 M5-specific entry gate 尚未關閉；目前 M5 不宣稱
任何 warrant／option format、mapping、session 或 accounting 已支援。最新的既有
multi-market display/replay workflow 仍只覆蓋 M3／M4 reference universe，不改變
上述 M5 狀態。下一個最小可驗證工作是分別完成 M5-W 與 M5-O 的 fixture acquisition
plan、authorization、provenance 與 official protocol review；在此之前不得以
synthetic payload 取代來源證據。

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

| ID | 驗收條件 |
| --- | --- |
| M5-AC-01 | M4 complete、warrant/option fixture authorization 與 official protocol review 通過 |
| M5-AC-02 | instrument identity、underlying、expiry、strike、call/put、multiplier 與 quantity provenance 通過 |
| M5-AC-03 | source cursor、session、trading-date、format 與 checksum integrity 通過 |
| M5-AC-04 | warrant 與 option mapping 的 positive/negative/golden tests 通過 |
| M5-AC-05 | book/trade state atomicity、unknown flags 與 unsupported formats strict handling 通過 |
| M5-AC-06 | source/cache reuse、offline rebuild、cache compatibility 與 no-network boundary 通過 |
| M5-AC-07 | mixed-market ordering、state isolation、strategy no-lookahead、stream selection 通過 |
| M5-AC-08 | contract multiplier、unit conversion、fee/tax、fill 與 ledger reconciliation 通過 |
| M5-AC-09 | 10 次 rerun、discovery permutation、cache rebuild、debug/release 結果 byte-identical |
| M5-AC-10 | corruption、secret scan、performance、traceability 與 formal report 完整 |

## 7. Out of scope

- 未有實際 fixture 的 market、instrument kind 或 Teralion format。
- 泛用衍生品定價、volatility surface、risk engine、portfolio margin 或交易所撮合
  模擬。
- queue position、逐筆 order book reconstruction、distributed source lake 或 plugin
  marketplace。

## 8. Completion criteria

M5 只有在 M5-W 與 M5-O 各自的 fixture、mapping、source/cache、simulation、determinism
與 formal acceptance 全部通過後完成。任何一個子範圍 `Blocked` 或 `Partial` 時，
M5 overall 必須保持 `partial` 或 `blocked`，不得用另一子範圍的證據升格。

## 9. References

- [product requirements](../product-requirements.md)
- [TWSE interface](../interfaces/twse.md)
- [TPEx interface](../interfaces/tpex.md)
- [TAIFEX interface](../interfaces/taifex.md)
- [data requirements](../requirements/data.md)
- [replay requirements](../requirements/replay.md)
- [traceability matrix](../traceability.yaml)
