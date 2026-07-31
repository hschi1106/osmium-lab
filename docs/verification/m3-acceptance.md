# M3 TAIFEX Acceptance

## 1. 範圍與目前狀態

M3 的 committed acceptance scope 是 `2026-07-20` 的三個 TAIFEX 商品：

| profile | symbol | session |
| --- | --- | --- |
| 股價指數期貨 | `TXFH6` | `after_hours`、`regular` |
| 盤後適用股票期貨 | `CDFH6` | `after_hours`、`regular` |
| 日盤限定股票期貨 | `CAFH6` | `regular` |

目前驗收狀態為 **Blocked / partial**。三商品 source、cache、replay、strategy、
futures accounting、offline inspect 與 deterministic checks 已可重跑；四商品
`TWSE 2330 + 三個 TAIFEX` 尚未通過，因 repository 沒有同一交易日的 TWSE tick
fixture。只有 daily instrument discovery payload 不能替代 ticks。

machine-readable 結果由
[`tools/run_m3_acceptance.sh`](../../tools/run_m3_acceptance.sh) 產生，預期位置為：

```text
docs/verification/evidence/m3/formal-<UTC-date>/acceptance-report.yaml
```

## 2. Entry gates

| gate | 狀態 | 證據 |
| --- | --- | --- |
| TAIFEX 三商品 redistribution approval | Passed | 各 fixture `metadata.yaml` 的 `redistribution` |
| TAIFEX shard／set checksum | Passed | `tools/verify_m3_fixtures.sh` |
| TAIFEX daily instrument checksum | Passed | 各 fixture `daily.json` 與 metadata |
| secret scan | Passed | fixture verifier 與 harness log |
| TWSE 2330 `2026-07-20` tick fixture | **Blocked** | harness `four-instrument-gate.yaml` |

TAIFEX fixture verifier 會檢查每個 selected shard 的 bytes、records、SHA-256、
global concatenation checksum、daily instrument checksum、market/symbol/date identity
與 forbidden fields。source payload 保持 wire type；`book`、`trade`、`close`、`stats`
不會被 generic adapter 改寫成 `quote`。

## 3. Reproduction

```sh
tools/run_m3_acceptance.sh \
  --output docs/verification/evidence/m3/formal-$(date -u +%Y-%m-%d) \
  --allow-blocked
```

此 command 會：

- 執行 formatter、clippy、debug/release workspace tests。
- 驗證三份 TAIFEX fixture 與 daily instruments。
- 用 committed fixture 經 cursor validation 建立 immutable source revision 與
  partition cache。
- 執行無 `TERALION_API_KEY`、network denied 的 `plan`、`verify`、`cache prepare`、
  `replay`、`backtest` 與 `inspect`。
- 執行 10 次 byte-identical rerun、三個 universe discovery permutations、cache
  rebuild、debug/release comparison 與 ledger corruption rejection。
- 記錄 opened stream audit 與三商品 performance baseline。

未提供 `--allow-blocked` 時，四商品 gate 仍為 Blocked，script 以 exit code `3`
結束；這是驗收阻塞訊號，不是測試 crash。

## 4. Acceptance catalog

`M3-AC-01` 至 `M3-AC-06`、`M3-AC-08` 至 `M3-AC-14` 與 `M3-AC-16` 在目前三商品
scope 由 harness report 記錄。`M3-AC-07`（四商品 ordering）、`M3-AC-15` 的
四商品 determinism 部分與 `M3-AC-17` 的四商品 performance 保持 Blocked。

正式 completion 需要：

1. 提交 authorized `fixtures/teralion/twse/2330/2026-07-20` tick shards、daily
   instrument、metadata、source page checksums 與 fixture-set checksum。
2. 以同一 config 重新建立四個 partition source/cache。
3. 重新執行 harness，並確認 report 不含 `Blocked`、`Partial` 或 missing evidence。
4. 更新 [`docs/traceability.yaml`](../traceability.yaml) 的 M3 status 為 `complete`。
