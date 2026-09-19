# 003：將 Teralion 收斂為單一 provider，讓 core source pipeline 解耦

Status: pending
Depends on: 002-provider-neutral-validation.md

## 目標

讓核心只理解 stable source identity、verified source partition、source revision lineage、normalizer mapping identity、neutral `DomainEvent`、replay cache。

核心不理解 Teralion endpoint、API key、cursor、query、`received_at`、JSON wire schema、format names 或 Teralion market normalizer implementation。

完成後，新增另一 provider 不需要修改 market-state/replay/strategy/simulation/accounting。

## 現況候選

實作前重新核對：

```text
run-planner::SourceId -> provider-specific enum
data-sync -> Teralion query/credential/cursor/transport
data-sync -> provider-specific normalization dispatch
normalizer/twse,tpex,taifex -> 實際解 Teralion JSON
```

已有 generic normalized-event cache publication 時優先重用。

## 目標 workspace

```text
crates/
├── data-sync/
├── providers/
│   └── teralion/
├── market-types/
├── market-state/
├── run-planner/
├── osmium-config/
├── osmium-cli/
└── ...

刪：
crates/normalizer/twse
crates/normalizer/tpex
crates/normalizer/taifex
```

增加 1 provider crate、刪 3 normalizer crates；不要再拆更多 provider crates。

## SourceId

改成 stable provider-neutral value type，概念：

```text
SourceId("teralion")
SourceId("synthetic-source")
```

要求 non-empty、storage-safe、canonical hash 實際 bytes；breaking identity 升版本，不留 alias/old reader。

Config 可保留 `source: teralion`；parser 只驗字串是否合法，provider availability 在 composition root 驗。

## data-sync 保留

- partition layout
- staging/immutable revision
- checksums
- source verification
- cache descriptor/publication/read/catalog
- NormalizerMappingIdentity
- generic errors

data-sync 不再 import Teralion transport/query/cursor/normalizer。

### Normalization ownership

```text
provider:
verified raw source
-> Vec<DomainEvent> + mapping identity

data-sync:
verified lineage + neutral events
-> replay cache
```

刪 generic cache layer 內 Twse/Tpex/Taifex normalizer enum dispatch。

## teralion-provider

收走 credential、endpoint/query、cursor、HTTP、provider sync logic、Teralion JSON parsing、TWSE/TPEx/TAIFEX wire mapping、mapping identity、provider conformance tests。

不要一概念一 crate。

## Composition root

目前只有：

```text
source == "teralion" -> TeralionProvider
else -> unsupported source
```

只有當 sync/mapping/normalize dispatch 已明顯重複才允許極小 interface。

禁止 registry/dynamic loading/DI/capability graph。

## Planning

osmium-config/run-planner 不自行知道 Teralion mapping。expected mapping identity 由上層已選 provider 的 resolver 傳入。

ExecutionPlan 不持有 provider object。

## Breaking change

允許 SourceId、partition version、effective config identity、source/cache path、mapping identity、normalizer crate path 改變。

不保留 legacy normalizer wrapper / dual pipeline。raw source 不自動刪，derived cache 可 rebuild。

## 驗收

- [ ] data-sync 不 import Teralion query/cursor/transport/normalizer。
- [ ] run-planner SourceId 不列舉 Teralion。
- [ ] osmium-config 不硬編 Teralion mapping。
- [ ] 三個 market-named normalizer crates 已刪。
- [ ] Teralion acquisition+normalization 集中單一 provider crate。
- [ ] generic cache publication 接 neutral events。
- [ ] arbitrary non-Teralion SourceId 可建 partition/identity。
- [ ] unsupported provider 只在 composition root 失敗。
- [ ] offline replay/backtest 不需 Teralion HTTP/credential。
- [ ] provider fixture regression 通過。
- [ ] workspace crate count 淨下降，無 provider framework。
- [ ] fmt/test/clippy 通過。
- [ ] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- Workspace crates before：
- Provider boundary changes：
- Deleted crates/paths：
- Validation：
- Rust LOC after / delta：
- Workspace crates after：
- Breaking changes：
- 剩餘風險：
- 下一步：004-unified-market-state.md
