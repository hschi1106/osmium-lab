# M3 效能基線

本文件定義 M3 acceptance harness 的效能記錄格式。它目前建立可比較的 baseline，
不在沒有足夠實測樣本前設定硬性 throughput threshold。

## Dataset

正式範圍包含三個 TAIFEX profile 及四商品組合：

1. `TXFH6`：盤後加日盤。
2. `CDFH6`：盤後加日盤。
3. `CAFH6`：只含日盤。
4. `TWSE 2330` 加上述三個 TAIFEX 商品。

目前 repository 只有前三項的 committed tick fixture；四商品基線因缺少同一
`2026-07-20` 的 TWSE `2330` tick fixture 保持 `Blocked`，不得以 synthetic data
替代。

## Recorded metrics

`tools/run_m3_acceptance.sh` 會在 `test-results/performance.yaml` 記錄：

```yaml
dataset: taifex-three
events: <replayed event count>
source_bytes: <published source bytes>
cache_bytes: <derived cache bytes>
elapsed_seconds: <wall-clock seconds>
```

此外，acceptance report 必須保留：

- cache build 與 cache-hit replay 的 elapsed time。
- source/cache/run bytes。
- opened stream count 與 stream-open audit。
- debug/release、cache rebuild 與 discovery permutation 的 result identity。
- 具備工具支援時的 peak resident memory；沒有實測值時標示 `not_recorded`，不能猜測。

## Interpretation

baseline 是同一 machine、同一 Rust toolchain、同一 fixture 與同一 config 的比較
基準，不是跨 machine 的 SLA。任何 threshold 必須在至少一個完整四商品 dataset、
明確 hardware profile 與 peak-memory measurement 齊備後，另以 reviewable change
加入；threshold 未定義不等於效能驗收自動通過。

## Reproduction

```sh
tools/run_m3_acceptance.sh \
  --output docs/verification/evidence/m3/formal-$(date +%Y-%m-%d) \
  --allow-blocked
```

若要完成 M3，先補齊 `fixtures/teralion/twse/2330/2026-07-20` 的 authorized
source、daily instrument、metadata 與 cache lineage，再移除 `--allow-blocked`。
