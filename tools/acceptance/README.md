# Acceptance tooling

這個目錄只放 maintainer／CI 使用的資料工具，不是 release `osmium` runtime API。

| Tool | 用途 |
| --- | --- |
| `generate_synthetic_fixtures.py` | 從自行撰寫的 scenarios 產生 deterministic synthetic fixtures |
| `verify_compact_fixtures.sh` | 檢查 repository synthetic fixture 的 provenance、大小、JSON 與 checksum |
| `source_partition.py` | Provider-neutral source partition integrity reader；只驗證 pointer、manifest、object checksum 與 record count，不解讀 wire fields |
| `stability_lifecycle.py` | Provider-neutral stability lifecycle validator；只接受 adapter 已分類的 observations |
| `export_teralion_semantic_fixture.py` | 從 verified Teralion partition 匯出精確、可重現的 provider-neutral stability observations，供外部策略 differential 使用 |
| `verify_teralion_stability.py` | Teralion adapter conformance：將 TWSE／TPEx wire fields 分類後交給 generic lifecycle validator |
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

若要稽核真實 TWSE／TPEx stability sequence，先用包含兩個商品的 config 執行
`osmium data verify`，再把兩個已發布 partition root 傳入 verifier：

```sh
python3 tools/acceptance/verify_teralion_stability.py \
  --twse-partition "$OSMIUM_VALIDATION_DATA_ROOT/source/teralion/twse/2026-08-10/3026" \
  --tpex-partition "$OSMIUM_VALIDATION_DATA_ROOT/source/teralion/tpex/2026-08-10/2948"
```

`OSMIUM_VALIDATION_DATA_ROOT` 應指向使用者管理、repository 外的資料目錄。工具需要系統
`zstd`。Generic partition reader 會重驗 tick pages 的 compressed／uncompressed checksums 與
manifest count；Teralion adapter 才解讀 `STOCK_REALTIME`、`STOCK_SNAPSHOT`，再由
provider-neutral lifecycle validator 檢查 trigger、trial、正式 call-auction result
與 continuous resume。stdout 只輸出 partition identity、事件時間、筆數及 canonical JSON digest；
不輸出價格、數量、book 或 API credential。此報告是 source evidence，不取代完整的
`osmium data verify`，也不會寫入 fixture 或 source partition。

來源 observations 匯出範例：

```sh
python3 tools/acceptance/export_teralion_semantic_fixture.py \
  --partition "$OSMIUM_VALIDATION_DATA_ROOT/source/teralion/twse/2026-08-10/3026" \
  --case-id twse-stability-up \
  --output "$OSMIUM_VALIDATION_DATA_ROOT/twse-stability-up.json"
```

Exporter 位於 Teralion adapter boundary，consumer 不解析 provider flags。它保留完整 priced
levels、市價委託量與沒有成交的 observation；numeric lexeme 不經 binary float。可選的 identity、
time、price、quantity transforms 會寫入 provenance；quantity scale 同時作用於 cumulative volume，
不破壞正式成交增量關係。每次逐頁重驗 source checksum，並限制 classified observation 數量。

`source_record_sha256` 使用 `canonical_json_numeric_lexemes_v1`，不是原始 bytes checksum，
也不能直接與既有 adapter certification 的 record digest 比較。原始 revision identity／page
checksums 仍是 source integrity 的依據。輸出只是來源轉換證據；非零 strategy intents、fills、fees
與 PnL 必須由兩版 production strategy／execution differential 另行驗證。
