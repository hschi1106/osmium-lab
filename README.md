# osmium-lab

Rust market replay 與 backtesting platform，支援 Teralion market data、verified local source、derived replay cache、離線回播與策略回測。

## 環境

- Rust toolchain 與 Cargo
- `config/` 內的 YAML 設定檔
- 需要下載資料時，可在 `.env` 設定 `TERALION_API_KEY`

所有 CLI 指令都可用 `--help` 查看參數：

```sh
cargo run -p osmium-cli -- --help
cargo run -p osmium-cli -- display --help
```

## 首次準備資料

以下流程會建立 verified source 與 replay cache：

```sh
# 尚未建立 .env 時才執行；已有 .env 請略過
cp .env.example .env
# 編輯 .env，填入 TERALION_API_KEY

cargo run -p osmium-cli -- plan \
  --config config/m2-twse-2330.yaml
cargo run -p osmium-cli -- sync \
  --config config/m2-twse-2330.yaml
cargo run -p osmium-cli -- verify \
  --config config/m2-twse-2330.yaml
cargo run -p osmium-cli -- cache prepare \
  --config config/m2-twse-2330.yaml
```

## 離線回播與回測

資料與 cache 準備完成後，不需要網路或 API key；offline commands 不會讀取 `.env`：

```sh
cargo run --release -p osmium-cli -- replay \
  --config config/m2-twse-2330.yaml \
  --output target/m2-replay
cargo run --release -p osmium-cli -- backtest \
  --config config/m2-twse-2330.yaml \
  --output target/m2-backtest
cargo run -p osmium-cli -- inspect \
  --run target/m2-backtest
```

`--output` 必須指定尚不存在的新目錄。

## 日盤歷史行情 TUI

使用日盤 config `config/m4-day-multi.yaml` 啟動台指、2330 與 TPEx 6488 的互動式歷史行情回播；執行前須完成 `verify` 與 `cache prepare`：

```sh
cargo run --release -p osmium-cli -- display \
  --config config/m4-day-multi.yaml
```

操作：`←/→` 切換標的、`Space` 暫停／繼續、`+/-` 切換固定速度、`R` 重設為 `1.0x`、`Q` 離開。

VOLUME 圖以 `match_time` 的一分鐘桶加總 observed quantity，並以柱狀圖呈現。

## M1 fixture replay

已有 fixture 時可離線直接回播，不需要 source、cache 或 API key：

```sh
cargo run --release -p osmium-cli -- replay \
  --fixture <fixture-directory> \
  --output target/m1-replay
```

`--fixture` 與 `--config` 互斥。

## 一次完成完整流程

`run` 會依設定執行準備資料、驗證、建立 cache 與回測：

```sh
cargo run --release -p osmium-cli -- run \
  --config config/m2-twse-2330.yaml \
  --output target/m2-run
```

完整需求、CLI 契約與驗收資料：

- [產品需求](docs/product-requirements.md)
- [CLI 操作說明](docs/operations/cli.md)
- [Market replay TUI 設計](docs/design/market-replay-ui.md)
- [M2 reference acceptance](docs/verification/m2-acceptance.md)
