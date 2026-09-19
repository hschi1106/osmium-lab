# 001：移除 TUI 與專用播放負擔

Status: done
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

- [x] `osmium --help` 無 `display`。
- [x] `display` 走一般 usage error，不載入 config/source/terminal。
- [x] TUI modules/exports/dependencies 消失。
- [x] 無 feature flag/alias/shim。
- [x] replay/backtest/inspect offline smoke 通過。
- [x] workspace fmt/test/clippy 通過。
- [x] 現行文件不再宣稱 TUI。
- [x] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：`a340050edef9921e41950476fdfba0ea137364e6`；goal 開始前 worktree clean。
- Rust LOC before：dirty-working-tree file-list cloc：110 files / 3,669 blank / 272 comment / 42,761 code；`crates/osmium-cli`：5 files / 3,058 code。
- 刪除 modules/deps：刪除 `crates/osmium-cli/src/market_replay.rs`（744 lines）與 `market_replay_ui.rs`（422 lines）；移除 `market_replay`／`market_replay_ui` modules、`MarketReplay*` public exports、`display` parser/dispatch/help、專用 `CliError` path 與 TUI tests；移除 `crossterm`、`ratatui` direct dependencies，Cargo lock 同步移除其 transitive-only packages。`replay-engine`、`MarketState` 與一般 replay command 未改動。
- Validation：
  
  - `cargo fmt --all --check`：exit 0。
  - `cargo test -p osmium-cli`：exit 0；18 tests passed。
  - `cargo clippy -p osmium-cli --all-targets -- -D warnings`：exit 0。
  - `cargo build --release --locked -p osmium-cli --bin osmium`：exit 0。
  - release `osmium --help` 輸出不含 `display`。
  - `osmium display --config /definitely/missing/config.yaml`：exit 2，第一行為 `error: unknown command: display`；無 config/source/terminal I/O error，證明未進入 config loading 或 terminal path。
  - `data verify`、`cache prepare`（reuse）、`replay`、`backtest`、`inspect` 與 compiled-strategy offline smoke：皆 exit 0；replay 2 events 的 event checksum `4668d8745908ed6475347c8e6d435fbcf865939a000498adee809f1a4b212860`、final-state checksum `a53c1e969d3b74398384527bf9c73e3ac8d0b30aa42a3c057aa720314c49a02a` 與 goal-000 baseline 相同。
  - `cargo test --workspace`：exit 0；317 passed、56 suites、0 failed。TUI-only tests 移除後核心 replay/backtest coverage 仍由 owner crates 測試與 smoke 覆蓋。
  - `cargo clippy --workspace --all-targets -- -D warnings`：exit 0；`git diff --check`：exit 0。
  - current docs／CLI/dependency scan：`ratatui`、`crossterm`、TUI module/API、`osmium display` 在 current source/docs/traceability 中均無命中；generic Rust `fmt::Display`、`Path::display()` 與 market `displayed_quantity` 未誤刪。
- Rust LOC after / delta：dirty-working-tree file-list cloc：108 files / 3,569 blank / 264 comment / 41,614 code；相較 before code `-1,147`。`crates/osmium-cli`：3 files / 1,911 code，code `-1,147`。
- Breaking changes：`osmium display` 與 `osmium-cli` 的 `MarketReplay`／playback public API 不再存在；`crossterm`／`ratatui` 不再是 workspace dependency。其他 command、config、source/cache、replay 與 artifact contract 不變。
- 剩餘風險：`CHANGELOG.md` 保留歷史版本曾提供 `display` 的事實，未視為現行支援文件；其餘 scope 內未發現 TUI alias、feature flag、shim 或 release check。
- 下一步：002-provider-neutral-validation.md
