# Release validation

這是 cleanup 後的短驗證入口。歷史 M1–M5 report 不再放在 repository；本文件只描述
目前 release tree 的可重現 checks 與 private acceptance 邊界。

## Repository checks

```sh
tools/acceptance/verify_compact_fixtures.sh
tools/acceptance/verify_fixture_bundle.sh \
  --bundle . \
  --manifest fixtures/smoke/manifest.yaml
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`verify_compact_fixtures.sh` 會確認：

- manifest entry 全部是 `complete_day: false`，且 metadata 是
  `fixture_scope: representative_slice`。
- TWSE 保留 `2330/2026-07-20`，不存在 `2330/2026-07-27`。
- 每個 JSONL record 可解析且有 `match_time`／`received_at`。
- 每個 session 不超過 512 records／512 KiB，整棵 compact tree 不超過 10 MiB。
- manifest path 與 metadata directory 一致，checksum 與 golden artifact allowlist 正確。

## Smoke acceptance

CI 使用 `examples/smoke.yaml` 與 `fixtures/smoke/` 建立 source/cache，然後在無
credential 的環境執行 `data verify`、`cache prepare`、`replay`、`backtest` 與
`inspect`。這是 synthetic smoke，不是 real market formal acceptance。

## Release archive

```sh
tools/release/package.sh --output target/osmium-internal.tar.gz
tools/release/smoke_clean_machine.sh \
  --archive target/osmium-internal.tar.gz \
  --checksum target/osmium-internal.tar.gz.sha256
SOURCE_DATE_EPOCH=0 tools/release/verify_reproducibility.sh \
  --output target/release-repro
```

archive 不應包含 raw data、`target/`、`.env`、credential 或未授權 full-day payload。

## Private full-day acceptance

需要完整交易日資料時，先由 internal artifact store／SSO 提供 authorized bundle，再
執行：

```sh
tools/acceptance/fetch_fixture_bundle.sh \
  --source https://<internal-artifact-store>/osmium/acceptance.tar.gz \
  --output target/acceptance-bundle
```

`OSMIUM_FIXTURE_BUNDLE_TOKEN` 由 deployment environment 注入。完整日 report 與 provider
authorization audit 是 external deployment evidence，不由 compact repository check
代替。
