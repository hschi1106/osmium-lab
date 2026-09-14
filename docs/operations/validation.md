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

## 2026-09-14 migration 驗收記錄

核心 revision `964831b2dfc2baab0a6eafa801e1cbffd1bb274d` 通過 320 項 workspace tests、
fmt、Clippy、四個 benchmarks 的 checksum assertions 與 clean-machine smoke。
TWSE 3026／TPEx 2948 的 2026-08-10 verified partitions 分別為：

- `44a8468b3056485a0e5009bb11cb04cf63cd7d47d76770344cc9d02277beee67`
- `d55cba9ce2e2d833f5227993bfd5b0dfdb7ad324d7d584bdefd3e6e120ad649d`

相同 source 完成 cache v3 rebuild／reuse、3,403 events replay、4 orders／4 fills backtest 與 inspect。
Event checksum 為 `6ef7cd0bf7bcfb6b7cb976d5cf458308d7cf19416ab1ca65b8337faa576a63ed`；
final-state checksum 為 `b16d7f36ded1dd6bc191a78bc510f7f5632be4cd9cebf6f1021b8123a65407d0`。
Source revisions 未變；raw payload 與 run artifacts 由使用者於 repository 外保存。

Alpha 正式 git pin 的 53 tests、fmt、Clippy 通過。TWSE Up、TPEx Up 與 synthetic Down
完成 production strategy／runner differential；僅 firm isolation、市價委託量表示與版本化 checksums
屬預期差異。完整 runner 財務比較使用明確的 synthetic clean-fill overlay，不代表真實完整日績效；
每版本重跑 trace 相同。一次性 harness 保留於 Alpha revision `7e51dce`，Lab 匯出工具與歷史
計畫保留於 `4ec928f`；日常回歸由目前 workspace tests 與 adapter verifier 負責。

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
