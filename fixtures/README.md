# Fixtures

本 repository 的 fixture 全部是自行建立的 synthetic scenarios：

- `smoke/` 是最小的 CLI、cache 與 backtest smoke payload。
- `teralion/` 是 Teralion wire shape 的 synthetic coverage matrix，涵蓋不同 market、
  instrument kind、session 與 source state。

兩者皆由 repository 擁有，可公開散布，不包含、抽樣或轉換任何真實市場行情。
`acceptance/manifest.yaml` 記錄 synthetic payload 的 checksum 與 coverage。

重新產生並檢查 synthetic tree：

```sh
python3 tools/acceptance/generate_synthetic_fixtures.py
tools/acceptance/verify_compact_fixtures.sh
```

實際 Teralion data 只可存在於 repository 外的 user-owned data root。
