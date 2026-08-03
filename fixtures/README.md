# Fixtures

本 repository 的 fixture 分成兩層：

- `smoke/` 是小型 synthetic payload，允許在 CI、developer machine 與 internal binary
  archive 中使用。它不代表任何真實市場交易日。
- `teralion/` 是從 private acceptance data 保留的 compact representative slices，
  用來涵蓋不同 market、instrument kind、session 與 source state。它不是完整交易日，
  也不代表 full-day acceptance 已在 repository 內完成。

`acceptance/manifest.yaml` 是 private bundle metadata。完整日 payload、provider
authorization 與 formal evidence 由 external/private bundle 管理，不進 Git 或 binary
archive。

檢查目前 compact tree：

```sh
tools/acceptance/verify_compact_fixtures.sh
```

compact slices 的矩陣與保留規則見
[fixtures/teralion/README.md](teralion/README.md)。TWSE 保留 `2330/2026-07-20`，
`2330/2026-07-27` 已從 release tree 移除。
