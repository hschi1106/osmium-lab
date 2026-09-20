# 驗證

本頁定義 current repository 與 release acceptance commands；不保存一次性 migration log 或外部
user-owned payload identity。

## Rust quality gates

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

只有 command 實際以 exit 0 完成才可宣稱 PASS。Focused crate tests 可先跑，但不能取代 final workspace
gate。

## Synthetic fixtures 與 acceptance tooling

```sh
python3 tools/acceptance/generate_synthetic_fixtures.py
git diff --exit-code -- fixtures
tools/acceptance/verify_compact_fixtures.sh
tools/acceptance/verify_fixture_bundle.sh \
  --bundle . \
  --manifest fixtures/smoke/manifest.yaml
python3 -m unittest discover -s tools/acceptance -p 'test_*.py'
tools/release/verify_license.sh
```

Generator 後 fixtures 必須無 diff，證明 committed outputs 可重現。Compact verifier 檢查 synthetic
provenance、market/instrument matrix、JSONL shape、timestamps、size、manifest path 與 checksum。
Committed scenarios 都是 `complete_day: false`，只固定 contract cases，不代表完整市場日。

`source_partition.py` 驗 provider-neutral partition integrity；`stability_lifecycle.py` 驗 neutral
observations；market adapter另由 wire conformance checker（例如 `verify_teralion_stability.py`）驗證。
詳細工具參數見 [`tools/acceptance/README.md`](../../tools/acceptance/README.md)。

## Fresh offline smoke

先 build release CLI 與 fixture helper：

```sh
cargo build --release --locked -p osmium-cli --bin osmium
cargo build --release --locked \
  --manifest-path tools/acceptance/osmium_fixture_data/Cargo.toml
```

使用 `examples/smoke.yaml` 與 `fixtures/smoke/` 在新的 temporary data/output roots 執行：

```text
config check
→ plan
→ data verify
→ cache prepare
→ replay
→ backtest
→ inspect
```

移除 `TERALION_API_KEY` 後執行，證明 source 完成後不需 network/credential。
`examples/smoke-example-strategy.yaml` 另驗 compiled registry、materialized parameters、order/fill 與
`strategy.json`。若文件將 `run` 列為 user workflow，也必須在 fresh root測一次有效 `run`。

Determinism 以同一 final version、config、source 與 clean derived roots重跑，event、final-state、
strategy output、ledger/run checksums 應一致。Version/mapping identity故意改變時，不能只更新 expected
checksum；需先證明 change 是 identity/semantics預期結果，再建立新 baseline。

## Documentation checks

- 執行 current `osmium --help`、`version` 與各 command `--help` 核對 command/flags。
- 驗證 Markdown relative links、source path、example config path 與 cross-reference 存在。
- 搜尋 stale API/version/removed interface 名稱；historical changelog 不視為 current contract。
- Docs-only change必須確認 `git diff -- crates` 與 Cargo manifests/lockfile為空。

## Release archive

```sh
tools/release/package.sh --output target/osmium-release.tar.gz
tools/release/smoke_clean_machine.sh \
  --archive target/osmium-release.tar.gz \
  --checksum target/osmium-release.tar.gz.sha256
SOURCE_DATE_EPOCH=0 tools/release/verify_reproducibility.sh \
  --output target/release-repro
```

Archive checks涵蓋 inventory、SHA-256、SBOM、third-party licenses、offline installation 與 smoke。
Archive 不得含 `.env`、credential、user data root、raw market payload、unrelated `target/` files 或
未授權資料。

## 外部完整日驗證

需要完整日證據時，在 repository 外的 authorized user-owned data root執行相同
verify/cache/replay/backtest gates。Credential、license entitlement、完整日 payload 與受限報告不提交
repository 或 release archive。
