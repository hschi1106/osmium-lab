# M2 Reference Acceptance

## 1. 驗收範圍

本文件記錄 M2 `TWSE / 2330 / 2026-07-27 / regular` reference slice 的實際驗收，
並以 [M2 offline backtest](../increments/M2-offline-backtest.md) 與
[verification plan](plan.md) 為判定基準。詳細 machine-readable 結果位於
[`evidence/m2/refreshed-2026-07-31.yaml`](evidence/m2/refreshed-2026-07-31.yaml)。
原始 `local-2026-07-31.yaml` 保留為 accounting correction 前的歷史 evidence，
不回寫其 checksum。

```text
acceptance_contract_version = 1
scope                       = M2 reference slice
status                      = Passed
completion_quality          = Strict
```

本結果只涵蓋 M2 的 TWSE 單商品範圍，不代表 M3 的 TAIFEX、多商品 merge 或 M4
市場擴充已完成。

## 2. Published source

live authorized sync 以 `[08:55, 13:35)` download window 取得 16 個 cursor pages，
terminal cursor 成功到達，並發布：

| 項目 | 值 |
| --- | --- |
| source revision | `3258d2ed449611aae74f5a0e8e471e88166483b6ff5a69759f17c08179d27a93` |
| tick records | 77,213 |
| uncompressed bytes | 46,956,177 |
| compressed bytes | 1,452,932 |
| source objects | 16 tick pages + 1 daily instrument |
| source encoding | per-object `ZstdPerPageV1` |

source revision 只含 `.json.zst` payload、manifest 與 current pointer；沒有保存
uncompressed source JSON。每個 object 的 compressed／uncompressed SHA-256 均由
`verify` 重算成功。相同 config 第二次 `sync` 回報 `http_requests=0`。

## 3. Offline execution

offline acceptance 使用 `sandbox-exec` 的 `deny network*` policy，並移除
`TERALION_API_KEY`。backtest 只讀已發布 source/cache，成功處理 73,795 events：

| 項目 | 值 |
| --- | --- |
| cache identity | `4a4956708c2ab359a4c8f05fc64a47f82ec8fed6d817676f18795b5a410b9db5` |
| event payload SHA-256 | `156c00f056086d27aae57ac94fc08669623dd227bff685d2add2db2b687ff66d` |
| accepted orders | 4 |
| fills | 2 |
| final position | 0 trading units |
| final cash atoms | `9981363000000000000000000` |
| realized P&L atoms | `-5000000000000000000000` |

acceptance strategy 發出 pre-open limit、market buy/sell 與 resting limit intents。
execution coordinator 只讓較早 occurrence 建立的 pending order 使用後續 evidence，
normal EOF 取消未完成 Day orders。market/limit、rejection、partial fill、allocation、
slippage、fee/tax 與 reconciliation 的其他固定 branches 由 workspace contract tests
覆蓋。

`a7dcb90` 修正 realized P&L 的 economic quantity scaling 後，reference backtest
重新執行。event、state、strategy、order、fill、final cash 與 position 不變；
ledger 及 run manifest checksum 依修正後的 `-5000` TWD realized P&L 更新。

## 4. Determinism and recovery

- 相同 source、cache、config 與 strategy 連續 10 runs 的 run directories
  byte-identical。
- network-enabled preparation、network-disabled debug 與 network-disabled release 結果
  byte-identical。
- 移走 derived cache 後，在無 key 環境只由 local source 重建；cache bytes、
  source revision 與 run artifacts 均不變。
- cache-hit release backtest reference time為 0.58 秒；cache payload為 24,268,900
  bytes。此數字是 reference measurement，不是效能門檻。

## 5. Integrity and security

`inspect` 不讀 Teralion、不重跑策略，會先驗證 manifest version 與全部 attachment
checksums。自動測試竄改 `ledger.bin` 後，`inspect` 必須回傳 checksum error。

repository tracked files、published source/cache 與 reference run artifacts 已以實際
credential exact value 掃描，findings 為零。credential、authorization header、
cursor 與 request payload 不進入 run artifacts 或 log。

## 6. Validation commands

```sh
cargo test --workspace
cargo test --workspace --release

target/debug/osmium verify --config config/m2-twse-2330.yaml
target/debug/osmium backtest \
  --config config/m2-twse-2330.yaml \
  --output target/m2-run
target/debug/osmium inspect --run target/m2-run
```

正式 offline verification 另以 OS network sandbox 執行同一個 `backtest` binary；
不能以 transport mock 取代。
