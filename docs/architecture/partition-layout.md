# Source／Cache 分區 Layout

M3 的 source 以 `SourcePartitionKey` 分區，每個 instrument／trading date／session
選擇都有自己的 `current.yaml` 與 immutable revisions：

```text
source/teralion/<market>/<trading-date>/<symbol>/
  partition.yaml
  current.yaml
  revisions/<source-revision>/
  staging/<attempt>/
cache/replay/teralion/<market>/<trading-date>/<symbol>/<cache-identity>/
```

`partition.yaml` 保存 source、instrument、交易日、session kinds、SessionPlan
identity 與 partition identity；repository 在讀取 current pointer 前會檢查 metadata
是否仍與 requested `SourcePartitionKey` 相符。cache catalog 只接受同一 partition
identity 與 source revision 的 descriptor，避免把其他 instrument 或舊 revision 的
cache 綁進 replay。

既有 M2 的 `source/current.yaml` 與 `derived/cache` layout 仍由原 API 保留；M3
sync、verify、partition cache builder 與 offline replay 使用上述 keyed layout。
