# Changelog

## 0.1.0 — public release preparation

- 建立 neutral `osmium-config` 與 `osmium-runner` production boundary。
- 移除 M1/M2/M3 milestone crates 與 legacy config v1 parser。
- 收斂 CLI 至 `config check`、`data sync/verify`、cache、replay、backtest、display、run
  與 inspect workflow。
- 將 fixture builder、歷史 runner、acquisition helpers 與 formal scripts 移到 tools。
- 加入 binary archive packaging、neutral example config、quickstart、config reference、
  local data layout 與 fixture manifest。
- 完成 RLS-06 JSON output/quiet/no-color 與 stable exit categories。
- 加入 synthetic smoke fixture、fixture bundle package/fetch/verify flow，以及
  deterministic archive、offline installer、clean-machine/reproducibility checks。
- archive 現在包含 CycloneDX SBOM 與 transitive third-party license inventory；新增
  [support policy](docs/release/SUPPORT.md)。
- 移除 historical increment／verification／milestone config 與舊 acceptance tooling；完整
  historical evidence 改由 Git history 或 external archive 保存。
- 保留 `examples/config.yaml` 原樣，並以 repository-owned synthetic scenarios 取代
  committed Teralion market data；新增可重建 fixture 的 deterministic generator。
