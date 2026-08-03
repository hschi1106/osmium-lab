# Release cleanup：production repository 邊界

本文件是目前 release tree 的 cleanup contract。它描述刪除哪些 milestone material、保留
哪些 production boundary，以及 compact fixtures、CLI 與外部 acceptance bundle 的責任
分界。產品範圍與 domain 規則仍以 [product requirements](../product-requirements.md) 為
準。

## 已固定的 release decisions

1. 首個 release 採 private/internal distribution，交付 access-controlled binary
   archive／installer。
2. 不維護 M2 `config_version: 1` compatibility。release 只接受目前的 v2 schema；v1
   直接以 upgrade error 拒絕。
3. binary `osmium` 是 user-facing product；Rust crates 只維持 internal implementation
   boundary，不承諾 crates.io 或穩定的第三方 library API。

這些 decisions 不會改變核心 replay、market state、normalizer、simulation 或 source/cache
語意。

## Release tree

```text
crates/
  osmium-cli/ osmium-config/ osmium-runner/
  data-sync/ run-planner/ replay-engine/
  market-types/ market-state/ strategy-api/ execution-sim/
  normalizer/{twse,tpex,taifex}/

examples/
  config.yaml                 # 保留原檔名與內容，user-facing v2 example
  smoke.yaml                  # synthetic fixture 的 CI config

fixtures/
  smoke/                      # 小型 synthetic、可在 CI 使用
  acceptance/manifest.yaml    # private bundle metadata
  teralion/                   # compact representative real-data slices

tools/
  acceptance/                 # fixture builder、bundle、compact verifier
  release/                    # archive、installer、SBOM/license、reproducibility

docs/
  product-requirements.md     # product source of truth
  requirements/ interfaces/ design/ operations/
  release/                    # release contract 與 validation
```

## Cleanup policy

### 已刪除

- `config/` 下所有 milestone-specific YAML。release example 維持在
  `examples/config.yaml`，不改名成 `representative.yaml`。
- `docs/increments/`、`docs/verification/` 與舊的 namespace review。它們是 historical
  planning／evidence，不是 release usage surface；需要追溯時從 Git history 或外部
  acceptance archive 取得。
- `tools/acquisition/` 與舊的 M1/M3/M4/M5 acceptance runner／verification scripts。
  release tree 不再保留 milestone-specific acquisition workflow。
- `tools/acceptance/osmium_m1_runner/`。release binary 不提供 fixture mode；目前的
  acceptance-only builder 與 generic bundle tools 保留在 `tools/acceptance/`。
- real fixture 的 full-day payload、舊 golden artifacts 與不必要的 shard。

### 保留／新增

- production crates 與現有 Rust tests；cleanup 不改變核心函式行為。
- `examples/config.yaml` 原樣保留；`examples/smoke.yaml` 作為 CI/developer smoke
  config。
- `fixtures/smoke/` 的 synthetic payload。
- `fixtures/acceptance/manifest.yaml`、compact fixture builder、bundle package/fetch/
  verify tooling。
- `docs/release/VALIDATION.md`、`fixtures/README.md`、`fixtures/teralion/README.md` 與
  `tools/acceptance/README.md`，作為新的短入口。

## Compact fixture contract

`fixtures/teralion/` 只保留能代表不同 market、instrument kind、session 與 source state
的 slices；每個 profile 都必須有 `metadata.yaml`、`daily.json`、JSONL payload 與
`golden/fixture-set.sha256`。`metadata.yaml` 必須標示：

```yaml
fixture_scope: representative_slice
complete_day: false
```

目前保留矩陣：

| Market | Symbol | Date | Representative scope |
| --- | --- | --- | --- |
| TWSE | `2330` | `2026-07-20` | equity regular，snapshot/realtime、opening／closing states |
| TWSE | `03003T` | `2026-07-20` | warrant regular |
| TPEx | `6488` | `2026-07-20` | equity regular |
| TPEx | `72328U` | `2026-07-20` | warrant regular |
| TAIFEX | `CAFH6` | `2026-07-20` | futures regular |
| TAIFEX | `CDFH6` | `2026-07-20` | futures after-hours + regular |
| TAIFEX | `TXFH6` | `2026-07-20` | futures cross-session |
| TAIFEX | `TXFH6` | `2026-07-28` | futures／underlying relationship |
| TAIFEX | `TXO24000U6` | `2026-07-28` | option after-hours + regular |

TWSE 的 repository fixture 明確保留 `2330/2026-07-20`；`2330/2026-07-27` 不屬於
release tree。compact verifier 會檢查日期、manifest、metadata、JSON、checksum、每個
session 的 record/byte budget，以及不得混入 full-day golden artifact。

完整日 real-data acceptance payload 不放入 Git，也不放入 binary archive。它由
`fixtures/acceptance/manifest.yaml` 的 private/internal authorization policy 管理，
需要時透過 `fetch_fixture_bundle.sh` 取得。compact slices 不能被宣稱為完整日資料，也
不能替代 provider authorization review。

## CLI boundary

正式文件只使用以下 command surface：

```text
osmium version
osmium init [--path <config.yaml>]
osmium config check --config <file>
osmium plan --config <file>
osmium data sync --config <file>
osmium data verify --config <file>
osmium cache prepare --config <file>
osmium replay --config <file>
osmium backtest --config <file> --output <new-directory>
osmium display --config <file>
osmium inspect --run <run-directory>
```

`replay`、`backtest`、`display` 不會在 runtime 隱式建立 HTTP client；資料準備完成後可
在無 credential／無網路環境執行。`display` 只讀取 explicit universe，並使用既有
`match_time` replay clock；它不新增 strategy、撮合或 queue-position 語意。

non-interactive commands 的 JSON、quiet、color 與 exit-category contract 見
[CLI operations](../operations/cli.md)。latency config 仍由 `examples/config.yaml` 的
`market_data_latency_ms` 與 `order_latency_ms` 示範，缺省為 0 ms。

## Archive boundary

```text
osmium-<version>-<target>/
  bin/osmium
  examples/config.yaml
  docs/quickstart.md
  docs/config-reference.md
  docs/data-layout.md
  RELEASE-NOTES.md SUPPORT.md
  SBOM.cdx.json THIRD-PARTY-LICENSES.txt
  BUILD-METADATA DEPENDENCIES.txt SHA256SUMS
  fixture-manifest.yaml       # metadata only
```

archive 不包含 `raw/`、`target/`、`.env`、API key、credential-bearing URL、未授權的
full-day payload 或 machine-specific absolute path。`tools/release/` 負責 deterministic
archive、offline installer、clean-machine smoke、SBOM/license inventory 與 reproducibility
gate。

## Cleanup completion checklist

- [x] production workspace 不含 `m1-*`、`m2-*`、`m3-*` crate／public type。
- [x] current v2 config／runner 使用 neutral crate identity；v1 不再相容。
- [x] `examples/config.yaml` 保留原檔名與內容。
- [x] historical increment、verification、milestone config 與舊 acceptance tooling
      不再佔用 release tree。
- [x] real fixtures 縮成 representative slices；TWSE 保留 0720、移除 0727。
- [x] compact fixture manifest、checksum 與 verifier 可在無網路環境執行。
- [x] README、CLI operations、CI 與 release validation 不再依賴已刪除路徑。
- [ ] private artifact store／SSO provider-side authorization review（部署責任）。
- [ ] 如需完整日 formal acceptance，從 private bundle 重新取得並保存外部 report。

驗證入口為 [Release validation](VALIDATION.md)。
