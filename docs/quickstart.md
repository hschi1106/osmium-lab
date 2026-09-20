# Release archive 快速開始

本頁是 release archive 內可獨立閱讀的最短入口；repository 使用者應改讀
[完整使用指南](user-guide.md)。Archive 需要 Linux x86_64、`osmium` binary、`examples/config.yaml`
與可寫入的資料目錄。

## 1. 檢查 binary 與設定

```sh
./osmium version
cp examples/config.yaml config.yaml
./osmium config check --config config.yaml
./osmium plan --config config.yaml
```

編輯 `config.yaml` 的 `data_root`、日期、商品、strategy 與 economics。設定 schema 見
[RunConfig 參考](config-reference.md)。

## 2. 取得並驗證資料

只有缺少 verified source 時的 `data sync` 需要網路。以 environment 提供 credential，不要寫入
YAML：

```sh
export TERALION_API_KEY='...'
./osmium data sync --config config.yaml
./osmium data verify --config config.yaml
./osmium cache prepare --config config.yaml
```

`data sync` 會 reuse 已完成的 immutable source；`cache prepare` 可由 verified source 離線重建
衍生 cache。若只拿到 archive 而沒有 source data，無法離線完成第一次 sync。

## 3. 離線回播與回測

```sh
unset TERALION_API_KEY
./osmium replay --config config.yaml
./osmium backtest --config config.yaml --output runs/example
./osmium inspect --run runs/example
```

Output directory 必須不存在。重跑時使用新目錄；不要覆寫既有 run evidence。若需要由 plan 自動
處理缺少的 source/cache，可執行：

```sh
./osmium run --config config.yaml --output runs/another-run
```

`run` 在 plan 要求下載時會使用網路與 `TERALION_API_KEY`。命令副作用與 error category 見
[CLI 參考](operations/cli.md)，資料問題見[本地資料](operations/local-data.md)。
