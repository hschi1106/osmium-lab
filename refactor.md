# market-types 重構計畫

> 狀態：MT-001～MT-013、本次 stability migration、Alpha production differential 與正式 git dependency pin 均已完成。最新證據以下方接手驗收狀態為準。

## 接手驗收狀態（2026-09-14）

舊 session `01a09d74-0d78-75b2-b448-dd82451ac52e` 最後停在 source-adapter profile resolver 收斂。
以下以目前工作樹與重新執行結果為準；前文的 51／52 tests、缺少真實 partitions、舊 benchmark
與舊版本數字是歷史 checkpoint，不代表目前完成狀態。

- 接手 baseline：`cargo test --workspace` 306 tests 通過；acceptance Python 8 tests、compact fixtures、
  smoke fixture bundle 與 license verifier 通過。
- TWSE `44a8468b3056485a…`／TPEx `d55cba9ce2e2d833…` immutable partitions 已重新通過
  Teralion adapter certification；兩市場 realtime／snapshot 都有 trigger、trial、formal result 與 resume。
- Review 發現 scheduled execution 只在完整 firm book event 更新交易資格，status-only `MarketStatus`
  與 `IndicativeAuction` 可能沿用舊資格；現已分離 latest trading state 並加入 runner regression。交易資格更新不得刷新
  depth timestamp、補充已消耗 depth，或把 trial book 當成 executable evidence。
- 核心 review 另修正跨商品誤取消、slippage 後 price policy 驗證，以及 `MarketStatus` 誤清除
  indicative state；五個核心 packages 的 139 tests／Clippy 通過。Run manifest 升為 v4 並保存 runtime versions。
- 第二輪 review 以 reducer 拒絕非 `IndicativeAuction` 的 TWSE／TPEx trial payload；typed kind
  與 delayed flags 矛盾時採 `Unknown`。新增兩檔 slippage 全量驗證 regression，保證非法價格
  不留下 partial fill／depth mutation；六個相關 packages 的 148 tests／Clippy 通過。
- Adapter review 修正 cache identity 漏掉 mapping name 的碰撞風險；cache format 升為 v3，
  新 identity 同時保存 mapping name／version，舊 caches 必須重建。CLI source-adapter resolver 與 online runtime 已收斂，整合 gates 通過。
- 正式 lab revision 為 `964831b2dfc2baab0a6eafa801e1cbffd1bb274d`，已推送工作分支。
  Alpha 六個 git dependencies 與 Cargo.lock 已更新；未使用 path override 的 workspace 53 tests、fmt、
  `cargo clippy --workspace --all-targets --locked -- -D warnings` 全部通過。四個 arbitrage strategy versions 升為 v2。
- 整合驗收：workspace 320 tests、fmt、Clippy 全部通過；四個既有 benchmarks 的 checksum
  equivalence assertions 通過。Fresh synthetic fixture helper 的獨立 workspace 也完成 migration、
  fmt／Clippy 與 offline flow；release clean-machine smoke 已通過 config v3。
- 真實 stability partitions 以獨立 validation root 重跑 v2 stale → v3 rebuild → reuse：3,403 events，
  event checksum `6ef7cd0bf7bcfb6b7cb976d5cf458308d7cf19416ab1ca65b8337faa576a63ed`，
  final-state checksum `b16d7f36ded1dd6bc191a78bc510f7f5632be4cd9cebf6f1021b8123a65407d0`；
  acceptance backtest 4 orders／4 fills，inspect successful。Source revisions 未變更。原始 logs 位於
  ignored `target/handoff-final-acceptance-20260914/logs`，這不是 Alpha 策略 differential 證據。
- 提交：Lab 核心 `964831b`；Alpha migration／pin `02ffc03`、differential harness `7e51dce`。
  兩個 repositories 使用 `codex/market-types-migration` 工作分支。
- Acceptance Python 10 tests 通過；原始行情、credential 與 build artifacts 不納入提交。
  Alpha differential 的輸入、預期差異與完整 runner 財務對照見 §4.6。

### 核心需求證據索引

以下索引對應目前程式與已執行的 workspace／acceptance gates；Alpha differential 與正式 pin
另列為跨 repository gate，不以核心測試代替。

| 項目 | 現況證據 |
| --- | --- |
| MT-001 signed／zero price | `market-types/tests/price.rs`、TAIFEX fixtures、`execution-sim` signed fill／absolute notional／multi-ledger reconcile tests |
| MT-002 明確 instrument contract | `run-planner/tests/config.rs` 的 outright/spread identity、option reference 必填／一致性 tests；config v3 examples |
| MT-003 symbol storage encoding | `data-sync::partition` 的 reversible path、collision、long symbol 與 calendar spread round-trip tests |
| MT-004 成交命名 | `ObservedTrade`／`TradeObservationKind`／`TradeBatchOrdering` 已遷移所有 direct consumers，沒有舊型別 alias |
| MT-005／006 MatchTime | `match_time.rs` module 與 public tests；offset／epoch／formatter 邊界及 canonical encode/decode 對稱性 tests |
| MT-007 Decimal | `decimal_display_is_exact_and_canonical`、TUI exact price rendering；浮點只留在 chart coordinates |
| MT-008 codec limits | count/frame/trailing bytes/mutation tests、16 MiB cache record guard、所有 stale schema 拒絕 tests |
| MT-009 TradingDate | checked epoch constructor、四位數日期 parse/display／canonical 邊界 tests |
| MT-010 hot path | 四個 benchmarks 在本次整合 gates 重跑 checksum equivalence；真實 source pipeline 與歷史 peak RSS 記錄保留 workload 限制 |
| MT-011 annotation boundary | TWSE／TPEx 獨立 public types，各 256-byte bit contract tests；private decoder 共用 |
| MT-012 test layout | `market-types/tests/` 分檔 public API tests，source module 沒有內嵌 public API test suite |
| MT-013 provider boundary | source adapter resolver/runtime、mapping name/version cache identity、sync/verify/cache mismatch regressions、generic Python integrity/lifecycle 與 adapter-specific extractor/conformance |
| Stability isolation／execution | typed trial reducer gate、MarketStatus 保留 firm state、session clear、matching restriction／同商品取消、scheduled status-only activation、slippage atomicity tests |

## 舊 session checkpoints（歷史記錄）

以下保留當時的進度與效能量測限制；其中「尚未完成」不代表最新驗收狀態。

> 狀態：實作中。主要 market-types primitive、indicative-auction schema、TWSE／TPEx trial isolation、明確 instrument class／contract shape／session profile、calendar-spread signed price 與 storage encoding migration 已完成，詳見 MT-001～MT-012。此 stability migration 是破壞性變更，不保留舊 API 或 cache 相容層。2026-09-14 已以三個完整日 verified raw source partitions 完成新版 cache 重建、replay、backtest、五輪 byte-for-byte reproducibility 與 peak-memory 量測；這是 TWSE 2330／TPEx 6488／TAIFEX TXFH6 的非 stability、非 Alpha workload 證據。驗證現分成 provider-neutral partition／domain／Alpha gates 與各 adapter 自己的 wire conformance；含 stability raw records 的 Teralion partition 只影響 Teralion adapter certification，不再阻擋核心 migration。尚未完成的正式核心 gate 是 Alpha 對同一 provider-neutral stability event stream 的新舊語意差異驗證，以及取得可 pin 的 lab revision 後更新 Alpha dependency。效能數據以 MT-010 的 harness 更正後結果與真實資料補測為準，舊 benchmark 值不可當作 runner-only 比較。
>
> MT-010 已完成第一階段實作與 synthetic pipeline benchmark：replay merge 對每個 head event 只 canonical-encode/hash 一次，並重用同一 frame 作 replay checksum；TAIFEX sort key 改為每筆 event 計算一次。固定 synthetic five-level quote microbenchmark 最近一輪中位數：重複編碼基線 201.1 ms、single-encode 94.2 ms（每組 250,000 次、交錯 5 輪，約 2.13x）。50,000-event `ReplayCore::replay_stream` microbenchmark 中位數：基線 100.9 ms／495,348 events/s，新路徑 78.8 ms／634,355 events/s（約 1.28x，event checksum 與 final-state checksum 相同）。新增 verified-cache pipeline benchmark；50,000 筆 synthetic quotes 的 cache prepare 中位單次 0.895 s／55,860 records/s，cache scan 五輪中位數 0.030 s／約 1.67M records/s，cache-backed ReplayCore replay 五輪中位數 0.089 s／約 562k events/s，五輪 event checksum 與 final-state checksum 相同。另新增 49,152 筆 synthetic five-level quote 的 1／8／32-stream merge benchmark，Linux x86_64／rustc 1.97.1 三輪中位數分別為 79.3／84.7／90.4 ms（約 620k／581k／544k events/s），各 stream-count case 的 event checksum 與 final-state checksum 在重複執行間相同。新增 `osmium-runner` 完整 multi-backtest benchmark：8 個 TWSE 商品、49,152 筆 synthetic events，涵蓋 merge、strategy callback/output、simulator、ledger 與 finalize/reconcile；Linux x86_64／rustc 1.97.1 三輪中位數 139.1 ms／約 353k events/s，每輪 16 fills、147,458 筆 strategy output records，event 與 final-state checksums 相同。接著 `ReplayStateViews` 改為借用 core state map，移除一般／scheduled runner 每筆 event 建立 universe view `Vec` 的配置；相同 benchmark 兩次後測三輪中位數為 135.9／137.1 ms（約 362k／359k events/s），fill/output 數與兩種 checksum 完全相同。基線及兩次後測皆僅三輪、且非交錯執行；觀察到的差異約 1.4–2.3%，視為初步方向而非穩定加速宣稱。測試版本為 event schema 4／canonical event 4／ordering rule 4、rustc 1.97.1、Linux x86_64。verified-cache pipeline benchmark 的輸入由 repository-owned synthetic TWSE fixture 擴增；其他 benchmarks 直接生成相同形狀的記憶體內 synthetic events；皆非真實行情。這些測試未量 peak memory，也不能代表真實完整市場日表現或優化前後的完整 backtest 加速率。真實 TWSE／TPEx stability golden fixture 尚未取得；`osmium-alpha` strategy 與文件 migration 已完成，並在隔離副本以 path override 對目前 lab 工作樹執行 workspace 測試（51 項通過），但實際依賴仍 pin 在舊 lab revision，尚未完成正式 revision migration。
> 再執行一輪相同的完整 multi-backtest benchmark（三輪中位數 134.4 ms／約 366k events/s）後，優化後三次 runs 的三輪中位數為 135.9、137.1、134.4 ms；基線一次為 139.1 ms。前後 workload checksums、16 fills 與 147,458 strategy output records 相同；基線／後測沒有交錯執行且每次只有三輪，因此僅記錄為 allocation 移除後的初步觀察，不當成穩定效能收益。
> Alpha regression 更新（2026-09-14）：新增 production `try_continuous_exits` 的 interleaved stock-trial／futures-firm regression，path-override Alpha workspace 現為 52 項測試通過（arbitrage-core 50 項）；舊段落中 51 項是此測試加入前的 checkpoint。正式 dependency pin 與真實行情 differential 仍未完成。
> 2026-09-14：normalizer 不由 `trial + instant_trend` 推斷 `IntradayStability(direction)`；盤中 trial 保持 `IntradayUnclassified`，方向只保留在 annotations，供 Alpha 與先前明確 trigger 關聯。以 `.env` credential 將 2026-08-10 的 TWSE 3026 與 TPEx 2948 同步成 repository 外的 immutable partitions；generic integrity 與 Teralion adapter conformance 均通過，兩市場的 `STOCK_REALTIME`／`STOCK_SNAPSHOT` 都找到完整 lifecycle。真實 cache build 另發現第一檔零價市價委託量未建模，以及 pause sentinel 被空 `QuoteSnapshot` 錯誤清空 firm book；現分別由 `BookSide.market_order_quantity` 與 provider-neutral `MarketStatus` 表達。event schema／canonical／market-types 升為 v6／v6／v7，TWSE equity／warrant mapping 為 v9／v6，TPEx 為 v6／v6。新版 cache 與 replay 已在相同 partitions 完成；Alpha 跨版本語意差異驗證仍未完成。
> `/data` 補查（2026-09-14）：`/data/hschi1106/disposal-arbitrage-data-v3` 保留舊版 normalized replay JSON，可觀察 TWSE 2340 與 TPEx 5425 的完整約 120 秒 stability lifecycle；但對應 published source 只含 `DISPOSAL_ARBITRAGE_V3_POINTER`，不是 `STOCK_REALTIME`／`STOCK_SNAPSHOT` raw wire payload，故不能解除 source-format gate，也不為 breaking migration 恢復 pointer compatibility。另以 `target/m4-data` 的三個 verified revisions建立獨立 v2 validation root，完成真實 source pipeline量測；詳細數字見 MT-010。
>
> 範圍：本文件只規劃 `market-types` 及其必要的直接 downstream migration，不規劃整個 workspace 的全面重寫。來源依據為 `docs/product-requirements.md`、`note.md`、`review.md`、目前程式與測試、repository fixtures、Teralion 文件及交易所公開制度。

