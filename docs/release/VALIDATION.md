# Release validation

這是 cleanup 後的短驗證入口。歷史 M1–M5 report 不再放在 repository；本文件只描述
目前 release tree 的可重現 checks 與 synthetic／real-data acceptance 邊界。

## Repository checks

```sh
python3 tools/acceptance/generate_synthetic_fixtures.py
git diff --exit-code -- fixtures
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
  `fixture_scope: synthetic_scenario`、`provenance: repository-owned-synthetic`。
- matrix 包含 TWSE／TPEx equity、warrant，以及 TAIFEX future、option scenarios。
- 每個 JSONL record 可解析且有 `match_time`／`received_at`。
- 每個 session 不超過 512 records／512 KiB，整棵 compact tree 不超過 10 MiB。
- manifest path 與 metadata directory 一致，checksum 與 golden artifact allowlist 正確。

## Smoke acceptance

CI 使用 `examples/smoke.yaml` 與 `fixtures/smoke/` 建立 source/cache，然後在無
credential 的環境執行 `data verify`、`cache prepare`、`replay`、`backtest` 與
`inspect`。同一份 cache 另以 `examples/smoke-example-strategy.yaml` 驗證 compiled custom
strategy，預期一筆 order、一筆 fill，並發布含 materialized parameters 的
`strategy.json`。這是 synthetic smoke，不是 real market formal acceptance。

## Release archive

```sh
tools/release/package.sh --output target/osmium-release.tar.gz
tools/release/smoke_clean_machine.sh \
  --archive target/osmium-release.tar.gz \
  --checksum target/osmium-release.tar.gz.sha256
SOURCE_DATE_EPOCH=0 tools/release/verify_reproducibility.sh \
  --output target/release-repro
```

archive 不應包含 raw data、`target/`、`.env`、credential 或未授權 full-day payload。

## External full-day acceptance

需要完整交易日資料時，使用 repository 外、經授權的 user-owned data root 執行相同
verify／cache／replay／backtest gates。credential、authorization evidence、report 與
full-day payload 不提交至 repository 或 binary archive。synthetic matrix 只驗證公開的
wire mapping 與 deterministic workflow，不能取代 full-day acceptance。
