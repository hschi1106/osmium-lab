# 授權

## Osmium core

本 repository 的 Osmium core 採用 GNU Affero General Public License version 3，且只指定
`AGPL-3.0-only`。完整條文位於根目錄 [`LICENSE`](../../LICENSE)。Cargo package metadata 使用
SPDX expression `AGPL-3.0-only`；本 repository 的 custom exception 沒有標準 SPDX exception
identifier，因此其法律效力以 `LICENSE` 中的完整文字為準。

Osmium core 包含 repository-owned 的 Cargo workspace crates、CLI、runner、normalizer、
simulation、replay、data tooling 與其他未另行聲明的程式碼。第三方 dependencies 依其各自
license；Teralion raw payload、user-owned source data、provider material、商標與其他第三方
內容不會因本 repository license 而取得新的散布權。

## Strategy Linking Exception

Exception 是 AGPL section 7 的 additional permission，適用範圍如下：

- 使用者自行撰寫、實作公開 `strategy-api` 的 trading strategy，可以使用任意 license，
  包括 proprietary／closed-source license。
- strategy 及其只透過公開 Strategy API 撰寫的獨立 adapter／registration glue，可以
  compile、link、register 到 Osmium binary；單純因為這個 compile／link／register 關係，
  不要求 strategy 遵守 AGPL。
- Exception 僅適用於獨立撰寫、未複製 Osmium core、未修改 Osmium core 的 strategy work。
- Exception 不涵蓋對 Osmium core 的修改、複製或衍生實作；modified core work 仍須依
  AGPL-3.0-only 發布。

完整、具法律效力的 exception 文字只以 [`LICENSE`](../../LICENSE) 為準。若本文件與
`LICENSE` 有任何差異，以 `LICENSE` 為準。
