# Release namespace review

截至 2026-08-03，production source 的 milestone naming review 已完成。release binary、
config、runner、public error variant 與 production help 不再以 M1/M2/M3 作為主要
identity。

以下 references 是刻意保留且不屬於 release public identity：

- `config/m3-*`、`config/m4-*`、`config/m5-*` 與 `crates/**/tests` 中的 historical
  acceptance inputs；它們維持 formal evidence 與 checksum 的可追溯性。
- `docs/increments/`、`docs/verification/evidence/` 與 acceptance tooling 中的 M1–M5
  名稱；它們是歷史 scope、fixture provenance 或 maintainer-only harness。
- `OSM3STRATEGY` 與 `OSM5PARAM`；它們是既有 domain identity digest tags，保留可避免
  改名造成結果 checksum 無意變更，不是 user-facing package 或 CLI identity。
- `legacy config_version: 1` 的 rejection message 與 test；它們明確表達不提供 M2
  compatibility，不能解讀成 legacy parser 仍存在。

本 review 不改寫既有 fixture、acceptance report 或 provenance checksum；後續新增
production code 必須使用 neutral naming，若需要 historical reference，應在 test、
acceptance 或 migration evidence 邊界內明確標示。
