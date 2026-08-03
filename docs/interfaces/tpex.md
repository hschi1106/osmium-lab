# TPEx Teralion Interface

## 1. 目的與固定範圍

本文件固定 TPEx ordinary-equity 與 warrant 的 Teralion quote wire 到 domain event
boundary；raw wire 不會流入 replay engine 或 strategy API。公開測試使用
[`SYNTH-TPEX-EQ`](../../fixtures/teralion/tpex/SYNTH-TPEX-EQ/2026-07-20) 與
[`SYNTH-TPEX-W`](../../fixtures/teralion/tpex/SYNTH-TPEX-W/2026-07-20) scenarios；兩者皆是
repository-owned synthetic data。適用 mapping 為 `TeralionTpexQuote`，
`mapping_version = 1`。

ordinary-equity scope 不包含 TPEx 零股或選擇權；warrant 使用獨立 profile。其他未被
mapping 固定的 format 仍不在支援範圍；`INTRADAY_ODDLOT_REALTIME` 明確列為 known skip。

## 2. Session 與兩個 clock

| 項目 | Asia/Taipei time |
| --- | --- |
| official regular | 09:00–13:30 |
| acquisition window (`received_at`) | `[08:55, 13:35)` |
| replay window (`match_time`) | `[08:55, 13:35)` |

source download 只使用 `received_at` 選取資料；timeline、ordering、state 與 strategy
只使用 `match_time`，且不以本地檔案日期或 capture time 猜測 trading date。

## 3. Wire registry

`STOCK_REALTIME` 與 `STOCK_SNAPSHOT` 共用下列 envelope 與 quote body：

| field | rule |
| --- | --- |
| `type` | 必須為 `quote` |
| `market` | 必須為 `tpex` |
| `symbol` | 必須等於 frozen partition symbol |
| `format` | 僅 `STOCK_REALTIME`、`STOCK_SNAPSHOT` 進入 replay；其他 format 為明確 known skip 或 strict error |
| `match_time` | 含 offset 的有效 ISO-8601；唯一 replay clock |
| `received_at` | 含 offset 的有效 ISO-8601；source diagnostics only |

每側 `bids`／`asks` 是由 best 到較差的 `0..5` levels；每一合法 quote 都是完整
snapshot replacement，少於五檔的 trailing slots 代表 empty，不與前一筆合併。price
以 exact decimal lexeme 解析，quantity、`cum_volume` 使用 `TradingUnit`。

`deal=null` 表示該 tick 沒有成交 observation。object-form `deal` 的 `quantity=0`
sentinel 由 TPEx normalizer 保留 raw source，並明確映射為
`NoObservation`，不偽造零數量 `TradePrint`。

`open_price`、`high_price`、`low_price` 僅是 snapshot raw lineage，尚未進入
`QuoteSnapshot` domain payload。

## 4. Domain mapping

一般完整 quote 映射為：

```text
QuoteSnapshot(
  complete_five_level_book,
  deal -> Set(TradePrint::Regular) 或 NoObservation,
  Set(cum_volume),
  TpexQuote(status_flags_raw, limit_flags_raw),
)
```

`STOCK_REALTIME` 的 `intermediate_print=true` 且有成交時，normalizer 產生
`TradeBatch`（所有同一 `match_time` 的 intermediate 保持 source order），接著產生
final `QuoteSnapshot`。synthetic scenario 涵蓋 1+1 group；final
`cum_volume` 僅以最後 intermediate 的累計量與 final deal 驗證，不由前一個 book
差分重建。

`status_flags` 與 `limit_flags` 以 TPEx-specific `TpexQuoteAnnotations` 無損保存。
synthetic scenario 驗證 Bit 7 trial 與 ordinary flags；Bit 3 opening marker、Bit 2
closing marker 及 limit
byte 的 raw value；只有 marker 或 session margin 能唯一分類時才產生
`IndicativeOpeningAuction`／`IndicativeClosingAuction`。沒有 marker 的 in-session
trial record 保留為普通 `QuoteSnapshot`，不猜測 auction phase。

normalizer 不產生 order book、queue position、aggressor、latency 或 synthetic
sequence。source 沒有可用 sequence 時，same-time tie-break 沿用 market rank、source
format、event kind 與 canonical fingerprint 的 deterministic fallback。

## 5. Reject／skip policy

- market、symbol、type mismatch、未知 format、缺欄位、非法時間、非法 price/quantity、
  超過五檔或不完整 intermediate group：strict reject。
- `INTRADAY_ODDLOT_REALTIME`：`KnownSkipped(IntradayOddLot)`；raw record 留在 source，
  不進 M4 replay cache。
- source page、cursor、query identity、daily instrument、每頁 checksum 與 fixture
  checksum 由 immutable source revision 保護；重抽取必須 byte-for-byte 一致。

## 6. TPEx warrant profile（M5 extension）

warrant profile 的公開 scenario 是
[`SYNTH-TPEX-W/2026-07-20`](../../fixtures/teralion/tpex/SYNTH-TPEX-W/2026-07-20)，
使用 `market=tpex` 與 `WARRANT_REALTIME`／`WARRANT_SNAPSHOT`，且 metadata 明確標示
`repository-owned-synthetic`。

TPEx 官方 Main Board IP specification 將 warrant continuous-trading 與 snapshot 定義為
獨立的 format family；本 profile 以實際 adapter 回傳的 `WARRANT_*` source-format naming
固定對應，而不是把它們靜默套用到普通股票 profile。

| 項目 | TPEx warrant contract |
| --- | --- |
| normalizer profile | `InstrumentProfile::Warrant` |
| mapping | `TeralionTpexWarrant`，`mapping_version = 1` |
| accepted quote formats | `WARRANT_REALTIME`、`WARRANT_SNAPSHOT` |
| known skip | `INTRADAY_ODDLOT_REALTIME` |
| state profile | `MarketStateReducer::tpex_warrant()` |

TPEx warrant 目前沿用已驗證的 TPEx quote envelope、五檔 snapshot、`TpexQuoteAnnotations`
與 `match_time` 語義，但保留獨立的 warrant source-format profile；M2、M3 fixture builder
及 cache descriptor 會保留 warrant-specific mapping identity，不再把它靜默落入普通 TPEx
branch。未被這份 contract 固定的 format 仍 strict reject。

這份 synthetic fixture 以普通 `QuoteSnapshot`、opening 與 closing indicative auction
event 驗證 quote/state mapping，不代表特定上市商品、完整日分布或交易結果。正式
real-data acceptance 由 repository 外的 authorized data root 執行。

protocol reference：[TPEx Main Board Stock IP Feed Specification V.12.17](https://dsp.tpex.org.tw/storage/regular_system/Main%20Board%20Stock%20IP%20Feed%20Specification%20%28V.12.17_TCPIP%29.pdf)；
warrant identity reference：[TPEx OpenAPI `tpex_warrant_issue`](https://www.tpex.org.tw/openapi/v1/tpex_warrant_issue)。
