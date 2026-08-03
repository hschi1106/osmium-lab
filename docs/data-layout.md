# Local data layout

`data_root` 由使用者設定，source revision 是 immutable、可跨 run 重用的 verified
material；replay cache 是 source-bound derived artifact，可刪除並由
`osmium cache prepare` 重建；run output 是單次執行的 immutable evidence。

```text
<data_root>/
  catalog/
  source/teralion/<market>/<date>/<symbol>/
    partition.yaml
    current.yaml
    revisions/<source-revision>/
  cache/replay/<market>/<date>/<symbol>/<cache-identity>/
  runs/<user-selected-run>/
```

release archive 不包含 `data_root`、raw dump、target、`.env` 或 acceptance payload。
repository fixture 只供 maintainer acceptance tooling 使用。詳細 state、checksum、
atomic publish 與 recovery 規則見 [local data contract](operations/local-data.md)。
