# osmium-lab

Rust market replay 與 backtesting platform，支援 Teralion market data、verified local source、derived replay cache、離線回播與策略回測。

## 環境

- Rust toolchain 與 Cargo
- `config_version: 2` 的 YAML `RunConfig`
- 需要下載資料時，可在 `.env` 設定 `TERALION_API_KEY`

所有 CLI 指令都可用 `--help` 查看參數：

```sh
cargo run -p osmium-cli -- --help
cargo run -p osmium-cli -- version
cargo run -p osmium-cli -- display --help
```

## 首次準備資料

以下流程會建立 verified source 與 replay cache：

```sh
# 尚未建立 .env 時才執行；已有 .env 請略過
cp .env.example .env
# 編輯 .env，填入 TERALION_API_KEY

cargo run -p osmium-cli -- config check \
  --config examples/config.yaml
cargo run -p osmium-cli -- plan \
  --config examples/config.yaml
cargo run -p osmium-cli -- data sync \
  --config examples/config.yaml
cargo run -p osmium-cli -- data verify \
  --config examples/config.yaml
cargo run -p osmium-cli -- cache prepare \
  --config examples/config.yaml
```

## 離線回播與回測

資料與 cache 準備完成後，不需要網路或 API key；offline commands 不會讀取 `.env`：

```sh
cargo run --release -p osmium-cli -- replay \
  --config examples/config.yaml
cargo run --release -p osmium-cli -- backtest \
  --config examples/config.yaml \
  --output target/example-backtest
cargo run -p osmium-cli -- inspect \
  --run target/example-backtest
```

`--output` 必須指定尚不存在的新目錄。

## 日盤歷史行情 TUI

使用已準備 source/cache 的 v2 config 啟動互動式歷史行情回播；執行前須完成
`data verify` 與 `cache prepare`：

```sh
cargo run --release -p osmium-cli -- display \
  --config examples/config.yaml
```

操作：`←/→` 切換標的、`Space` 暫停／繼續、`+/-` 切換固定速度、`R` 重設為 `1.0x`、`Q` 離開。

VOLUME 圖以 `match_time` 的一分鐘桶加總 observed quantity，並以柱狀圖呈現。

## Fixture tooling

repository 只保留小型 synthetic smoke fixture，以及各市場／狀態的 compact
representative slices。real acceptance payload 不是完整交易日資料，且依
`fixtures/acceptance/manifest.yaml` 的 private authorization policy 管理。

```sh
tools/acceptance/verify_compact_fixtures.sh
```

fixture builder 與 bundle fetch／verify 都是 maintainer tooling；release `osmium`
不接受 `--fixture`，也不會在 replay 中隱式下載資料。

## 一次完成完整流程

`run` 會依設定執行準備資料、驗證、建立 cache 與回測：

```sh
cargo run --release -p osmium-cli -- run \
  --config examples/config.yaml \
  --output target/example-run
```

## 建立 internal binary archive

首個 release 以 private/internal binary archive 交付；archive 不包含 raw data、target、
`.env` 或 acceptance payload：

```sh
tools/release/package.sh \
  --output target/osmium-internal.tar.gz
```

腳本同時產生 `<archive>.sha256`，並將 neutral example、文件、fixture manifest、
dependency inventory、CycloneDX SBOM 與 third-party license inventory 放入 archive。
在 clean machine 驗證安裝與 deterministic bytes：

```sh
tools/release/smoke_clean_machine.sh \
  --archive target/osmium-internal.tar.gz \
  --checksum target/osmium-internal.tar.gz.sha256
tools/release/verify_reproducibility.sh \
  --output target/release-repro-gate
```

Private acceptance bundle 不隨 binary archive 發布。maintainer 可使用：

```sh
tools/acceptance/fetch_fixture_bundle.sh \
  --source https://<internal-artifact-store>/osmium/acceptance.tar.gz \
  --output target/acceptance-bundle
```

HTTPS flow 需要 `OSMIUM_FIXTURE_BUNDLE_TOKEN`；實際 URL 與 SSO policy 由 internal
deployment 提供。小型 synthetic smoke fixture 位於 `fixtures/smoke/`，可在無 credential
環境驗證。

完整需求、CLI 契約與驗收資料：

- [產品需求](docs/product-requirements.md)
- [CLI 操作說明](docs/operations/cli.md)
- [Quickstart](docs/quickstart.md)
- [User guide](docs/user-guide.md)
- [RunConfig reference](docs/config-reference.md)
- [Local data layout](docs/data-layout.md)
- [Market replay TUI 設計](docs/design/market-replay-ui.md)
- [Release validation](docs/release/VALIDATION.md)
- [Fixture policy](fixtures/README.md)
- [Release cleanup](docs/release/release-cleanup.md)
- [內部支援政策](docs/release/SUPPORT.md)
