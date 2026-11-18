# 架構

決策與理由見 [ADR-0008](decisions/0008-full-market-matching-simulation.md)。本頁描述目標架構與遷移
狀態；各元件的精確行為以 [`docs/reference/`](../reference/) 為準。

## 資料流

```text
marketdata /srv/md/v1（唯讀）
  │  osmium-md：schema 與 manifest 驗證 → 依 seq 的訊息流（可經衍生快取）
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
| `osmium-types` | 價格、數量、金額、時間、商品、board、行情訊息的型別與換算 | 任何規則 |
| `osmium-md` | marketdata layout、schema 檢查、manifest identity、訊息解碼、衍生快取 | 排序以外的語義 |
| `osmium-replay` | 跨 feed 全序、replay clock、事件時間線 | 狀態、撮合、讀檔 |
| `osmium-book` | 撮合方式、交易模式、目前五檔、試撮與暫緩撮合狀態 | 模擬委託 |
| `osmium-strategy` | 策略 trait、context、委託意圖、計時器 | 成交判定 |
| `osmium-matching` | 委託生命週期、交易規則檢查、撮合、排隊估計 | 現金與部位 |
| `osmium-ledger` | 現金、部位、信用交易、交割、公司行動、費稅、損益、對帳 | 成交判定 |
| `osmium-runner` | 組裝、run 設定、輸出 artifacts | 上述任何規則 |
| `osmium-cli` | 指令、參數、錯誤輸出 | 上述任何規則 |

## 不變條件

- 策略只看得到 replay clock（加行情延遲）之前已揭示的資料。
- 模擬委託只會被其到達交易所之後的訊息撮合，且不改變真實行情。
- 價格、數量、金額全部是整數運算，沒有 binary floating point。
- 相同 marketdata identity、版本與設定，產生位元組相同的輸出。
- 資料與 manifest 不一致、schema 不符、或遇到未定義的語義組合，一律失敗，不猜測。

## 遷移狀態

| 目標 crate | 狀態 | 取代的舊 crate |
| --- | --- | --- |
| `osmium-types` | 進行中：價格、股數、代號、時間、feed、行情訊息、商品基本資料；金額等隨使用的 crate 加入 | `market-types` |
| `osmium-md` | 讀取、驗證、訊息解碼完成；串流讀取與衍生快取未開始 | — |
| `osmium-replay` | 跨 feed 合併與時間線完成；交易日流程在 `osmium-runner` | `replay-engine` |
| `osmium-book` | 完成：撮合方式、交易模式、目前五檔、試撮與暫緩撮合狀態 | `market-state` |
| `osmium-strategy` | 未開始 | `strategy-api` |
| `osmium-matching` | 未開始 | `execution-sim`（撮合部分） |
| `osmium-ledger` | 未開始 | `execution-sim`（帳務部分） |
| `osmium-runner` | 未開始 | `osmium-runner`、`run-planner` |
| `osmium-cli` | 未開始 | — |
| `osmium-examples` | 未開始 | `example-strategy` |

舊 crate 的文件暫存在 [`legacy/`](legacy/)，隨舊 crate 刪除。
