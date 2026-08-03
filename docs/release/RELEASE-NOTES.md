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
- acceptance fixture builder 與 formal harness 不包含在 binary archive；需依 fixture
  manifest 及 authorization 另行取得。

## Compatibility

binary、run config、event/cache 與 accounting versions 必須依 archive 中的文件與
`osmium version` 檢查。crate rename 不代表 domain checksum 自動相容；run artifacts
仍保存 source/cache lineage 與 checksum。

## Known release gates

clean-machine installation、private fixture authorization、完整 JSON output contract、
SBOM/license inventory 與 installer workflow 仍由後續 release gate 管理。
