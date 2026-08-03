# M3 TAIFEX Acceptance

## 1. 範圍與目前狀態

M3 的 committed acceptance scope 是 `2026-07-20` 的 TWSE `2330` 與三個 TAIFEX
商品：

| profile | symbol | session |
| --- | --- | --- |
| TWSE equity | `2330` | `regular` |
| 股價指數期貨 | `TXFH6` | `after_hours`、`regular` |
| 盤後適用股票期貨 | `CDFH6` | `after_hours`、`regular` |
| 日盤限定股票期貨 | `CAFH6` | `regular` |

目前驗收狀態為 **Passed / full**。TWSE fixture 來自 Teralion `quote` cursor
download 的 22 頁；抽取 `STOCK_SNAPSHOT` 3,571 筆與 `STOCK_REALTIME` 98,298 筆，
並保留 daily instrument、source page checksums、fixture-set checksum 與 source/cache
lineage。零股 format 未進 regular fixture。

machine-readable 結果由
[`tools/acceptance/run_m3_acceptance.sh`](../../tools/acceptance/run_m3_acceptance.sh) 產生，預期位置為：

```text
docs/verification/evidence/m3/formal-<UTC-date>/acceptance-report.yaml
```

source/cache 與 repeated run directories 是可重建的 derived artifacts，harness
完成後會清除；durable evidence 保留 acceptance report、test logs、stream-open audit、
performance summary、corruption result 與 canonical artifact checksums。

## 2. Entry gates

| gate | 狀態 | 證據 |
| --- | --- | --- |
| TAIFEX 三商品 redistribution approval | Passed | 各 fixture `metadata.yaml` 的 `redistribution` |
| TAIFEX shard／set checksum | Passed | `tools/acceptance/verify_m3_fixtures.sh` |
| TAIFEX daily instrument checksum | Passed | 各 fixture `daily.json` 與 metadata |
| TWSE 2330 shard／set checksum | Passed | `tools/acceptance/verify_m3_fixtures.sh` |
| TWSE 2330 redistribution approval | Passed | `fixtures/teralion/twse/2330/2026-07-20/metadata.yaml` |
| TWSE daily instrument checksum | Passed | fixture `daily.json` 與 metadata |
| secret scan | Passed | fixture verifier 與 harness log |
| TWSE 2330 `2026-07-20` tick fixture | Passed | harness `four-instrument-gate.yaml` |

fixture verifier 會檢查每個 selected shard 的 bytes、records、SHA-256、global
concatenation checksum、daily instrument checksum、market/symbol/date identity 與
forbidden fields。source payload 保持 wire type；TAIFEX `book`、`trade`、`close`、
`stats` 不會被 generic adapter 改寫成 `quote`。

## 3. Reproduction

```sh
tools/acceptance/run_m3_acceptance.sh \
  --output docs/verification/evidence/m3/formal-$(date -u +%Y-%m-%d)
```

此 command 會：

- 執行 formatter、clippy、debug/release workspace tests。
- 驗證 TWSE fixture、三份 TAIFEX fixture 與四份 daily instruments。
- 用 committed fixture 經 cursor validation 建立 immutable source revision 與
  partition cache。
- 執行無 `TERALION_API_KEY`、network denied 的 `plan`、`verify`、`cache prepare`、
  `replay`、`backtest` 與 `inspect`。
- 執行三商品與四商品各 10 次 byte-identical rerun、各三個 universe discovery
  permutations、cache rebuild、debug/release comparison 與 ledger corruption rejection。
- 記錄三商品與四商品 opened stream audit、performance baseline 與 canonical artifact
  checksums。

network runner 必須拒絕外網；harness 會在沒有 credential 的環境執行所有 offline
commands。

## 4. Acceptance catalog

`M3-AC-01` 至 `M3-AC-17` 均由四商品 formal report 記錄為 `Passed`；三商品
scope 的結果也保留，作為 TAIFEX profile 與 multi-market run 的對照。

formal report 不含 `Blocked`、`Partial` 或 missing evidence；source/cache 與 repeated
run directories 仍是可重建的 derived artifacts，不納入版本控制。
