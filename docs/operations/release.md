# 發布

`osmium` 以 public source repository 與 target-specific binary archive 發布。Rust workspace crates 的公開原始碼可供檢查，但不承諾 crates.io distribution 或穩定的第三方 library API。

## 發布內容

- `osmium` binary 與 installer。
- quickstart、config、data layout、release 與 support 文件。
- deterministic `SHA256SUMS`。
- CycloneDX `SBOM.cdx.json`。
- `THIRD-PARTY-LICENSES.txt`。
- repository-owned synthetic smoke／acceptance fixtures只存在 source tree；binary archive 不包含 fixture builder 或受限 payload。

## 相容性

使用 `osmium version` 檢查 product、CLI、config、run manifest、event、cache 與 accounting identity。artifact 是否相容由其 descriptor／manifest 中的直接輸入與版本決定，不以 crate 名稱或檔案路徑推定。

cache 不相容時可由 compatible verified source 重建。run artifacts 不會就地 migration 或覆寫；需以目前 binary、設定與資料建立新 run directory。

## 部署前提

- 執行 [repository 與 archive 驗證](validation.md)。
- 以 clean machine 驗證安裝、`osmium version` 與 offline smoke。
- 保留 archive 與外部 checksum。
- full-day acceptance 使用 repository 外、經授權的 data root。
- 不在 archive 中加入 credential、raw dump 或未授權市場資料。

目前版本資訊與使用入口見 repository 根目錄的 `README.md`。
