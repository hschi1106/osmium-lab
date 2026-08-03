# Acceptance tooling

這個目錄只放 maintainer／CI 使用的資料工具，不是 release `osmium` runtime API。

| Tool | 用途 |
| --- | --- |
| `generate_synthetic_fixtures.py` | 從自行撰寫的 scenarios 產生 deterministic synthetic fixtures |
| `verify_compact_fixtures.sh` | 檢查 repository synthetic fixture 的 provenance、大小、JSON 與 checksum |
| `verify_fixture_bundle.sh` | 驗證 manifest entry、payload JSON、record count、checksum 與 secret 欄位 |
| `package_fixture_bundle.sh` | 依 manifest 打包 authorized fixture bundle |
| `fetch_fixture_bundle.sh` | 從 local archive／directory 或 bearer-token HTTPS source 取得 bundle |
| `osmium_fixture_data/` | 以 fixture transport 建立 local source/cache，供 offline acceptance 使用 |

generator 不讀取外部資料；輸出的 `fixtures/teralion/` 只可標示
`synthetic_scenario`／`complete_day: false`／`repository-owned-synthetic`。generator
不會改變 production normalizer、replay 或 simulation code。

常用檢查：

```sh
python3 tools/acceptance/generate_synthetic_fixtures.py
tools/acceptance/verify_compact_fixtures.sh
tools/acceptance/verify_fixture_bundle.sh \
  --bundle . \
  --manifest fixtures/acceptance/manifest.yaml
```

完整日 formal acceptance 使用 repository 外的 user-owned data root；credential 與
market payload 不提交到 repository。
