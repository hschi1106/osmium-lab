# Changelog

## 0.1.0 — internal release preparation

- 建立 neutral `osmium-config` 與 `osmium-runner` production boundary。
- 移除 M1/M2/M3 milestone crates 與 legacy config v1 parser。
- 收斂 CLI 至 `config check`、`data sync/verify`、cache、replay、backtest、display、run
  與 inspect workflow。
- 將 fixture builder、歷史 runner、acquisition helpers 與 formal scripts 移到 tools。
- 加入 binary archive packaging、neutral example config、quickstart、config reference、
  local data layout 與 private acceptance manifest。
- 保留 M1–M5 historical evidence，不把 acceptance payload 放入 binary archive。
