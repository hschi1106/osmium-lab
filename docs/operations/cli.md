# CLI 操作契約

## 1. 文件目的

本文件定義目前可執行的頂層命令、輸入、輸出與失敗語意。

```text
cli_contract_version = 1
current_scope         = M1 TWSE 2330 replay
binary                = osmium
```

M1 CLI 只把已驗證的本地 fixture 串接至 replay、ExampleStrategy 與 artifact
export。它不下載資料、不讀取 API key、不建立 replay cache，也不提供 order／fill
或 P&L。`plan -> sync -> verify -> backtest -> inspect` 的完整 workflow 屬於 M2。

依據：

- [產品需求](../product-requirements.md)
- [M1 TWSE replay](../increments/M1-twse-replay.md)
- [Verification plan](../verification/plan.md)

## 2. 執行完整 M1 replay

從 repository root 執行：

```sh
cargo run --release -p osmium-cli -- replay \
  --fixture fixtures/teralion/twse/2330/2026-07-27 \
  --output target/m1-replay
```

`--fixture` 指向 trading-date fixture root，該目錄必須包含：

```text
metadata.yaml
regular-quotes/
golden/fixture-set.sha256
```

`--output` 是這次執行要建立的新目錄。為避免不同 run 的結果混在一起，CLI
不接受既有路徑，也沒有 `--force`。需要重跑時應指定另一個目錄，或由使用者明確
處理舊輸出。

## 3. 成功輸出

成功時 stdout 顯示穩定的摘要欄位：

```text
M1 TWSE replay completed
input_records=73796
normalized_events=73795
strategy_callbacks=73795
strategy_output_records=147590
warnings=0
output=target/m1-replay
```

命令在 staging directory 完成所有檔案後，才以 rename 發布至 `--output`：

| 檔案 | 內容 |
| --- | --- |
| `fixture-metadata.yaml` | committed fixture metadata 的 exact copy |
| `fixture-set.sha256` | approved fixture-set SHA-256 |
| `normalized-events.bin` | versioned canonical normalized event stream |
| `event-stream.blake3` | ordered replay event stream checksum |
| `final-state.blake3` | final MarketState set checksum |
| `strategy-output.bin` | ExampleStrategy canonical output |
| `strategy-output.blake3` | canonical strategy output checksum |
| `warnings.yaml` | ordered stable warnings；無 warning 時為空陣列 |
| `run-summary.yaml` | versions、counts、checksums 與 outcome |

`normalized-events.bin` 與 `strategy-output.bin` 是可重現的 canonical binary，
不是逐行人類可讀的 trace。一般檢查先看：

```sh
sed -n '1,200p' target/m1-replay/run-summary.yaml
sed -n '1,200p' target/m1-replay/warnings.yaml
```

binary 的逐 event decode／query 尚未提供，留給後續 `inspect` command。

## 4. 離線與安全邊界

`replay` command：

- 只讀取 `--fixture` 下的 committed local files。
- 不讀取 `.env` 或 `TERALION_API_KEY`。
- 不建立 HTTP client 或呼叫 data-sync。
- 不讀取 `raw/`、`archive/` 或 `derived/`。
- 不把 credential、wall-clock duration 或 machine-specific absolute fixture path
  寫入 artifacts。

這些程式邊界讓 command 可以在 network-disabled container 執行；正式
`M1-T053` 仍必須由該環境的 CI evidence 證明。

## 5. 錯誤與 exit status

| Exit status | 語意 |
| --- | --- |
| `0` | replay 與完整 artifact publish 成功，或只顯示 help |
| `1` | fixture、normalization、replay、strategy 或 artifact I/O 失敗 |
| `2` | command／option 使用錯誤 |

錯誤輸出至 stderr，並包含 fixture root 或 artifact output path。失敗時不會把
staging directory rename 成成功輸出；staging cleanup 採 best effort。

顯示 help：

```sh
cargo run -p osmium-cli -- --help
```
