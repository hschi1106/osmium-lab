# 快速開始

`osmium` 需要 Rust 1.97.1 或相容的 release binary。repository 內的 fixture 是可重散布的合成測試資料，不會自動成為使用者設定的 source。

```sh
cargo build --release -p osmium-cli
target/release/osmium version
```

建立設定並依 [設定參考](config-reference.md)填入 universe、strategy 與 instrument economics：

```sh
osmium init --path config.yaml
osmium config check --config config.yaml
osmium plan --config config.yaml
```

需要取得資料時，只有 `data sync` 使用 `TERALION_API_KEY`：

```sh
osmium data sync --config config.yaml
osmium data verify --config config.yaml
osmium cache prepare --config config.yaml
```

source 與 cache 準備完成後可離線執行：

```sh
osmium replay --config config.yaml
osmium backtest --config config.yaml --output runs/example
osmium inspect --run runs/example
```

`backtest --output` 指向的目錄必須不存在。replay cache 可刪除並由 verified source 重新執行 `cache prepare` 建立，不需要重新下載資料。

只讀行情介面：

```sh
osmium display --config config.yaml
```

完整命令與副作用見 [CLI 參考](operations/cli.md)，資料狀態與復原方式見 [本地資料](operations/local-data.md)。
