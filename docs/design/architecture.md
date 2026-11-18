# 架構

決策與理由見 [ADR-0008](decisions/0008-full-market-matching-simulation.md)。本頁描述架構與實作
狀態；各元件的精確行為以 [`docs/reference/`](../reference/) 為準。

## 資料流

```text
marketdata /srv/md/v1（唯讀）
  │  osmium-md：schema 與 manifest 驗證 → 依 seq 的訊息流
  ▼
osmium-replay：多 feed 合併 → observed_time 時鐘
  │
  ├─▶ osmium-book：每個商品 × board 的五檔與交易階段
  │
  ├─▶ osmium-matching：模擬委託對真實訊息撮合（連續交易、集合競價、延遲、排隊估計）
  │        │ 成交
  │        ▼
  │   osmium-ledger：現金、部位、信用交易、交割、公司行動、費稅、損益
  │
  └─▶ osmium-strategy：策略讀取已揭示狀態，輸出委託與計時器
                      （委託經延遲後交給 osmium-matching）

osmium-runner：串接以上各層、run 設定與輸出；osmium-cli：指令列入口
```

## Ownership

| Crate | 擁有 | 不擁有 |
| --- | --- | --- |
| `osmium-types` | 價格、數量、金額、時間、商品、board、行情訊息、委託與委託事件的型別與換算 | 任何規則 |
| `osmium-md` | marketdata layout、schema 檢查、manifest identity、訊息解碼、參考表（日曆、盤後價格、公司行動、事件）、衍生快取 | 排序以外的語義 |
| `osmium-replay` | 跨 feed 全序、replay clock、事件時間線 | 狀態、撮合、讀檔 |
| `osmium-book` | 撮合方式、交易模式、目前五檔、試撮與暫緩撮合狀態 | 模擬委託 |
| `osmium-strategy` | 策略 trait、context、行情與帳戶視圖、參數、策略註冊表 | 成交判定、帳務 |
| `osmium-matching` | 委託生命週期、交易規則檢查、撮合、排隊估計、瞬間價格穩定措施的範圍 | 現金與部位、延遲、讀檔 |
| `osmium-ledger` | 現金、部位、信用交易、交割、公司行動、費稅、損益、對帳 | 成交判定 |
| `osmium-runner` | 交易日流程、延遲的排程、run 設定、輸出 artifacts | 上述任何規則 |
| `osmium-cli` | 指令、參數、錯誤輸出 | 上述任何規則 |
| `osmium-examples` | 範例策略 | 任何規則 |

`osmium-matching` 是同步的：呼叫端依時間線把行情訊息與已到達交易所的委託交給它，它回傳委託
事件。延遲與排序屬於 `osmium-replay`／`osmium-runner`，帳務的事前檢查由呼叫端以 closure 傳入。
它不依賴 `osmium-md`；只有真實資料測試以 dev-dependency 讀取 marketdata。

`osmium-ledger` 也是同步的，而且不讀檔：呼叫端每個交易日把當天的事實（`DayFacts`：商品、前一日
盤後資料、事件、公司行動）與收盤價交給它，委託的事前檢查（`check_new`、`check_reprice`、
`check_forced`）就是傳給 `osmium-matching` 的 closure。它依賴 `osmium-matching` 只為了商品分類與
升降單位；`osmium-md` 同樣只是真實資料測試的 dev-dependency。

`osmium-runner` 是唯一同時認得所有層的 crate。每個交易日它讀取當天的資料，把行情訊息依時間線
交給 `osmium-matching`（交易所當下）與策略（加上行情延遲），把策略的委託加上委託延遲後交給
`osmium-matching`，把回報先記入 `osmium-ledger`、再加上回報延遲後交給策略。策略看到的帳戶是
回報產生當下的快照，不是帳務的即時狀態。

## 不變條件

- 策略只看得到 replay clock（加行情延遲）之前已揭示的資料。
- 模擬委託只會被其到達交易所之後的訊息撮合，且不改變真實行情。
- 價格、數量、金額全部是整數運算，沒有 binary floating point。
- 相同 marketdata identity、版本與設定，產生位元組相同的輸出。
- 資料與 manifest 不一致、schema 不符、或遇到未定義的語義組合，一律失敗，不猜測。

## 實作狀態

ADR-0008 的十個 crate 都已實作，舊 crate 已刪除。

| Crate | 狀態 |
| --- | --- |
| `osmium-types` | 完成 |
| `osmium-md` | 完成：讀取、驗證、訊息解碼、參考表；row group 平行解碼與差分保存見[效能](performance.md)。衍生快取沒有實作（同一頁第 6 節） |
| `osmium-replay` | 完成：跨 feed 合併與時間線；交易日流程在 `osmium-runner` |
| `osmium-book` | 完成 |
| `osmium-strategy` | 完成：[策略介面規格](../reference/strategy.md) |
| `osmium-matching` | 完成：[撮合規格](../reference/matching.md) |
| `osmium-ledger` | 完成：[帳務規格](../reference/accounting.md) |
| `osmium-runner` | 完成：[回放規格](../reference/replay.md)第 4 節以後、[設定](../reference/config.md)、[輸出](../reference/artifacts.md)；逐日循序，一次載入一個交易日 |
| `osmium-cli` | 完成：`osmium` 執行檔與可重用的指令列 library |
| `osmium-examples` | 完成：三個範例策略 |

`osmium-fixture` 不屬於產品：它為 `osmium-md` 以上各層的測試寫出小型的合成 marketdata 樹。
