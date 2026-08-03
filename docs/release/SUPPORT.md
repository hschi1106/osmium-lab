# osmium 支援政策

## 支援範圍

`osmium` 是公開原始碼工具，支援範圍固定為：

- target-specific binary archive／installer。
- `config_version: 2` 的 `RunConfig`。
- 已完成 `data sync`、`data verify` 與 `cache prepare` 的離線 replay/backtest。
- archive 內列出的 target、Rust toolchain 相容性與 release 文件。
- smoke fixture 與另行授權的 acceptance bundle 驗證流程。

不在支援範圍內：

- `config_version: 1`、M2 compatibility 或 legacy parser。
- crates.io、第三方 Rust library API 或自行替換 production crate 的 ABI/API。
- 未經授權的 Teralion payload、repository fixture path 或外部資料轉發。
- 即時交易、完整交易所撮合、queue position 或由低粒度資料推導的市場語意。

## 版本與相容性

使用者回報問題時，必須附上：

```text
osmium version
archive version/target
config schema version
command and exit category
是否 network-disabled
source/cache/run identity（不得包含 credential）
```

同一個 major release 只保證當前文件所列的 config、event/cache 與 artifact schema。
schema 或 accounting identity 變更時，release notes 必須說明 migration 或重新準備
資料的要求；不以 crate rename 自動宣稱 artifact 相容。

## 問題分級

| 等級 | 定義 | 目標回應 |
| --- | --- | --- |
| S0 | credential 外洩、archive 被竄改、結果可能使用未來資料 | 立即停止散布並通知 maintainer |
| S1 | release binary 無法啟動、完整 source/cache 被錯誤接受、determinism 破壞 | 下一個 release gate 前處理 |
| S2 | 已支援 workflow 的可重現功能錯誤或資料驗證錯誤 | 排入近期 maintenance release |
| S3 | 文件、UX、非阻斷警告或新功能建議 | 依容量排程 |

security／integrity 問題不要在 public issue 貼出 API key、cookie、signed URL、raw
payload 或受授權限制的 fixture；請使用 repository Security 頁面提供的 private reporting
channel，並附 sanitized command、exit category 與可安全分享的 checksum。

## 維護政策

每個 release 都必須重新執行 `cargo test --workspace`、archive checksum、clean-machine
install 與 reproducibility gate。受授權限制 fixture 的 redistribution approval 到期或
scope 改變時，立即停止對應 bundle 的下載，不回填到 binary archive。

未另行公告時，只有最新一個 release 接受一般維護；舊版仍可被歷史 acceptance
重現，但不保證取得新的 fixture、toolchain 或 provider endpoint。此政策不改變資料提供者
契約，也不授予 fixture 對外散布權。
