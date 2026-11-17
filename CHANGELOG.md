# Changelog

## Unreleased

- 改建為 marketdata-native 架構（[ADR-0007](docs/architecture/decisions/0007-marketdata-native.md)）。
  移除 Teralion provider、`data-sync`（source sync／verify／replay cache）、舊 CLI 與 config loader、
  synthetic Teralion fixtures、acceptance 與 release tooling，以及 AGPL／Strategy Linking Exception。
  `run-planner` 只保留 effective config、session 與 canonical encoding，資料根目錄改為 `md_root`。
  舊版本歷史保留於 Git。
