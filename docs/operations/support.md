# 支援政策

## 支援範圍

- target-specific binary archive 與 installer。
- `config_version: 2` RunConfig。
- 文件列出的 Teralion source sync、verify、cache、offline replay/backtest、TUI 與 inspect workflow。
- repository-owned synthetic fixtures 與另行授權的外部資料驗證流程。

不包含：

- crates.io 或第三方 Rust library API stability。
- 未經授權的 Teralion payload轉發。
- 即時交易、完整交易所撮合、queue position 或來源無法支持的市場語意。
- 未列入 interface registry 的 format／instrument profile。

## 問題回報

請附上：

```text
osmium version
archive target
command and exit category
是否關閉網路
source/cache/run identity（不得包含 credential）
可重現的最小設定或 synthetic fixture
```

不要在公開 issue 提交 API key、cookie、signed URL、受限 raw payload 或個人資料。security／integrity 問題請使用 repository Security 頁面的 private reporting channel。

## 問題分級

| 等級 | 定義 |
| --- | --- |
| S0 | credential 外洩、archive 被竄改或結果可能使用未來資料 |
| S1 | binary 無法啟動、完整性判定錯誤或 determinism 破壞 |
| S2 | 已支援 workflow 的可重現功能或資料驗證錯誤 |
| S3 | 文件、UX、非阻斷警告或新功能建議 |

每次發布重新執行 workspace tests、archive checksum、clean-machine install 與 reproducibility gate。一般維護以目前發布版本為準；資料提供者契約與 redistribution 權限不因本專案的支援政策而改變。
