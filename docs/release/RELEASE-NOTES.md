# osmium 0.1.0 internal release

## Distribution

此版本只供 private/internal distribution。交付物是 target-specific binary archive；Rust
crates 是 internal implementation boundary，不承諾 crates.io library API。

## User-visible boundary

- release CLI 使用 `osmium`、`RunConfig` 與 `config_version: 2`。
- `config_version: 1` 不提供 migration，會以 upgrade error 拒絕。
- source acquisition 使用 `osmium data sync`；verify、cache、replay、backtest 與 inspect
  在資料準備後可離線執行。
- `osmium display` 是只讀 v2 historical-market TUI。
- non-interactive command 支援 `--format human|json`、`--quiet` 與 `--no-color`；exit
  categories 為 usage/config/source/cache/replay/simulation/integrity/internal。
- acceptance fixture builder 與 formal harness 不包含在 binary archive；需依 fixture
  manifest、checksum 與 authorization 另行取得。`tools/acceptance/fetch_fixture_bundle.sh`
  會驗證下載 archive 後才發布 local bundle。
- repository 只保留 compact representative real-data slices；它們的 metadata 固定標示
  `complete_day: false`。TWSE fixture 保留 `2330/2026-07-20`，`2026-07-27` 已移除。
- `fixtures/acceptance/manifest.yaml` 仍定義 private full-day bundle 的 access boundary；
  `tools/acceptance/verify_compact_fixtures.sh` 是目前 repository fixture gate。
- archive 包含 deterministic `SHA256SUMS`、CycloneDX `SBOM.cdx.json` 與
  `THIRD-PARTY-LICENSES.txt`；`tools/release/install.sh` 支援無網路安裝。
- 支援政策見 [SUPPORT.md](SUPPORT.md)。

## Compatibility

binary、run config、event/cache 與 accounting versions 必須依 archive 中的文件與
`osmium version` 檢查。crate rename 不代表 domain checksum 自動相容；run artifacts
仍保存 source/cache lineage 與 checksum。

## Deployment prerequisites

- private artifact store／SSO 必須提供 HTTPS bundle URL 與 bearer token injection；repository
  只提供 provider-neutral flow，不保存實際 credential 或 URL。
- TPEx warrant 與其他 M5 acceptance payload 仍是 `private-internal-review-only`，不能
  放入 unrestricted archive。
- 每次部署仍需執行 clean-machine install、offline smoke 與 reproducibility gate，並保留
  archive/external checksum。
