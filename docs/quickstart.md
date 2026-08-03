# osmium quickstart

`osmium` 是 private/internal release tool。先準備一份 v2 config，確認
`data_root` 指向使用者擁有的資料目錄；repository fixture 不會自動成為 live source。

```sh
osmium version
osmium config check --config examples/config.yaml
osmium plan --config examples/config.yaml
```

第一次需要下載資料時，只有 `data sync` 會使用 `TERALION_API_KEY`：

```sh
osmium data sync --config examples/config.yaml
osmium data verify --config examples/config.yaml
osmium cache prepare --config examples/config.yaml
```

準備完成後，執行與檢查都不需要網路或 credential：

```sh
osmium replay --config examples/config.yaml
osmium backtest --config examples/config.yaml --output runs/example
osmium inspect --run runs/example
```

`backtest` 的 output directory 必須事先不存在。刪除 replay cache 後，可重新執行
`osmium cache prepare`，不需要重新下載 immutable source。完整資料生命週期見
[local data contract](operations/local-data.md)。

`osmium display --config examples/config.yaml` 是只讀的歷史行情 TUI；它需要已驗證的
source/cache，不寫入 run artifacts。鍵盤操作與顯示邊界見
[market replay TUI design](design/market-replay-ui.md)。