> 效能數據更正（2026-09-14）：上述早期 `full_multi_backtest` 結果的計時區包含複製整份 synthetic stream，不能作為 runner-only baseline／post-change 或峰值記憶體證據。benchmark 已改為計時前建構輸入、計時時 move streams，並增加至 21 rounds；修正後測量與舊數字限制見 MT-010。不可把 harness 修正前後差異宣稱為產品程式加速。

## 1. 目標與不變條件

重構後必須維持：

- Teralion wire payload 與 domain event 分離。
- `match_time` 仍是唯一 replay clock。
- 同一 source observation 的不可分割內容仍為單一 atomic event。
- 試算價量不得成為正式成交、正式 cumulative volume、一般 fill evidence 或正式 mark。
- 不從 flags 重建交易所內部撮合、queue、stability start/end 或參考價算法。
- exact values 不經 binary floating-point。
- 任何 event schema／canonical bytes 變更都明確升版；舊 cache 必須拒絕或由 verified source 重建。
- 本次 stability／indicative auction 調整採破壞性 migration；為維持 codebase 精簡與 domain correctness，不保留舊 API、舊 event variants、舊 cache decoder 或雙軌相容層。
- 每個步驟保持 commit-sized，先增加測試證據，再改模型，再做 downstream migration。

## 2. 來源調查與 stability 結論

### 2.1 已確認的 Teralion wire contract

Teralion 的 TWSE／TPEx 行情以 `type: "quote"` 表達，payload 欄位包含：

- `format`
- `deal`
- `bids`／`asks`
- `cum_volume`
- `limit_flags`
- `status_flags`
- `intermediate_print`

目前 normalizer 接受的 format family：

| Market／商品 | snapshot | realtime |
| --- | --- | --- |
| TWSE／TPEx equity | `STOCK_SNAPSHOT` | `STOCK_REALTIME` |
| TWSE／TPEx warrant | `WARRANT_SNAPSHOT` | `WARRANT_REALTIME` |

Teralion Feed Archive 文件在開盤／收盤 indicative pre-auction 的說明中，以 TWSE／TPEx quote message 的 `status_flags & 0x80` 辨識試算，並指出 `/ticks` 保留原始 `status_flags` 與 `format`。該文件說 TAIFEX 不使用此 flag，而以 `I022`、`I082` 等獨立 message format 表達試撮；此欄位契約的穩定期適用範圍仍待真實資料確認。

