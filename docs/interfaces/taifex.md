# TAIFEX Teralion 介面

適用 normalizer：

- outright futures：`TeralionTaifexFutures`，mapping version `5`，wire market `taifex_fut`。
- calendar spreads：`TeralionTaifexCalendarSpreads`，mapping version `2`，wire market `taifex_fut`。
- index options：`TeralionTaifexOptions`，mapping version `4`，wire market `taifex_opt`。

上述 profiles 在 domain 中都使用 `MarketId::Taifex`；archive market 與 mapping identity依明確
instrument contract選擇，不由 symbol推定。共通 observation/state semantics 見
[回播模型](../architecture/replay-model.md)。

I020 trade 與 I080／I082 完整五檔是已驗證的 firm observation，normalizer 明確輸出
`MarketSignal::Continuous`；I022 opening calculated observation 輸出
`AuctionCollecting(AuctionPurpose::Opening)`。generic event constructor 不會用
`Continuous` 補缺少的 provider evidence，也沒有 instrument-day market background lookup；若
來源未提供 signal，core 保留 `NoObservation`／`Unknown`。

## 1. Session 與時間

regular session 為 08:45–13:45。index futures／options 的 after-hours session 為 15:00–次一交易日 05:00，stock futures 為 17:25–次一交易日 05:00；planner 另加入前後五分鐘 margin。跨週末／假日的 after-hours segment 仍歸屬目標 exchange trading date。

`received_at` 只用於 source query，`match_time` 是唯一 replay time。Teralion record 沒有可用的 TAIFEX global sequence；`first_packet` 是 message 內語意，不能替代 source sequence。page、cursor、file order 與 `received_at` 不參與 event ordering。

## 2. Format registry

| Type / format | 處理方式 |
| --- | --- |
| `trade / I020` | `TradeBatch` |
| `trade / I022` | `IndicativeAuction(Opening)` |
| `book / I080` | `BookSnapshot` |
| `book / I082` | WarmUp reference `BookSnapshot` |
| `trade / I021` | `KnownSkipped(IntradayHighLow)` |
| `trade / I023` | `KnownSkipped(OpeningReference)` |
| `stats / I030` | `KnownSkipped(OrderStatistics)` |
| `close / I070`、`I072` | `KnownSkipped(ClosingStatistics)` |

known-skipped record 保留 raw payload、count 與 reason，不產生 event。wrong type/format pair、unknown format 或 malformed known format 不是 known skip，在 strict mode 拒絕。

## 3. I020 成交批次

`trades` 至少一筆，保留 array order；outright futures／options price 為 positive exact decimal，calendar spread price 可為負、零或正；quantity 為 positive `Contract`。`aggregate.match_total_qty` 保留在 source payload，目前不映射為 domain cumulative volume，也不由 normalizer 加總當筆 `trades` 推定累計量。

```text
TradeBatch(
  trades = source-ordered regular prints,
  cumulative_volume = NoObservation,
  quantity_unit = Contract
)
```

`first_packet=true` 的完整 item 形成單一 atomic batch。缺少穩定 continuation identity 時，`first_packet=false` 或無法配對 continuation 會被拒絕，不依 input order猜測合併。

## 4. I080／I082 完整五檔

每側接受 0–5 檔，bid 價格嚴格遞減、ask 價格嚴格遞增。price/quantity 必須同時為有效值；trailing empty slots 不沿用前一筆 book。每個 event 都完整替換 state 中的 book。

I080 的 `derived` side 是來源提供的衍生 observation，不是第六檔，也不進一般五檔。I082 是 opening 計算後的 reference book，保留 `I082` source format；只有 session policy允許時可成為 matching evidence。

## 5. I022 開盤試算

I022 產生 `IndicativeAuction(Opening)`，不是 `TradeBatch`。來源 `0/0` 映射為
price／quantity `NoObservation`，同為正值時映射為 typed observation。I022 不更新實際成交
或成交量，也不產生 fill evidence。Calendar spread profile 拒絕 I022，因其不參與開盤集合競價。

I070／I072 的 settlement、open interest、close price 與 statistics 不進 replay timeline 或 MarketState；如需作為帳務輸入，必須另建具時間與版本的正式 domain contract。

## 6. Numeric 與錯誤規則

- JSON numeric lexeme 直接轉 exact decimal，不先經 binary floating-point。
- quantity 必須為正；outright futures／options 的 populated price 必須為正。Calendar spread 的 I020 trade 與 I080 book 保留 signed／zero price，不把零價當缺值。
- I022 的 `0/0` 只屬該 format 的缺值 sentinel，不延伸至其他 formats。
- counter 為 non-negative integer；order count 不等於 contract quantity。
- null、zero 與 unknown 保持不同語意。
- identity、trading-date/session ownership、book ordering、packet shape 或 numeric validation 失敗時 strict reject。

## 7. Instrument reference 與 observed limits

option profile 使用獨立 `taifex_opt` query identity。underlying、expiry、strike、option side、currency、multiplier 與 quantity unit 由 reference／economics 明確提供。

Calendar spread 設定必須明確指定 `contract_shape: calendar_spread` 與適用 session profile。含 `/` 的
symbol 保持 byte-exact identity；storage path另做 reversible encoding。Source mapping只保留可觀察的
signed/zero spread price與 contract quantity，不定義 accounting formula；futures/options economics、
fees與 P&L 見[執行與帳務模型](../architecture/execution-model.md)。

repository fixture 位於 [`fixtures/providers/teralion/taifex`](../../fixtures/providers/teralion/taifex)，只固定合成 futures/options contract，不代表完整交易日。

官方參考：[TAIFEX 逐筆行情資訊傳輸作業手冊](https://www.taifex.com.tw/cht/8/techDocsDetails?idx=67)。共通 event、session 與 ordering 規則見 [回播模型](../architecture/replay-model.md)。
