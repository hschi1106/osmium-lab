# osmium-lab

Rust market replay 與 backtesting platform。目前的 M2 reference workflow 支援
Teralion TWSE 2330 單日資料同步、verified local source、derived replay cache、
離線策略回測與 result inspection。

## 簡單執行範例

第一次執行需要以環境變數提供 Teralion API key：

```sh
export TERALION_API_KEY="<your-key>"

cargo run -p osmium-cli -- plan \
  --config config/m2-twse-2330.yaml
cargo run -p osmium-cli -- sync \
  --config config/m2-twse-2330.yaml
cargo run -p osmium-cli -- verify \
  --config config/m2-twse-2330.yaml
cargo run -p osmium-cli -- run \
  --config config/m2-twse-2330.yaml \
  --output target/m2-run
```

source 與 cache 準備完成後，`backtest` 與 `inspect` 不需要網路或 API key：

```sh
unset TERALION_API_KEY

cargo run -p osmium-cli -- backtest \
  --config config/m2-twse-2330.yaml \
  --output target/m2-offline-run
cargo run -p osmium-cli -- inspect \
  --run target/m2-offline-run
```

每次 `--output` 必須指定尚不存在的新目錄。完整需求與驗收結果請見
[產品需求](docs/product-requirements.md)及
[M2 reference acceptance](docs/verification/m2-acceptance.md)。
