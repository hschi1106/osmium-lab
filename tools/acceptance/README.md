# Acceptance tooling

這個目錄只放 maintainer／CI 使用的資料工具，不是 release `osmium` runtime API。

| Tool | 用途 |
| --- | --- |
| `compact_fixture_data.py` | 從 private full-day tree 產生 deterministic representative slices |
| `verify_compact_fixtures.sh` | 檢查 repository compact fixture 的 scope、日期、大小、JSON 與 checksum |
| `verify_fixture_bundle.sh` | 驗證 manifest entry、payload JSON、record count、checksum 與 secret 欄位 |
| `package_fixture_bundle.sh` | 依 manifest 打包 authorized fixture bundle |
| `fetch_fixture_bundle.sh` | 從 local archive／directory 或 bearer-token HTTPS source 取得 bundle |
| `osmium_fixture_data/` | 以 fixture transport 建立 local source/cache，供 offline acceptance 使用 |

compact builder 的 input 必須是 maintainer-owned private full-day fixture；輸出的
`fixtures/teralion/` 只可標示 `representative_slice`／`complete_day: false`。builder
不會改變 production normalizer、replay 或 simulation code。

常用檢查：

```sh
tools/acceptance/verify_compact_fixtures.sh
tools/acceptance/verify_fixture_bundle.sh \
  --bundle . \
  --manifest fixtures/acceptance/manifest.yaml
```

完整日 formal acceptance report、provider authorization 與 access token 不提交到
repository；它們由 private artifact store 保存。
