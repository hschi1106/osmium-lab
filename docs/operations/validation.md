# 驗證

## Repository checks

```sh
python3 tools/acceptance/generate_synthetic_fixtures.py
git diff --exit-code -- fixtures
tools/acceptance/verify_compact_fixtures.sh
tools/acceptance/verify_fixture_bundle.sh \
  --bundle . \
  --manifest fixtures/smoke/manifest.yaml
tools/release/verify_license.sh
python3 -m unittest discover -s tools/acceptance -p 'test_*.py'
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

compact fixture verifier 檢查 synthetic provenance、market/instrument matrix、JSONL shape、時間欄位、檔案大小、manifest path 與 checksum。所有 committed scenarios 都標示 `complete_day: false`，不作為完整市場資料代表。

## Stability 驗證邊界

`source_partition.py` 驗證通用 partition integrity，`stability_lifecycle.py` 驗證 provider-neutral observations 的生命週期；兩者的 unit tests 包含合法序列與拒絕案例。各 adapter 的 wire conformance 另外驗證，例如 `verify_teralion_stability.py`。Adapter certification 不取代 domain isolation、execution eligibility 或 Alpha 新舊策略語意差異驗證。執行方式見 [acceptance tooling](../../tools/acceptance/README.md)。

## Smoke flow

CI 使用 `examples/smoke.yaml` 與 `fixtures/smoke/` 準備 source/cache，再於無 credential 環境執行：

```text
data verify -> cache prepare -> replay -> backtest -> inspect
```

`examples/smoke-example-strategy.yaml` 另驗證 compiled strategy registry、materialized parameters、order、fill 與 `strategy.json` publication。

## Release archive

```sh
tools/release/package.sh --output target/osmium-release.tar.gz
tools/release/smoke_clean_machine.sh \
  --archive target/osmium-release.tar.gz \
  --checksum target/osmium-release.tar.gz.sha256
SOURCE_DATE_EPOCH=0 tools/release/verify_reproducibility.sh \
  --output target/release-repro
```

archive 驗證涵蓋 checksum、inventory、SBOM、third-party licenses、離線安裝與 smoke。archive 不得包含 `.env`、credential、`data_root`、raw market payload、`target/` 內容或未授權資料。

## 外部完整日驗證

需要完整交易日證據時，在 repository 外的 user-owned、authorized `data_root` 執行相同 verify/cache/replay/backtest gates。credential、授權文件、完整日 payload 與受限報告不提交至 repository 或 binary archive。
