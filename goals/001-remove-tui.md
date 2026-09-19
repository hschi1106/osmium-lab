# 001：移除 TUI 與專用播放負擔

Status: pending
Depends on: 000-baseline-and-inventory.md

## 目標

徹底移除互動式行情 TUI、`osmium display` 與僅為它存在的播放控制、依賴、測試與文件。

保留非互動式 `init`、`config check`、`plan`、`data sync/verify`、`cache prepare`、`replay`、`backtest`、`run`、`inspect`、`version`。

不保留 deprecated command、feature flag、alias 或替代 UI。

## 執行前核對

已知候選：

```text
crates/osmium-cli/src/market_replay_ui.rs
crates/osmium-cli/src/market_replay.rs
ratatui
crossterm
display parser / dispatch / help
TUI docs / traceability / release checks
```

實作前重新搜尋 caller；不要依檔名盲刪。

## 必須做

1. 刪 `display` parse/dispatch/help/public API。
2. 刪 terminal control、keyboard、wall-clock playback、pause/speed、TUI history/chart。
3. 刪只因 TUI 存在的 direct dependencies。
4. 刪只驗 TUI 的 tests/fixtures/tools。
5. 若 TUI test 是核心 replay behavior 唯一 coverage，把 behavior 搬回真正 owner。
6. 更新 PRD、README、architecture、CLI、support、traceability。
7. `display` 走一般 unknown-command usage error，不留專用 tombstone。

## 不做

- 不改 deterministic replay / MarketState / simulation / accounting。
- 不改 source/cache format。
- 不新增 Web UI/GUI/renderer abstraction。
- 不誤刪 `fmt::Display`、`Path::display()` 或一般行情資料。

## LOC

依 README 記 before/after。此 goal 預期 production Rust LOC 明顯下降。

## 驗收

- [ ] `osmium --help` 無 `display`。
- [ ] `display` 走一般 usage error，不載入 config/source/terminal。
- [ ] TUI modules/exports/dependencies 消失。
- [ ] 無 feature flag/alias/shim。
- [ ] replay/backtest/inspect offline smoke 通過。
- [ ] workspace fmt/test/clippy 通過。
- [ ] 現行文件不再宣稱 TUI。
- [ ] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- 刪除 modules/deps：
- Validation：
- Rust LOC after / delta：
- Breaking changes：
- 剩餘風險：
- 下一步：002-provider-neutral-validation.md