2026-09-14 重新核對官方 [Feed Archive API](https://docs.teraliontech.com/feed-archive/) 頁面：文件明確說 `/ticks` 原樣回放 client tick envelope，包含試算訊息；`/trades`、`/bars`、`/quotes` 僅提供 firm data，不包含其所述開盤／收盤 indicative prices。TWSE／TPEx 的試算標記說明為 quote message `status_flags & 0x80`，TAIFEX 使用 `I022`／`I082`／`I064` format。這確認查詢 stability 必須看未過濾 `/ticks`，不能由衍生 endpoints 重建；但頁面只描述 pre-open／close trial，沒有描述盤中 stability period 的 `deal`／book／zero quantity／`cum_volume` mapping。故這份官方文件仍不足以解除 stability source golden gate。

2026-09-13 進行的只讀 source sample：

- 查詢 1101／2026-08-13 開盤前 quote，實際看到同一 trial observation family 同時出現 `STOCK_REALTIME` 與 `STOCK_SNAPSHOT`，兩者皆有 `status_flags = 128`、`limit_flags = 0`。
- 查詢同商品 09:00–13:25 共 32,046 筆 quote，沒有 `limit_flags` 低兩 bits 非零的 stability sample。
- checked-in synthetic fixtures 的 `limit_flags` 也全為 0；目前 stability 測試僅使用人工建立的 flag，不能視為 source fixture 證據。

2026-09-14 追加查詢 3026／2026-08-10 與 2948／2026-08-10 的 regular quote ticks：兩市場的 trigger 與 stability-period trial 都可見於 `STOCK_REALTIME`，同時間也有 `STOCK_SNAPSHOT` observation。這證明 parser／reducer 的語意不能依賴其中單一 format；同 timestamp 的 realtime 與 snapshot 仍各自是不同 source observations，依既有 ordering 與 event identity 處理。

官方交易所規格的 bit contract 已另外核對：TWSE B.12.00 與 TPEx V.12.18 均將 `limit_flags` 最低兩 bit 定義為瞬間價格趨勢：`00` 一般揭示、`01` 暫緩撮合且瞬間趨跌、`10` 暫緩撮合且瞬間趨漲、`11` 保留。兩者均以 `status_flags` bit 7 表示試算；bit 6／5 在試算狀態表示延後開盤／收盤。TWSE 規格對暫緩撮合欄位另說明：若成交價量欄位有揭示，價格是最近一筆成交價、數量為 0，且不揭示買賣價量。這些是交易所 IP wire 規格語意，不足以單獨證明 Teralion JSON `deal`／`bids`／`asks` 如何映射此狀態。

原差異已由 verified partitions 釐清：2026-08-10 TWSE 3026 與 TPEx 2948 的盤中 trigger 都呈現 `status_flags=16`、`limit_flags=2`、`deal.quantity=0`、空 `bids`／`asks`，累計量保持不變。兩個 normalizer 現都將這種只有狀態、沒有 firm book／trade 的暫緩撮合 sentinel 映射為 `MarketStatus`；它保存累計量與 typed annotations，不建立零量成交，也不以空陣列清除先前 firm book／trade。同時拒絕沒有 VI trend、intermediate print 或仍帶有 book 的零量 deal。真實 TWSE 資料另有 1,886 個零價 book entries；官方 B.12.00 明定這是最佳第一檔的市價委託量，不是可成交零價格，故 domain 以 `BookSide.market_order_quantity` 分離保存。TWSE mapping identities 升為 v9／v6，TPEx 升為 v6／v6，令舊 derived cache 必須重建。真實 raw rows 存在 ignored validation root，不 check-in repository。

因此需修正語意界線：instant-trend bit 是「暫緩撮合／趨勢」證據，不等同「這筆價量與五檔是模擬集合競價結果」；`status_flags.trial` 才是交易所規格明確標示的試算揭示。正式實作不得只因 trial observation 同時帶 up/down trend 就宣稱是 stability-period indicative auction，也不得只因 trend bit 就把 payload 欄位統稱為試算值。Normalizer tests 固定兩個界線：帶趨勢 bit 的 trial 仍產生 `IntradayUnclassified` 並保留 annotations；`trial=false` 的 status-only pause observation 產生 `MarketStatus`，無成交且不改寫 firm book。真實 TWSE／TPEx partitions 已支持 zero-quantity／empty-book trigger mapping與完整 lifecycle；provider-neutral domain classification 仍不依賴 Teralion flags。

### 2.2 交易制度語意

TWSE 與 TPEx 公開制度均說明：盤中瞬間價格穩定措施觸發後暫緩撮合約兩分鐘，期間持續接受受限委託，並每五秒揭露模擬撮合價格、數量與最佳五檔；期滿後以集合競價撮合，再恢復逐筆交易。

因此需區分：

1. `stability trigger observation`：來源明確提供的趨漲／趨跌或延緩撮合註記。
2. `stability-period indicative observation`：期間內揭露的模擬成交價量與五檔。
3. `stability auction result`：期滿後真正以集合競價產生的正式成交結果。

第 2 類在 domain 上應屬 `IndicativeAuction`，不是 `QuoteSnapshot` 或 `TradeBatch`。第 1 類若 wire 沒有獨立 transition identity，就只能作為 observation／annotation 保存，不能合成 `StabilityStarted`。第 3 類才可依來源 format／flags 建立正式成交事件。

### 2.3 目前實作的確定問題

原先 TWSE／TPEx normalizer 的 `auction_phase()` 對盤中 trial、沒有 opening／closing marker 的情況回傳 `None`；`final_event()` 隨後建立 `QuoteSnapshot`。這會造成：

- 模擬 `deal` 可能被 reducer 寫入正式 recent trade。
- 模擬 cumulative volume 可能覆蓋正式 volume state。
- 試算五檔可能被保存為一般 matching book。

這不是單純命名問題，而是 domain event 分類錯誤。此問題已於 2026-09-14 修正：normalizer 將帶 `trial` 的盤中 observation 分類為 `IndicativeAuction(IntradayUnclassified)`；其價量與 book 只更新獨立 indicative state，不寫入正式 trade、volume 或 firm book。`trial = false` 的 status-only pause observation 則產生 `MarketStatus`，只更新來源真正提供的 cumulative volume 與 annotations，不因趨勢 bit 合成試算事件，也不把「未揭示」誤作「完整空簿」。TWSE／TPEx reducer tests 與 normalizer tests 覆蓋這兩條路徑。

來源契約已由 repository 外的 immutable partitions 重跑。TWSE revision `44a8468b3056485a0e5009bb11cb04cf63cd7d47d76770344cc9d02277beee67` 含 4,185 records；TPEx revision `d55cba9ce2e2d833f5227993bfd5b0dfdb7ad324d7d584bdefd3e6e120ad649d` 含 834 records。Generic `data verify`、`source_partition.py` integrity 與 `verify_teralion_stability.py` adapter certification 均通過。TWSE 3026 在 09:00:57.720542 出現 trigger、23 筆 realtime trials、09:02:57.764676 正式結果；TPEx 2948 在 09:01:43.199692 trigger、6 筆 realtime trials、09:03:43.292109 正式結果。此處為 cache v2 歷史 checkpoint；目前 v3 rebuild 證據見接手驗收狀態。相同 source 當時成功重建 caches（TWSE `3c496fdd1b0e15df48d1d1ac31861d0e7fd0c4fc872b8bc6b7ba4784147da84b`；TPEx `272f5ed885aa67b716ed492f07615b534fe09c0e805453abfaa9d4a87c0c31eb`）並 replay 3,403 events，event checksum `6ef7cd0bf7bcfb6b7cb976d5cf458308d7cf19416ab1ca65b8337faa576a63ed`、final-state checksum `b16d7f36ded1dd6bc191a78bc510f7f5632be4cd9cebf6f1021b8123a65407d0`；backtest 為 4 orders／4 fills。Raw payload 不 check-in；validation root 受 git ignore，provider-specific certification 不取代 provider-neutral domain／Alpha gates。

### 2.4 `/data` 中既有資料的證據與限制

2026-09-14 盤點絕對路徑 `/data/hschi1106`。舊 Alpha data root
`osmium-delayed-20260623-20260715` 的 equity source partitions 各只含一筆
`DISPOSAL_ARBITRAGE_V3_POINTER`，以 `external_path`／`external_blake3` 指向
`disposal-arbitrage-data-v3` 的 normalized replay JSON。後者已失去 Teralion 原始
`format`、`status_flags`、`limit_flags` 與 page manifest 關聯，不能作為 provider adapter
conformance 的輸入；本次
breaking migration 也不新增 pointer normalizer。

這批 normalized data仍提供有用的 Alpha 行為證據：2026-06-23 的 TWSE 2340 在
09:04:45.838736 觀察到 `stability_delay=up` trigger，09:04:50.889195～
09:06:41.235951 有 23 筆 `trial=true` auction observations，09:06:45.851172 為正式
auction result，09:06:45.878447 恢復 continuous；TPEx 5425 則於 09:00:35.645910
trigger，09:00:40.675174～09:02:31.025634 有 23 筆 trial，09:02:35.840513 正式撮合，
09:02:35.869710 恢復 continuous。兩組 lifecycle都約 120 秒。舊資料只在 trigger 保留
`stability_delay=up`，期間 trial 全標成 `stability_delay=normal`；因此舊 Alpha 必須跨事件保存
pending stability state，不能只看當前 trial 的 direction。這也支持新版將 trial 表達為
`IndicativeAuction` 並以先前明確 trigger關聯方向，而不是把 trial book寫入 firm book。

既有 `osmium-alpha-stability-fix-f9e9aad` run artifact 是舊平台基線：2026-06-23 共
13,418,722 events、4 orders、5 fills，stability sleeve有 2 candidates／2 entries。它可用於
確認舊策略輸出，但因輸入是 pointer＋舊 normalized schema，不能直接與新版 cache做同源
byte-level differential；必須先取得 raw published partitions或建立具明確 provenance 的一次性
source migration artifact。

## 3. 建議的 stability／auction 處理方法

### 3.1 先依單筆 source observation 分類

Normalizer 必須保持 stateless、source-local：只根據目前這筆 wire payload 能證明的內容分類，不使用前後兩分鐘資料補造 lifecycle。

| Source observation | Domain event | Matching／用途 |
| --- | --- | --- |
| `trial = true`，且有 opening window／marker／`delayed_open` 證據 | `IndicativeAuction(Opening)` | `Indicative(PreOpenTrial | DelayedOpen)` |
| `trial = true`，且有 closing window／marker／`delayed_close` 證據 | `IndicativeAuction(Closing)` | `Indicative(PreCloseTrial | DelayedClose)` |
| `trial = true`，且來源契約確認欄位是模擬集合競價 observation | `IndicativeAuction(Opening／Closing／Intraday*)` | `Indicative(...)`；方向只在來源契約確認時附加 |
| `trial = true`，盤中但 Teralion 欄位對 stability 的語意未確認 | 暫以 `IndicativeAuction(IntradayUnclassified)` 隔離價量，不臆測 stability phase | `Indicative(UnclassifiedTrial)`；可由上層與先前明確 trigger 關聯 |
| `trial = false`，trend bit 表示暫緩撮合，且來源未揭示 firm book／trade | `MarketStatus`；只保存 cumulative volume 與 annotations，不清空先前 firm state | `Indicative(VolatilityInterruption*)`／matching disabled |
| stability 期滿後來源明確提供的正式集合競價結果 | 正式 `QuoteSnapshot`／`TradeBatch` | `Enabled(CallAuction)`，可作正式成交與 fill evidence |
| 恢復逐筆後的正式行情 | 正式 `QuoteSnapshot`／`TradeBatch` | `Enabled(Continuous)` |

`status.trial()` 代表來源標示的試算狀態；唯讀真實 `/ticks` 實例確認兩市場都以 `status_flags=128` 揭露 stability-period trial，並觀察到 trigger 的 `instant_trend`、零量 `deal` 與空簿形狀。這些樣本支持 adapter 將明確 status-only trigger 映射為 `MarketStatus`、將 trial 價量/book 隔離為 indicative observation；但不支持單靠 flags 合成 start/end event 或推斷完整 stability lifecycle。Raw records 保留在 repository 外 immutable partitions，由 Teralion adapter conformance 驗證；通用 lifecycle gate 只接受已分類的 provider-neutral observations。

### 3.2 統一 event variant，但保留 typed classification

將目前：

```text
IndicativeOpeningAuction(IndicativeAuction)
IndicativeClosingAuction(IndicativeAuction)
```

改為單一：

```text
IndicativeAuction(IndicativeAuction)
```

payload 至少包含：

```rust
pub enum IndicativeAuctionKind {
    Opening,
    Closing,
    IntradayStability { direction: StabilityDirection },
    IntradayUnclassified,
}

pub enum StabilityDirection {
    Down,
    Up,
}

pub struct IndicativeAuction {
    kind: IndicativeAuctionKind,
    price: Observation<Price>,
    quantity: Observation<Quantity>,
    book: Observation<CompleteBookSnapshot>,
    cumulative_volume: Observation<Volume>,
    annotations: MarketAnnotations,
}
```

`kind` 表示 observation 所屬的試算階段；只有 `IntradayStability` 能攜帶 stability direction，避免形成 `Opening + StabilityDirection` 等非法組合。`TradingContext` 的 `IndicativeReason` 則表示目前為何不能正常撮合。event classification 與 matching reason 是不同維度，不能只留下不帶原因的 `IndicativeAuction`。

`IntradayStability` 只在這一筆 observation 有足夠 source evidence 時使用。後續 trial 若 direction bits 已回到 normal，stateless normalizer 應輸出 `IntradayUnclassified`；strategy／runner 可以依先前明確 trigger 建立 pending session並關聯，但不得改寫該筆原始 classification。

替代方案是保留 Opening／Closing／Stability 三個 event variants。較推薦單一 variant + typed kind，因欄位和 reducer semantics 相同，也可避免 `event.rs`、canonical codec、reducer 與 strategy view 重複分支。

### 3.3 趨勢 flag 不等同 lifecycle

目前 `InstantTrend::VolatilityInterruptionUp/Down` 把「暫緩撮合且瞬間趨漲／趨跌」直接命名成 volatility interruption，仍可能讓下游誤認為完整 stability auction 或 lifecycle。官方 bit contract 已核對方向，唯讀樣本亦觀察到 `limit_flags=2` 的 Teralion trigger；但缺少可重現 source golden，兩方向與各 record family 的 source contract 尚未完成回歸驗證。候選較中性的名稱：

```rust
pub enum IndicativePriceTrend {
    Normal,
    Down,
    Up,
    Reserved,
}
```

即使保留 `StabilityDirection` 作為 strategy-facing trigger reason，也必須標明它來自「暫緩撮合方向」而非 trial auction classification。無論採何名稱，都不得只由一個 bit 合成 `StabilityStarted`／`StabilityEnded`。

若同一 active stability window 內重複出現同方向 pulse，不應把預計撮合時間往後延；若方向在 window 內改變，應保存 anomaly／最新 observation，但在取得交易所欄位契約前不得擅自把它當成新一輪 stability。

### 3.4 Firm 與 indicative state 必須分離

Indicative event 應更新獨立的 indicative state，不得更新：

- `last_trade`
- 正式 `recent_trade_batch`
- 正式 cumulative volume
- 一般 executable／matching book

Indicative state 可保存：

- indicative price／quantity
- indicative full book
- indicative cumulative-volume observation（只能作來源稽核，不是正式 volume）
- raw annotations
- event identity／`match_time`
- `IndicativeAuctionKind`
- observation-local stability direction

收到已確認的正式 auction result、恢復 continuous 的正式事件、session boundary 或 trading-date boundary 時，依明確 reducer policy clear／supersede indicative state。清除動作不得修改最後一筆正式 book／trade／volume。

實作進度（2026-09-14）：reducer 在正常 matching 已恢復的 firm event 上將 indicative 欄位標為 `Unavailable::Cleared`，並記錄清除 event reference；TWSE／TPEx 的 trial、stability pause 或 reserved flags 不會觸發清除。`Carry` session boundary 也會清除 indicative 欄位，`ResetObservableFields` 維持整組 observable reset。正式 firm book／trade／volume 不受影響。`MARKET_STATE_VERSION` 升為 5、`STATE_REDUCER_VERSION` 升為 4；測試涵蓋 resumed matching、pause 不清除與 carry boundary 清除。

### 3.5 TradingContext、execution 與 lifecycle

`TradingContext` 應把所有 indicative events 映射為 typed `MatchingState::Indicative(reason)`，並保留：

- opening／closing／intraday classification。
- delayed open／delayed close。
- stability direction（若來源可證明）。
- unclassified trial，不得 fallback 成 enabled matching。

只有來源確認的正式 stability auction result 才可成為 call-auction trade evidence。Execution simulator 必須拒絕用 indicative price、book 或 cumulative volume 填一般 order。

平台不從固定 120 秒自行補出虛構 start/end domain events。若市場規則需要提供 expected match time，應由 versioned session／trading policy 計算並標示為 expectation，而不是偽裝成已觀察到的 source event；實際結束仍以正式來源 observation 或 session boundary 為準。

## 4. `osmium-alpha` 對應修正計畫

### 4.1 相容性與版本策略

本次變更是刻意的破壞性 migration。為維持 codebase 精簡以及 market semantics 正確，不保留：

- `IndicativeOpeningAuction`／`IndicativeClosingAuction` 舊 variants。
- 接受 trial `QuoteSnapshot` 的舊 strategy adapter workaround。
- deprecated type aliases、feature flags 或新舊 API 同時存在的 compatibility shim。
- 舊 canonical event decoder、舊 cache dual-read 或 cache 就地轉寫。
- 讓 alpha 同時支援兩個 `osmium-lab` revisions 的條件編譯。

`osmium-alpha` 目前以完整 commit pin 依賴 `osmium-lab`。在 dependency revision 未更新前不受新 schema 影響；migration 時必須在一個 commit-sized change 內更新 pin、修正所有 exhaustive matches、更新 strategy tests 與重建 fixtures。編譯錯誤是預期的 migration gate，完成前不允許執行或比較新回測。

歷史 checkpoint（正式 pin 前；最新結果見 §4.6）：alpha 已改用 typed indicative observation，分離 firm／trial book，並調整 pending stability session；同時將受 `Price::new` 語意變更影響的正價計算改用 `Price::positive`。以隔離副本將六個 lab 套件 path override 至目前工作樹後，alpha workspace 測試通過。alpha 的實際 git pin 尚未變更，此處為提交前的歷史 checkpoint；不可把本地 path override 當成最終 dependency migration。

正式 pin gate 的歷史 checkpoint：直接以 Alpha 工作樹的舊 revision 測試會因缺少 `IndicativeAuctionKind`／`StabilityDirection`、`EventPayload::IndicativeAuction` 與 `Price::positive` 等 API 而編譯失敗，符合預期中的 breaking migration。2026-09-14 以隔離副本指向目前 lab 工作樹後，新增 Alpha delayed-open regression 的完整 workspace 測試通過；必須等 lab revision 可 pin，再更新正式 dependency 並重跑不使用 path override 的 workspace 測試。

同日新增 `strategy-api` runner 整合測試：以帶有 trial 與 delayed-open flags 的 TWSE `STOCK_REALTIME` JSON，實際經 `TwseNormalizer` 產生 domain event 再執行 `run_strategy`；測試確認 strategy callback 可透過 `TradingContext::matching()` 觀察 `Indicative(DelayedOpen)` 並輸出對應 indicator。這驗證來源 mapping 到 lab callback 的訊號可見性；不取代 `osmium-alpha` 專屬策略行為／order-intent migration 測試，也不代表 alpha dependency pin 已更新。

舊的 verified source data仍可重用，但所有 derived replay cache 必須以新 normalizer／event schema重建。讀到舊 cache 應明確拒絕並回報版本不相容，不嘗試 fallback。

若修正只改 Rust representation，且 strategy-visible semantics 經 differential tests 證明完全相同，可以維持 strategy identity；只要 order intent、fill、PnL 或 eligibility 有任何變化，就將 opening／stability／portfolio strategy version 升版，保留舊 run 的可稽核性。

event／canonical／cache schema 即使經濟結果相同也必須升版並重建 cache；不能拿 fingerprint 或 checksum 的改變判定為策略 regression。

### 4.2 Strategy API migration

實作進度（2026-09-14）：`StrategyEventContext::market_states()` 改回傳 allocation-free `ReplayStateViews`，直接唯讀遍歷 `ReplayCore` 的 deterministic instrument map；一般、multi-market、scheduled 與 example-strategy callback 都已改用此 view，不再每筆 event 建立 universe `Vec`。移除舊 slice-based constructor、不保留 compatibility adapter，`STRATEGY_API_VERSION` 升為 3。Workspace 測試 282 項通過；full multi-backtest benchmark 的 event/final-state checksum、fill 數與 output 數不變，效能差異僅列初步觀察。

接續升為 `STRATEGY_API_VERSION` v4：新增 `CancellationReason::VolatilityInterruption`，並由 TWSE／TPEx trading context 將暫緩撮合標記為 restricted indicative state。Alpha 可在每次 `on_event` 透過 `context.trading().matching()` 讀到 `Indicative(VolatilityInterruptionDown | VolatilityInterruptionUp)`；盤前延後開盤則讀到 `Indicative(DelayedOpen)`。這是行情事件驅動的狀態，不是平台另行合成的 start/end 通知；沒有來源事件時不會觸發 callback。普通 `OrderIntent` 只有 ROD（Day），所以本次只落實：stability pause 中拒絕新 market ROD、保留 limit ROD，並取消既有未完成 market ROD。`EXECUTION_SIM_VERSION` 升為 v3。Teralion mapping 與完整 lifecycle 已由外部 verified partitions 通過；provider-neutral Alpha 舊／新 semantic differential 與正式 dependency pin 已另外通過，見 §4.6；adapter certification 不取代策略結果驗證。

scheduled execution 也已補上同一 order-entry 規則：activation 使用最新已可見 `NewOrderEntry` 驗證，pause 中的 market request 以 `NewOrderEntryBlocked` 結束；收到 volatility-interruption observation 時只取消已 active／partially-filled／match-attempted 的 market ROD，feedback 透過 runner control queue 交付。尚未 activation 的 scheduled request 是未送出的 intent，不因交易所 trigger 取消。Simulator regression 覆蓋已啟動單取消及 pause 中 activation 拒絕；runner-level regression 以「active order → stability observation → feedback callback」序列確認沒有 fill、order 取消且策略收到 cancellation feedback。

優先讓 alpha 依賴穩定的 strategy-facing adapter，而不是直接 match 所有 raw `EventPayload` variants。`EquityIndicativeObservation` 應隨新 schema 改成至少提供：

```text
kind()
reason() 或 context.trading().matching()
price()
quantity()
book()
annotations()
```

alpha 對應 mapping：

```text
Opening + PreOpenTrial       -> normal opening candidate
Opening + DelayedOpen        -> delayed-opening candidate
Closing + PreCloseTrial      -> normal closing candidate
Closing + DelayedClose       -> delayed-closing candidate
MatchingState::Indicative(VolatilityInterruption*) -> 建立／更新 pending stability trigger
IntradayStability + direction -> 只有 source contract 證實是 trial auction 時才作 trial observation
IntradayUnclassified         -> 只有已存在相容 pending stability session 才接受為 trial
```

alpha 必須把兩條資料路徑分開：`TradingContext.matching()` 的暫緩撮合 reason 用來知道 matching 已受限並建立 trigger session；`IndicativeAuction` 的 typed kind／價量則表示一筆可用於策略判斷的試算 observation。不能單憑 trigger direction 就把同一 event 的 price／quantity／book 當成 trial auction data。

移除把 trial `QuoteSnapshot` 當成 `EquityIndicativeObservation::Unclassified` 的舊 workaround；新 schema 下任何 `status.trial()` observation 都必須在 platform boundary 先成為 indicative event。

### 4.3 分離 alpha 的 firm book 與 trial observation

目前 `ArbitrageEngine::remember_book()` 對所有 `BookSnapshot`／`QuoteSnapshot` 都更新 `visible_books`，因此被誤分類的 stability trial 會覆蓋股票正式 book。後續由股期 continuous event 觸發 exit 時，可能使用這份股票試算 book。

修正方法：

- `visible_books` 只保存 `MatchingState::Enabled(...)` 下的 firm book。
- `IndicativeAuction::book` 若策略需要，存到獨立的 `visible_indicative_books`／trial observation，不得供 continuous exit、mark、融資維持率或一般 fill 使用。
- stability entry 現行只需要 trial price／quantity 與股期 firm depth；不要為了方便把股票 trial book塞回 firm map。
- `remember_formal_auction_visibility()` 只接受 `Enabled(CallAuction)` 且有正式 clearing evidence 的事件。

這是 alpha 的必要防禦性修正；根因仍是 platform 把 trial 表示成 `QuoteSnapshot`，但 strategy 不應再依賴錯誤 representation。

### 4.4 Pending stability session

保留 alpha 現有「明確 trigger + 後續 trial correlation」概念，但修正狀態機：

1. 只有 typed `Stability(Up|Down)` trigger 才建立 pending session；不得從價格跳動、缺 trade 或時間窗口猜測。
2. `trigger_time` 使用原始 event `match_time`，`expected_match_time` 初期維持現行 `trigger + 120 秒` policy。
3. active window 內重複 pulse 不延長 120 秒；反方向 pulse 記錄 anomaly，在 source contract 未確認前不重啟 session。
4. `IntradayUnclassified` trial 只有在同商品、同 trading date、`trigger_time <= match_time < expected_match_time` 且沒有 opening／closing reason 時才串入。
5. 正式 call-auction result、session boundary、trading-date boundary或 timeout 結束 pending session；timeout 是 strategy policy，不得回寫成來源 `StabilityEnded` event。
6. 新一輪 stability 必須在上一輪結束後由新的明確 trigger 開始。

長期可把「延後開盤固定 09:02」與「stability trigger + 120 秒」移到 versioned market timing policy，由 `TradingContext` 提供 expected match time；alpha 只消費結果。第一階段 migration 先維持原時間算法，避免把 schema 修正和策略規則改動混在同一 commit。

### 4.5 Delayed open 不得與 stability 混淆

延後開盤必須由 `Opening + DelayedOpen` 表示；stability 必須由 `Intraday* + Stability(direction)` 或 pending stability correlation 表示。不能只因兩者都是兩分鐘就共用 signal identity。

alpha 目前以 09:00 加 120 秒得到 09:02；這是策略的 expected-time policy，不是 `DelayedOpen` source flag 保證的交易所實際撮合時間。平台只在該商品的行情 callback 中提供 `Opening + DelayedOpen` matching reason，不另發全市場 delay notification，也不提供確切的 rescheduled match timestamp。Alpha 若在該 expected decision time 之後才看到 trial，現行 `trial_from_event` 會以 `decision_time_already_visible` 拒絕，不會自動把 09:02 timer 往後延。migration 需保留此界線，並明確決定「較晚 trial 是否只能拒絕」或「是否引入有來源依據、版本化的 expected-time policy」；不可把逾時候選偷偷改成新時間或由沉默行情推測。驗收已新增：明確檢查 09:00／09:02 expectation，並測 delayed-open trial 在 decision time 前與後才可見時分別排 timer／拒絕。

migration 後應先保持既有策略行為，並增加以下拒絕條件：

- `IntradayUnclassified` 沒有 pending stability 時，不建立 opening 或 stability candidate。
- `Opening` 沒有 `DelayedOpen` reason 時，不自行延後。
- 09:00 沒看到正式成交不等於 `DelayedOpen`。
- stability trigger 不得重設 opening timer。

實作檢查（2026-09-14）：計畫先前記錄的 TPEx precedence 疑慮已依現況重驗；normalizer 會優先處理 delayed flags，`evaluate_annotated_matching()` 也在 generic opening/closing trial 分類前處理 delayed flags，因此目前程式分支不會因 marker／時間窗口覆蓋明確 delayed reason。strategy-api 測試覆蓋 TPEx `0xC8`（trial + delayed-open + opening marker）；TWSE／TPEx normalizer 現新增 `0xC8` 與 `0xC0`（trial + delayed-open、無 opening marker）案例，確認產生 `IndicativeAuction(Opening)` 並保留 delayed flag。TWSE B.12.00 與 TPEx V.12.18 相關 bit 定義已核對；instant-trend 是暫緩撮合方向，TWSE raw wire 對暫緩撮合揭示最近成交價／零成交量／不揭示買賣價量。這不代表 Teralion JSON mapping 已驗證：仍缺兩市場真實 stability records，故人工 bit tests 不能證明 stability trial classification 正確。

Alpha 的 `fixed_auction_times_use_taipei_and_delayed_extension` 現明確斷言一般開盤 expectation 是 09:00、`DelayedOpen` expectation 是 09:02，而非只斷言兩者相差 120 秒；另將過期檢查抽為 `schedule_trial`，測試候選在 decision time 前一微秒可排 timer、後一微秒會被拒絕且不留 pending trial。`interleaved_futures_exit_uses_stock_firm_book_not_stability_trial_book` 依 firm stock → stability indicative stock → firm futures 順序更新可見簿，直接呼叫 production `try_continuous_exits` 驗證安全路徑不會發出 exit order；污染對照則證明若把 trial 簿誤寫入 firm map，同一 futures event 會錯誤產生兩筆 exit orders。隔離 path-override 副本的 `cargo fmt --all --check` 與 `cargo test --workspace` 通過（arbitrage-core 50 tests；workspace 共 52 tests）；這仍不是正式 dependency pin 或真實行情 differential evidence。

同一隔離 Alpha workspace 的 `cargo clippy --workspace --all-targets -- -D warnings` 亦通過。當時的舊 Alpha dependency pin 會因缺少新 `market-types` API 而無法直接編譯；以上 Clippy／測試只證明 path override 指向目前 lab worktree 時的遷移結果。

### 4.6 Cross-repo differential tests

建立同一組 sanitized source sequence，同時餵給舊版與 vNext semantic adapter：

```text
firm stock quote
-> stability Up/Down trigger
-> stability trial
-> interleaved futures continuous quote
-> another stability trial with normal direction bits
-> formal stability call-auction result
-> continuous stock quote
```

經濟語意應保持：

- trigger time／direction相同。
- expected match time相同。
- trial price／quantity相同。
- entry candidate、decision timer與 order intents相同。
- 未碰到舊污染路徑時，fills／fees／PnL相同。

刻意要求不同的部分：

- trial 不再覆蓋 firm stock book／recent trade／cumulative volume。
- interleaved futures event 不得用股票 trial book建立 continuous exit。
- event fingerprint、cache checksum與 final-state checksum依新版本重算。

另建立 delayed-open sequence，驗證 `DelayedOpen` 仍得到 09:02 expectation，且不會被分類為 stability。

**正式驗收完成（2026-09-14）**：Alpha `scripts/semantic-differential/run-matrix.py` 以同一份 provider-neutral fixture 分別驅動舊 Alpha `96ba9c8c8834f45ccf74a06a6b1e35e55ec0862a`／舊 Lab `e8f2e2e8700649583e6946953220aa33ffe1c865` 與新版 production `ReplayCore`、`TradingContext`、`on_event`／`on_timer`。TWSE Up、TPEx Up、synthetic Down 三案例全部通過；每案也執行 delayed-open callback，斷言 09:02 expectation、09:01:59 timer 且不建立 stability pending。

- TWSE／TPEx observations 由 verified source 經 `export_teralion_semantic_fixture.py` 匯出；完整五檔、trade absence、exact decimal、累計量與 market-order quantity 均保留，數量縮放、symbol/time transforms 與 synthetic futures supplements 明列 provenance。TWSE 23 trials、TPEx 6 trials；Down 的 2 trials 是獨立 synthetic contract。
- 每案產生兩筆完整 entry intents；舊／新 trigger、方向、expected match、全部 trial、timer、formal reset 與 resume 一致。僅 trigger/trial firm-book isolation 與舊 API 無法表示的 market-order quantity 列為預期語意差異。TWSE 新版實際保存 `Some(31200)` 市價買量，不以相容 shim 模擬。
- Production simulator／ledger component 對照每案皆有非零 fills。另以明確標示的 `synthetic_clean_fill`（formal price 改為 109）執行完整 `run_scheduled_multi_backtest`，使雙腿成交、feedback 回到策略並完成 ledger／finalize。每案舊／新版皆 2 fills；fills、fee/tax、fee config、realized／unrealized PnL、cash、positions 與 final equity 精確一致。此合成成交案例不代表原始行情必然成交或真實完整日策略績效。
- 完整 runner 每版本再跑一次並要求 trace byte-identical。跨 schema event／final-state checksums 不要求相同；比較器只排除這兩個 versioned fields，且拒絕重複、缺少、空白 trace 欄位。
- 重跑方式：在 Alpha 執行 `python3 scripts/semantic-differential/run-matrix.py --lab /path/to/osmium-lab --trace-dir target/semantic-differential/traces`。本次 exit 0 的 log 為 Alpha `target/semantic-differential/logs/final-matrix.log`，18 個 traces 位於對應 `traces/`。正式 dependency pin 與 workspace checks 見接手驗收狀態。


## 5. `market-types` 其他修正計畫

### MT-001：價格模型支援 signed 與 zero market price

目前 `Price` 強制大於零，但 TAIFEX 官方規則明確指出 calendar spread 報價是遠月減近月，可為負、零或正數。不能再把「零一律是 absent」當成通用市場價格規則：只有 wire contract 明確定義的特定欄位／record 才能將零解讀為 sentinel；其他情況要保留零價。canonical price 本來就用 signed `i128` atoms，限制只存在 constructor。TAIFEX 也說 calendar spread 不參與開盤集合競價，且只在盤中逐筆交易時段收單，應使用明確商品形狀與 session profile，而非當成一般 outright future。

建議：

- 將通用 `Price` 改為可精確表示 signed decimal（含零）；將缺值留給 `Observation`，不以 `Price` constructor 代表缺值。
- 由明確的 instrument contract shape／market profile 驗證 positive-only 商品，並允許 futures calendar spread 使用 signed／zero price；normalizer 必須依 source format 的 wire contract 解讀零 sentinel。
- 在 fee／tax basis 為 notional rate 時，確認 signed spread notional 使用絕對值或明確拒絕該 charge model，避免負價格生成負手續費；交易所 per-contract fee 應使用固定每單位 basis。
- audit book ordering、limit order comparison、slippage、notional、marking 與 accounting 對負價格的假設。
- 新增 negative／zero trade、negative book、format-specific zero sentinel、positive-only profile 與 charge/accounting tests。

注意：不能只放寬 `Price::new()` 就結束，否則 equity 也會接受負價，且 simulation/accounting 可能產生方向錯誤。

完成（2026-09-14）：`Price` 精確表示 signed／zero decimal；TWSE／TPEx normalizer 對正價行情使用 positive parser，TAIFEX normalizer 依明確的 instrument profile 區分 spread 與 outright，並保留 I022 的零價／零量 sentinel 規則。`PricePolicy` 從 contract shape 傳入一般／scheduled simulator 與 multi-ledger，equity 預設 positive-only、calendar spread 使用 signed。帳務 notional-rate charge 以絕對名目金額計算，fixed-per-unit charge 不受價格正負影響；signed fill／mark、零價 spread fill、spread round trip、positive-only equity rejection、負價五檔與 limit eligibility 均有測試。新增 multi-ledger 負價／零價完整持倉、mark、平倉與 reconcile 測試。TAIFEX 具體 source 的 zero sentinel 仍只適用已在 interface contract 標明的 I022 欄位，不能泛化到其他 record format。

### MT-002：釐清 instrument taxonomy，移除 `Unknown` 的隱性 fallback

目前 `InstrumentKind` 同時被用於：

- normalizer 選擇。
- archive market 選擇。
- session profile 選擇。
- accounting model 選擇。

問題：

- `Unknown` 在部分路徑會由 market 推定為 equity／future，accounting dispatch 甚至與 equity 共用 branch；這與「缺值不得自行推定」衝突。
- 只用 Equity／Warrant／Future／Option 無法表達 outright future、calendar spread、block market 等會影響價格與 session 規則的能力差異。
- option／warrant reference 欄位目前集中在 `osmium-config`，不是可版本化的 domain instrument contract。

建議先拆成兩個概念：

```text
InstrumentClass：Equity／Warrant／Future／Option
InstrumentCapabilities 或 ContractShape：正價限制、signed spread price、session profile、archive market、accounting model所需能力
```

- 缺少 class 時保留 unknown metadata，但任何需要 economics／normalizer／session 的 execution plan 必須拒絕，不得套 default。
- 不以 symbol hard-code 推定 TAIFEX session profile；設定必須明確選擇適用 profile。
- instrument reference identity 應保存可用的 underlying、expiry、strike、option side、currency、multiplier 與 provenance，但不要求不適用的商品填入假欄位。

完成（2026-09-14）：執行設定要求明確 class；TAIFEX future 要求 contract shape 與 session profile，RunPlanner 不依 symbol 推定。`InstrumentReferenceConfig` 已納入 versioned `InstrumentContractConfig`，包含 underlying、expiry、strike、option side、currency、multiplier、quantity unit／trading-unit size 與 provenance；effective config version 升為 v6，reference 全欄位進入 canonical identity。Option／warrant contract 缺 reference、reference 欄位無效或 reference economics 與 execution economics 不一致時會拒絕。測試確認 option reference 改變會改變 checksum，且缺少 reference 被拒絕。`InstrumentClass` 以明確 enum 表示，不再有 `Unknown` execution fallback。

### MT-003：calendar spread symbol 與 storage path 分離

`Symbol` 保留 byte-exact UTF-8 是正確的；Teralion 說明 calendar spread symbol 包含 `/`。目前 storage 的 `safe_component()` 拒絕 `/`，導致合法 symbol 無法成為 partition path。

建議：

- 不在 `Symbol` trim、替換或禁止 `/`。
- storage 使用 reversible、collision-free path encoding，或以 instrument identity hash 作 directory component。
- manifest 永遠保存原始 symbol，path encoding 不成為 domain identity。
- 加入 `/`、`%`、Unicode、`.`、`..` 與 collision tests。

完成（2026-09-14）：source partition 與 derived cache layout 以 reversible percent-encoding 將 symbol 轉成 path-safe component，過長 symbol 再切成固定長度的 ASCII components；manifest 和 partition identity 保留原始 symbol。測試涵蓋 calendar spread `/`、literal `%`、Unicode、`.`／`..`、反斜線、collision 與長 symbol；另直接驗證 TAIFEX `TXFH6/TXFM6` 的 source/cache roots 不會把 `/` 當成目錄分隔符，且 manifest round-trip 保持原 symbol。

此項實作位於 `data-sync`，但由 `market-types::Symbol` 的合法 identity 範圍驅動，因此列入本計畫的 downstream migration。

### MT-004：`TradePrint` 與 `TradeOrder` 重新命名

`TradePrint` 容易與 simulation fill 或 print 動作混淆；`TradeOrder` 更容易被誤讀為買賣委託。

建議名稱：

```text
TradePrint       -> MarketTrade 或 ObservedTrade
TradePrintKind   -> MarketTradeRole 或 TradeObservationKind
TradeOrder       -> TradeBatchOrdering
SourceOrdered    -> SourceSequencePreserved
Unspecified      -> Unspecified
```

選名原則：

- 明確區分歷史市場成交與模擬 `FillRecord`。
- batch ordering 名稱不得像 order intent。
- `Intermediate` 必須表達它是同一 source observation 中的中間正式成交，不是 partial fill 或 indicative price。

這是 public API 與 canonical enum identity 變更，應與 event schema migration 一起進行，或只做 Rust symbol rename、保持 canonical discriminants 不變。

實作狀態：已將 `TradePrint`／`TradePrintKind`／`TradeOrder` 改為 `ObservedTrade`／
`TradeObservationKind`／`TradeBatchOrdering`，並將 `print_kind()` 改為 `observation_kind()`、
`SourceOrdered` 改為 `SourceSequencePreserved`。enum numeric discriminants 維持 0／1，沒有額外改動 canonical bytes；不保留舊名稱 alias。

### MT-005：`time.rs` 改名為 `match_time.rs`

- module 實際只定義 `MatchTime`，不是一般 time utilities。
- 使用 `git mv` 保留歷史；`lib.rs` 仍可 `pub use match_time::{...}`，外部 type path 不需改變。
- 不與 `MatchTime` overflow 修正混在同一 commit，以保持 mechanical rename 可單獨 review。

完成（2026-09-14）：實作檔已改名為 `match_time.rs`，`lib.rs` 保持相同的 root-level public re-export；public API tests 分離到 `tests/match_time.rs`。核心 commit `964831b` 已將此變更辨識為 rename。

### MT-006：修正 `MatchTime` formatting boundary

`MatchTime` 內部仍以 `i64` microseconds 表示，可容納超出四位數 ISO 日期的值；格式化必須明確回傳錯誤，不能 overflow 或 panic。canonical event 的 encode/decode 必須使用相同可格式化範圍，避免產生無法 round-trip 的 event。

建議：

- 使用 typed `UtcOffsetMinutes`，constructor 驗證支援範圍。
- `to_iso8601` 回傳 `Result<String, MatchTimeFormatError>`，或限制 `MatchTime` constructor 只建立 formatter 可處理的範圍。
- canonical decoder 同樣驗證可接受範圍。
- 補 `i64::MIN/MAX`、跨日、負 epoch、最大合法 offset 與非法 offset tests。

完成（2026-09-14）：`UtcOffsetMinutes` 驗證 ISO offset 範圍，`to_iso8601` 使用 checked arithmetic 並回傳 `MatchTimeFormatError`；canonical event 現在在 encode/decode 兩端都拒絕無法以 UTC 呈現為四位數 ISO 日期的 timestamp，market-types version 升為 5 以使舊 cache identity 失效。測試涵蓋 offset 邊界、i64 邊界、canonical encode/decode 對稱性與 round trip。

### MT-007：統一 `Decimal` exact text formatting

`Decimal` 已實作 exact `Display`；CLI/TUI 價格文字不應再各自格式化 atoms。圖表座標使用 `f64` 是繪圖 library 的 presentation boundary，不得回流至行情、下單或帳務計算。

建議：

- 實作 exact `Display` 或明確的 `to_decimal_string()`。
- 固定負零、trailing zeros、最小 atom、`i128::MIN/MAX` 行為。
- UI 若要限制小數位，使用另外命名的 presentation formatter，不能影響 canonical identity。
- 移除文字價格顯示的 `f64` conversion；若 UI chart library 需要浮點座標，轉換只能留在繪圖邊界。

完成（2026-09-14）：`Decimal::Display` 已覆蓋 zero、最小 atom、負值、trailing zero 與 `i128::MIN/MAX`，並 round-trip 測試。market replay TUI 的價格文字現在透過 `Price::as_decimal()` 使用 canonical `Decimal` display，移除第二份 atom formatter；圖表的 `f64` 僅用於 ratatui chart coordinates／axis，保留為明確 presentation conversion。

### MT-008：限制 canonical decoder 的 hostile allocation

`TradeBatch` decoder 已依剩餘 frame bytes 與每筆 trade 最小編碼長度限制 `Vec::with_capacity(count)`，並有 oversized count／truncated vector tests。cache reader 原先仍會直接依未信任的 u32 record length 配置整筆 buffer，故單筆 canonical event frame 也需要上限。

建議：

- 以 remaining frame bytes／每筆 trade 最小 bytes 計算安全上限。
- decoder 接受全域 frame size limit 或由 cache reader在進入 decoder 前限制 record length。
- oversized count、truncated vector、trailing bytes 與 fuzz/property tests。

完成（2026-09-14）：derived cache 單筆 event record 設為 16 MiB 上限，writer 與 reader 兩側都檢查；reader 在配置 record buffer 前拒絕超限長度。cache format 升為 v2，避免舊 cache identity 被當成新格式直接沿用。canonical decoder 覆蓋 trailing bytes、oversized count、truncated vector，並對四種 event payload 執行逐 byte／bit deterministic mutation property 測試（不 panic；成功 decode 必須 re-encode 為相同 canonical bytes）。另有 cache oversized record 損壞檔案端到端測試。此處以確定性 mutation property tests 作為 fuzz/property 驗證，不另引入 coverage-guided fuzz infrastructure。

### MT-009：統一 `TradingDate` 的可建構與可格式化範圍

目前 text parser 只接受四位數年份，但 `from_epoch_days(i32)` 與 canonical decoder 接受任意值。

建議先決定 contract：

- 若產品只支援 `0000-01-01..9999-12-31`，建立 checked constructor 並讓 decoder 拒絕範圍外值。
- 若完整 `i32` epoch day 都合法，則 parser／formatter 必須定義 signed extended-year round trip。

市場資料平台建議採前者，因 source query、YAML、manifest 與 Teralion API 都使用四位數日期。

完成（2026-09-14）：`TradingDate::from_epoch_days` 與文字 parser 共用 `0000-01-01..=9999-12-31` 範圍，`Display` 不再輸出 parser 無法 round-trip 的 signed extended year；canonical decoder 經 checked constructor 拒絕範圍外 epoch day。測試涵蓋最小／最大合法日期、相鄰越界值、`i32::MIN/MAX` canonical payload，以及 parse／display round trip。

### MT-010：canonical encoding hot path 避免重複配置

目前 `DomainEvent::fingerprint()` 每次先建立新的 `Vec<u8>`；ordering、normalizer sorting、cache writing 與 replay checksum 可能對同一 event重複 encode／hash。這可能是回測效能問題之一，但需 benchmark 證明占比。

候選設計：

- `encode_canonical_into(&mut Vec<u8>)` 允許 caller 重用 buffer。
- canonical encoder 可寫入抽象 sink／hasher，fingerprint 不必 materialize完整 `Vec`。
- 在 normalize／sort boundary 預計算 ordering key／fingerprint，避免 comparator 反覆 hash。
- 不在 `DomainEvent` 內貿然加入 interior cache，避免改變 equality、clone cost、thread semantics 與 event size。

優化前後必須比較 event checksum 與 final-state checksum完全一致。

實作進度（2026-09-14）：`ReplayCore` 的多 stream merge 現在會在每個 stream head 預先生成 canonical frame 與 ordering key；事件套用時重用該 frame 作 replay checksum，避免排序鍵與 checksum 各自重複 encode/hash。同時 TAIFEX normalizer sort 使用 cached sort key，避免 comparator 每次比較都重新 fingerprint。

基準程式：

- `crates/replay-engine/benches/canonical_hot_path.rs`：執行 `cargo bench -p replay-engine --bench canonical_hot_path`。固定建構五檔 TWSE quote 和一組 50,000 個 match-time／sequence 遞增的同 payload events，依版本印 event schema、canonical event、ordering rule、target。canonical 子測試比較舊路徑（merge key、apply key、checksum 各自 canonical encode）與單次 encode/hash，先斷言 fingerprint 與 canonical bytes 相同；最近一次結果是基線中位數 201,073,817 ns、新路徑 94,204,274 ns，每組 250,000 次。ReplayCore 子測試使用相同固定 stream，基線在 stream read 時重做舊路徑的 merge fingerprint 與 checksum frame 工作，新路徑不重做；兩者 event checksum 與 final-state checksum 必須相同。最近一次 Linux x86_64／rustc 1.97.1 結果是基線中位數 100,939,069 ns（495,348 events/s）、新路徑 78,820,191 ns（634,355 events/s），約 1.28x。
- `crates/data-sync/benches/verified_cache_pipeline.rs`：執行 `cargo bench -p data-sync --bench verified_cache_pipeline`。從 repository-owned 五筆 synthetic TWSE fixture 複製一筆 regular quote 並合成 50,000 筆遞增時間序列，經 staging／source verification 建立 verified source；計時 cache build（source verify、decompress、normalize/sort、canonical encode 與 cache write）、cache reader 全量 scan，以及 `CacheReader` 餵給 `ReplayCore`。Linux x86_64／rustc 1.97.1 最近一輪 cache prepare 為 0.895 s（55,860 records/s）；cache scan 五輪中位數 0.030 s（約 1.67M records/s）；cache-backed replay 五輪中位數 0.089 s（約 562k events/s），重複 replay 的 event checksum 與 final-state checksum 相同。這是當前 pipeline throughput，不是優化前後比較；不含 strategy callback、execution/accounting 或 peak-memory measurement。synthetic fixture 非真實行情，不能據此宣稱完整真實回測表現或加速率。
- `crates/replay-engine/benches/multi_stream_merge.rs`：執行 `cargo bench -p replay-engine --bench multi_stream_merge`。以相同 49,152 筆 synthetic five-level quote 分別分布於 1／8／32 個 TWSE instrument streams，透過 `ReplayPlan::new_multi` 與 `replay_frozen_multi` 計時三輪中位數。最近一輪 Linux x86_64／rustc 1.97.1 結果為 79.3／84.7／90.4 ms（約 620k／581k／544k events/s）；各 stream-count case 重複 replay 的 event 與 final-state checksum 相同。資料 stream 是記憶體內複製，計時不包含資料解碼、strategy callback、execution/accounting，也未量 peak memory；此 benchmark 用於觀察 merge fan-in 成本，不代表完整多商品回測表現。
- `crates/osmium-runner/benches/full_multi_backtest.rs`：執行 `cargo bench -p osmium-runner --bench full_multi_backtest`。8 個 TWSE streams 共 49,152 筆 synthetic quote，經 `run_multi_backtest` 完整執行 `ReplayCore` merge、`AcceptanceStrategy` callback／output、order simulation、equity ledger、finalize 與 reconciliation。Linux x86_64／rustc 1.97.1 三輪中位數 140.6 ms（約 350k events/s），每輪 16 fills 與 147,458 strategy output records；event 與 final-state checksums、fill／output counts 在重跑間一致。stream 在記憶體中複製，benchmark 不包含 source decode、normalization、cache I/O 與 artifact publication，也未量 peak memory；strategy 僅為 deterministic acceptance workload，不是真實 Alpha 策略。仍需真實完整 source day 與 peak-memory measurement，才可判斷生產回測的瓶頸及效益。
- `crates/osmium-runner/benches/full_multi_backtest.rs`：執行 `cargo bench -p osmium-runner --bench full_multi_backtest`。8 個 TWSE streams 共 49,152 筆 synthetic quote，經 `run_multi_backtest` 完整執行 `ReplayCore` merge、`AcceptanceStrategy` callback／output、order simulation、equity ledger、finalize 與 reconciliation。優化前單次三輪中位數 139.1 ms（約 353k events/s）；移除每筆 event 建立 universe view `Vec` 後，三次三輪中位數為 135.9／137.1／134.4 ms（約 362k／359k／366k events/s），每輪 16 fills 與 147,458 strategy output records，event／final-state checksums 相同。基線及後測未交錯執行且各僅三輪，約 2.3% 的中位數差異尚非穩定加速證據。stream 在記憶體中複製，benchmark 不包含 source decode、normalization、cache I/O 與 artifact publication，也未量 peak memory；strategy 僅為 deterministic acceptance workload，不是真實 Alpha 策略。仍需真實完整 source day 與 peak-memory measurement，才可判斷生產回測的瓶頸及效益。

2026-09-14 補測：對已編譯的 `full_multi_backtest` synthetic benchmark 單次執行 `GNU time -v`，process peak RSS 為 78,296 KiB；本測量含程式啟動與 allocator overhead，且未重複量測，只作目前 49,152-event synthetic workload 的粗略 process 上限觀察。此值後由下方 benchmark harness 更正取代；真實完整 market-day peak-memory measurement 仍未完成。

2026-09-14 benchmark harness 更正：舊版 `Case::run(&self)` 在計時區複製整個 event-stream map，故前述 140.6 ms、139.1／135.9／137.1／134.4 ms 與 78,296 KiB peak RSS 都包含額外輸入複製，不作為 runner-only 效能／記憶體基線。現在 `Case::run(self)` 將預先建構的 streams move 進 factory，並在計時開始前建立每輪輸入；runner、replay、strategy、simulator、ledger 與 finalize 仍在計時內。`cargo bench -p osmium-runner --bench full_multi_backtest` 每個 process 執行 21 輪，3 次觀察到的 process 內 median 為 125.396／120.157／121.365 ms（約 392k／409k／405k events/s）；每輪 16 fills、147,458 output records，event／final-state checksum 相同。後兩次 warm runs 約 120–121 ms；第一輪 process 較慢，故保留三次值而不只報最佳值。另一次 `GNU time -v` process peak RSS 為 50,012 KiB。以上仍是 8-stream、49,152-event synthetic in-memory workload，不含 source decode、normalization、cache I/O、artifact publication，也不是 Alpha strategy 或真實 market-day profile；peak RSS 包含 runtime／allocator 且只有一次。舊數字與新數字不可直接比較以宣稱程式加速，真實完整 source day、Alpha workload 和可重複 peak-memory profile 仍缺。

第二次獨立 `GNU time -v` 量測為 median 124.303 ms、process peak RSS 49,848 KiB。連同前述三次，各 process 內 median 範圍為 120.157–125.396 ms，checksums／fill／output counts 均一致；兩個 process peak RSS 為 49,848／50,012 KiB。這是四個 process 的初步可重複性證據，不是跨機器／真實資料日的效能保證。

2026-09-14 真實 source pipeline補測：使用已通過 `data verify` 的 TWSE 2330
（105,236 raw records）、TPEx 6488（83,036）與 TAIFEX TXFH6（192,667），合計
380,939 records。原 `target/m4-data` 外層 `partition.yaml` 是 layout v1；為保留 immutable
revision，另建 ignored `target/refactor-real-data-v2` validation root，只 reflink/copy
`current.yaml` 與 revisions，不複製舊 descriptor，再由目前 `SourcePartitionKey` 建立 v2
descriptor與新 cache。三個 source revision identities保持不變。

- cold `cache prepare`：12.09 s，peak RSS 508,744 KiB；三個 caches皆由 verified source重建。
- warm `cache prepare`：2.67 s，peak RSS 61,964 KiB，三個 cache identities皆重用且無 filesystem output。
- `replay`：370,299 domain events，3.47 s，peak RSS 61,640 KiB；event checksum
  `a5f828450085389b745ef1a24056f4fd1e81bc01ed5679d50c8c16dd8dad670c`，final-state checksum
  `6fb3ef874ff879500ab4740db01e0d46e381f6548d3ca90430c878a29060edc8`。
- 完整 acceptance-strategy `backtest` 五輪 wall time為 3.93／3.96／4.03／4.04／4.11 s，
  中位數 4.03 s（約 91.9k events/s）；peak RSS中位數 349,432 KiB。每輪 6 orders／6 fills，
  五個 run directories逐檔 byte-identical，event／state／strategy output／orders／fills／ledger
  checksums完全一致。
- 每個 run的 `strategy-output.bin` 為 92,574,962 bytes，`GNU time` 顯示約 180,976 個
  filesystem output blocks；這解釋 backtest相較純 replay的大量 I/O與部分記憶體增幅。

此補測已解除「沒有任何真實完整日 source/cache/replay/backtest與可重複 peak RSS」的缺口，
但只涵蓋三個商品的一個完整交易日，strategy是高輸出量 acceptance workload，不是
`osmium-alpha`。因此不能宣稱 Alpha加速率或全市場容量。數據另揭露兩個下一階段熱點：

1. cache builder目前將 page lines、normalized events與排序結果 materialize於記憶體；僅
   370k events的 cold build即達約 497 MiB。應改為 bounded page decode＋分段排序／merge，
   但不可改變 canonical ordering與 cache checksum。
2. runner將每筆 strategy output保留到 finalize後一次發布；本次 92.6 MB output令 backtest
   peak約 341 MiB。應設計 callback transaction完成後即可追加到 staging spool、同步更新 hash，
   simulator只保留未完成 order／ledger必要狀態，最後 atomic publish；失敗 run不得發布成功 artifact。

另發現 CLI診斷不一致：舊 layout v1 source執行 `data verify` 會顯示 revision verified，但
`plan` 正確標成 `RejectCorrupt { reason: ReferenceMismatch }`，`cache prepare` 最終只回泛化的
`CacheMissing`。revision payload驗證與 partition-key驗證本來就是兩層 contract，行為本身可接受；
但 `data verify`／`cache prepare` 應輸出 market、symbol、date與 reference mismatch原因，避免使用者
誤以為 verified source可直接建 cache。這是小範圍 CLI error-context refactor，不應放寬 v1 compatibility。

### MT-011：保留 market-specific annotation type，抽出共同 bit decoding

`TwseStatus`／`TpexStatus` 與 `TwseLimits`／`TpexLimits` 目前有大量相同程式。市場型別分開是正確的，因兩市場未來可能分歧；但 bit extraction 可由 private helper 共用。

建議：

- public `Twse*`／`Tpex*` 型別保持分離。
- 只抽 private bitfield decoder，不建立會掩蓋市場差異的 generic public annotation。
- 每個市場仍有獨立 fixture tests 與 mapping identity。

完成（2026-09-14）：`TwseStatus`／`TpexStatus` 共用 private `QuoteStatusBits`，兩市場的 limit 欄位共用 private `QuoteLimitBits`；public wrapper 與 `MarketAnnotations` variant仍分開，TWSE／TPEx normalizer mapping name/version 也各自獨立。新增 public API 測試，逐一驗證兩市場 status／limit bytes 的 256 種 raw 組合，檢查 bit extraction 與 reserved bits 不遺失。

### MT-012：測試與 source 分離

market-types public API 測試放在 `crates/market-types/tests/`，避免測試原始碼與 domain implementation 混在同一 module。

建議：

- 將只測 public API 的 `time.rs` tests 搬到 `tests/match_time.rs`。
- private parser helper 若需要白箱測試才留在 source module；優先透過 public API 測不變條件。
- event／canonical tests 可依責任拆成 `book.rs`、`observations.rs`、`event_codec.rs`、`stability.rs`，避免 `domain_events.rs` 持續膨脹。

完成（2026-09-14）：`time.rs` 已改名為 `match_time.rs`，MatchTime public API tests 位於 `tests/match_time.rs`；`domain_events.rs` 已拆除並分為 `annotations.rs`、`book.rs`、`observations.rs`、`event_codec.rs`、`stability.rs`，test builders 依用途拆在 `tests/support/book.rs` 與 `tests/support/event.rs`，避免每個 integration-test crate 編入未使用 helper。`market-types/src/` 無 `#[cfg(test)]` 測試；測試仍透過 public API 驗證行為。另將只在 `osmium-config` tests 使用的 `EFFECTIVE_CONFIG_VERSION` 改為測試處 qualified path，清除該 library unused-import warning；`data-sync` 的 path-decoding test helper 移到 test module 前，符合 Rust item layout。`cargo fmt --all --check`、`cargo test -p market-types -p osmium-config`、`cargo test -p data-sync partition::tests` 與 `cargo clippy --workspace --all-targets -- -D warnings` 通過。

### MT-013：驗證與 cache reuse 不依賴特定 provider

平台核心不是 Teralion-specific。Provider 名稱只應存在於 source adapter、wire normalizer、其
fixtures 與 conformance tests；partition integrity、domain lifecycle、cache codec、replay、strategy
與 simulation 不得列舉 Teralion formats／flags。

完成（2026-09-14）：

- `data.source` 已進入 effective config 與 partition/cache storage namespace；通用路徑使用
  `source/<source>/...` 與 `cache/replay/<source>/...`，不再由 repository／cache factory 假設
  `teralion`。
- `source_partition.py` 與 `stability_lifecycle.py` 只接受 provider-neutral storage/domain contract；
  `verify_teralion_stability.py` 明確只屬 Teralion adapter certification。
- 新增可由外部 adapter 建立的 `NormalizerMappingIdentity`。`CacheReader` 只驗證能否解碼的通用
  cache／domain schema；`PartitionCacheCatalog` 依本次 source adapter 提供的 exact mapping identity
  判斷 derived artifact 是否可重用。
- 修正 catalog 曾只比對 source revision／partition identity、導致 schema 升版後仍規劃 reuse、
  最後在 replay 才得到 `IncompatibleDescriptor` 的錯誤。現在舊 schema／mapping 明確分類為
  `CacheState::Stale`，由 verified source 離線重建。真實 TWSE／TPEx stability partitions 已驗證
  `stale -> rebuild -> reuse -> replay -> backtest`。

接手收斂（2026-09-14）：`SourceAdapterRuntime` 由 `SourceId` 選擇 online adapter；credential、
coverage／range／ticks／daily queries、cursor sync、staging 與 publish 都位於 `data-sync` adapter
boundary。CLI 只傳入 partition／instrument contract／session plan 並接收 `SourceSyncResult`，
`osmium-config` 不再暴露 `ArchiveMarket`。Normalizer config 與 catalog mapping identity 共用同一
profile resolver；TAIFEX futures／spread／options 都保留 session 的離散 windows，不把中間 gap
當成可回播時間。新增 provider 不需修改 domain／replay 的 wire contract。

Cache identity 原先只 hash mapping version，現加入 mapping name 並升為 cache format v3；測試
實際建立同版號、不同 mapping name 的 caches，確認 descriptor identity 與路徑不同。
`data verify` 與 `cache prepare` 也已共同驗證 partition descriptor；layout/reference mismatch
會回報 market／symbol／date 與 `ReferenceMismatch`，不再誤報 revision verified 或只回 `CacheMissing`。

## 6. 建議實作順序

### Phase 0：補證據，不改 schema

1. **規格核對完成但版本仍須 source-owner 確認**：TWSE B.12.00 與 TPEx V.12.18 的 trial、延後開收盤、暫緩撮合方向及 raw wire 欄位語意已記入 interface docs；公開規格不等同 Teralion JSON mapping 證據。
2. **Teralion adapter certification 完成**：以 `.env` credential 同步 TWSE 3026 與 TPEx 2948／2026-08-10 為 ignored immutable partitions；`data verify` 分別驗證 4,185／834 records，adapter verifier 在兩市場的 realtime／snapshot streams 都找到 trigger → trial → formal result → continuous lifecycle。相同 partitions 已完成 cache build 與 replay。
3. **Generic gate 已與 provider certification 拆分**：`source_partition.py` 只驗證 published pointer／manifest／object checksum／record count，`stability_lifecycle.py` 只驗證 adapter 已分類的 observations；兩者的 synthetic tests 不含 Teralion 欄位。`verify_teralion_stability.py` 明確只做 Teralion wire mapping certification。`/data/hschi1106` 的 normalized lifecycle只作 Alpha semantic baseline；provider-neutral domain／Alpha gates 不依賴特定 provider raw payload。
4. **parser、domain 與 source regression 完成**：盤中 trial 不得落入 firm `QuoteSnapshot`／更新正式 recent trade／book／volume；`trial=false` 的 status-only 暫緩撮合產生 provider-neutral `MarketStatus`，保留既有 firm book／trade 並更新 annotations，不合成 trial event。TWSE／TPEx zero-quantity VI trigger 只接受明確 VI trend、非 intermediate、空 book；一般 quote 或非空 book 的零量 deal被拒絕。真實 TWSE cache build另揭露零價市價委託量，已以 provider-neutral `BookSide.market_order_quantity` 保存且不作 price/fill evidence。`strategy-api` 分 TWSE／TPEx 測試 trigger matching state 和 order restriction。

### Phase 1：低風險 primitive 修正

**已完成（2026-09-14）**：`time.rs` → `match_time.rs`、public API tests 拆分、`Decimal` exact formatter、`MatchTime`／`TradingDate` boundary validation，以及 canonical/cache decoder allocation bounds。各項已由 `market-types`、`data-sync` tests 覆蓋；接手後以核心 commit `964831b` 一起提交必要 downstream migration，以保持 breaking API 變更後可編譯。

### Phase 2：signed price 與 instrument capability

**主要實作與 workspace 驗證已完成（2026-09-14）**：TAIFEX spread／signed price 測試、positive-only market validation、明確 `InstrumentClass`／contract shape／session profile、移除 `Unknown` execution fallback 與 symbol 推定、支援 `/` symbol 的 reversible storage path encoding，以及 normalizer／replay／orders／simulation／accounting 的 signed-price audit。具體設計與測試範圍分別記於 MT-001～MT-003；真實 stability source certification 另由 provider adapter gates 驗證。

### Phase 3：indicative auction schema vNext

**已完成（2026-09-14）**：

1–6. typed indicative event、firm／indicative state separation、trading context／execution eligibility 與所有 schema／mapping identities 已通過 Lab workspace 驗證；舊 cache 拒絕並由 verified source 重建。
7. Alpha adapter 與四個 strategy versions 已遷移，正式 pin 至 `964831b2dfc2baab0a6eafa801e1cbffd1bb274d`；未使用 path override 的 53 tests、fmt、Clippy 通過。
8. 同一 provider-neutral TWSE／TPEx source-derived fixtures 與 synthetic Down case 完成兩版 production strategy、simulator／ledger 及完整 scheduled runner differential。範圍與合成補充明列於 §4.6，不宣稱所有真實行情策略結果完全相同。

### Phase 4：命名與效能

**第一階段已完成（2026-09-14）**：`TradePrint`／`TradeOrder` public API rename、ReplayCore merge 重用 canonical frame、TAIFEX cached sort key，以及固定 synthetic benchmarks／checksum equivalence tests。已量測 canonical hot path、cache pipeline、1／8／32-stream merge、修正計時邊界後的完整 synthetic multi-backtest，以及三商品完整交易日的真實 source cache/replay/backtest與五輪 peak RSS；結果與限制記於 MT-010。真實補測已識別 cache-build materialization與 strategy-output artifact為下一階段熱點。**尚待 Alpha strategy與更大 universe的同源前後量測**，才能宣稱生產回測加速率或容量上限。

## 7. 必要測試矩陣

| 類別 | 最少案例 | 目前證據／未完成門檻 |
| --- | --- | --- |
| Stability | TWSE／TPEx trigger、持續 trial、正常 flags、reserved bits、正式 auction result | 通用 lifecycle tests 不含 provider wire fields；Teralion adapter 已對兩市場真實 immutable partitions 的 realtime／snapshot streams完成 certification，並只保存 sanitized report數值於本文件。TWSE／TPEx parser/context tests、`MarketStatus` reducer isolation 與 simulator policy tests 已有。 |
| Trial classification | opening、closing、delayed open／close、active-session stability、active-session unclassified trial | Opening／closing／delayed 與 unclassified synthetic cases 已有；在來源 mapping 未證實前，不把 synthetic up/down trial 說成已驗證 `IntradayStability(direction)`。 |
| State isolation | indicative price／book／volume 不覆蓋 firm state；session boundary reset | `market-state` reducer tests 已覆蓋 isolation、matching resume 與 carry boundary。 |
| Execution | indicative observation 不填一般 order；正式 call-auction result 可依明確 policy 評估 | 一般 execution 的 indicative rejection、scheduled auction matching 與 VI cancellation／activation tests 已有；正式 auction matching 測試使用明確構造的 evidence，非 Teralion stability sample。 |
| Alpha migration | trigger／方向／trial價量／timer／order intent語意等價；interleaved futures event不得使用股票 trial book | Alpha regressions 覆蓋 pending window、重複／反向 pulse、kind/reason 配對、delayed-open 09:00／09:02 與 stale-candidate 邊界；continuous-exit regression 確認 firm map 不因 stability trial 污染，並以污染對照證明可避免錯誤雙腿 exit。正式 git pin 的 Alpha workspace 53 tests 通過；三案例 production differential 與完整 runner 財務比較見 §4.6。 |
| Signed price | negative spread trade／book／limit order、zero sentinel、equity negative rejection | TAIFEX synthetic/source-format fixtures、order／simulation／accounting 測試已涵蓋；zero sentinel 僅限 interface contract 指定的 I022 欄位。 |
| Symbol | calendar spread `/`、path encoding round trip、collision、unsafe component | partition/cache layout tests 涵蓋 `/`、percent、Unicode、unsafe component、collision 與長 symbol。 |
| Codec | vNext round trip、舊版拒絕、oversized count、truncated／trailing bytes、checksum | canonical mutation/property tests、oversized cache record、round trip 已有；`CacheReader` 只拒絕 cache／domain／canonical／ordering schema 不相容，不維護 provider mapping 白名單。Catalog 另以 source adapter 提供的 mapping identity 將舊 mapping分類為 `Stale` 並重建；真實舊 cache → rebuild → reuse流程已通過。 |
| Time/date | epoch boundary、offset overflow、四位數日期範圍 | `market-types` boundary 與 canonical encode/decode symmetry tests 已有。 |
| Reproducibility | shuffled source input、相同 checksum、cache rebuild equivalence | 合成 replay、cache pipeline 與 benchmark checksum equivalence 已有；三個非 stability verified real-source partitions已完成新版 cache重建，五次 backtest run directories逐檔一致。真實 stability partitions 的 v3 rebuild/reuse/replay 通過；Alpha full runner 同版本重跑 traces 相同，跨版本只比較語意／財務，schema checksum 明確排除。 |
| Performance | encode/hash throughput、normalizer sort、cache read、replay events/sec、peak memory | synthetic hot-path、cache、merge、full runner benchmark 已有；修正計時邊界後 full runner 為 21 rounds，四個 process median 125.396／120.157／121.365／124.303 ms，兩次 RSS 50,012／49,848 KiB。另完成三商品完整日真實 source量測：cold cache 12.09 s／508,744 KiB，warm reuse 2.67 s／61,964 KiB，370,299-event replay 3.47 s／61,640 KiB，五輪 full backtest中位數 4.03 s／349,432 KiB。尚缺 Alpha strategy與更大 universe的同源前後比較。 |

## 8. 文件與版本影響

需要同步更新：

- `docs/product-requirements.md`：只在產品 scope 需要新增明確 stability observation contract 時修改。
- `docs/architecture/replay-model.md`
- `docs/interfaces/twse.md`
- `docs/interfaces/tpex.md`
- `docs/interfaces/taifex.md`（signed spread price 與 symbol）
- `docs/traceability.yaml`
- `EVENT_SCHEMA_VERSION`
- `CANONICAL_EVENT_VERSION`
- normalizer mapping identities
- cache identity／format compatibility
- ordering rule（只有 event kind rank 改變時升版）
- public strategy API migration notes與 `CHANGELOG.md`
- `osmium-alpha` dependency revision、strategy identity/version決策與 migration notes

## 9. 暫不做的事情

- 不重算 3.5% stability threshold、五分鐘 VWAP 或交易所 reference price。
- 不依兩分鐘 duration 自行補出缺少的結束事件。
- 不模擬真實 queue position、hidden liquidity 或交易所完整撮合。
- 不把 `received_at` 用作 replay ordering或 stability lifecycle時間。
- 不因 TWSE／TPEx bit layout目前相似就合併成同一 public annotation type。
- 不把 repository 外的真實 Teralion certification 說成所有 provider 的通用驗證；新增 provider 必須通過相同 domain contract及自己的 wire conformance。
- 不為舊 indicative event API、trial `QuoteSnapshot` workaround 或舊 derived cache 建立 compatibility layer。

## 10. 參考來源

- [Teralion Feed Archive API](https://docs.teraliontech.com/feed-archive/)
- [Teralion Feed API](https://docs.teraliontech.com/feed/)
- [TWSE 集中市場交易制度介紹](https://wwwc.twse.com.tw/zh/products/system/trading.html)
- [TWSE 瞬間價格穩定措施](https://shl.twse.com.tw/page/trading/6.html)
- [TPEx 上櫃有價證券交易系統簡介](https://www.tpex.org.tw/zh-tw/mainboard/trading/rules/system.html)
- [TPEx 盤中全面逐筆交易專區](https://www.tpex.org.tw/zh-tw/mainboard/trading/rules/continuous.html)
- [TAIFEX 期貨跨月價差委託說明](https://www.taifex.com.tw/cht/4/oamIntroduction)
- [TAIFEX Futures Calendar Spread FAQ](https://www.taifex.com.tw/eng/eng2/dev_plans/02%20Futures%20Calendar%20Spread.pdf)
