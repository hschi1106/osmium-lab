# 003：將 Teralion 收斂為單一 provider，讓 core source pipeline 解耦

Status: done
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

- [x] data-sync 不 import Teralion query/cursor/transport/normalizer。
- [x] run-planner SourceId 不列舉 Teralion。
- [x] osmium-config 不硬編 Teralion mapping。
- [x] 三個 market-named normalizer crates 已刪。
- [x] Teralion acquisition+normalization 集中單一 provider crate。
- [x] generic cache publication 接 neutral events。
- [x] arbitrary non-Teralion SourceId 可建 partition/identity。
- [x] unsupported provider 只在 composition root 失敗。
- [x] offline replay/backtest 不需 Teralion HTTP/credential。
- [x] provider fixture regression 通過。
- [x] workspace crate count 淨下降，無 provider framework。
- [x] fmt/test/clippy 通過。
- [x] cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：`c323f51` / clean。
- Rust LOC before：dirty-working-tree file-list cloc：108 files / 3,569 blank / 264 comment / 41,649 code。
- Workspace crates before：14。
- Provider boundary changes：新增 `crates/providers/teralion`，集中 Teralion credential、query、cursor、HTTP transport、wire parsing、TWSE／TPEx／TAIFEX normalizer、mapping identity 與 source orchestration；`data-sync` 改成 provider-neutral staging、verification 與 neutral event cache publication；`osmium-config` 由 composition root 傳入 mapping resolver；`SourceId` 改為 storage-safe value type。
- Deleted crates/paths：刪除 `crates/normalizer/twse`、`crates/normalizer/tpex`、`crates/normalizer/taifex`，並將原 provider acquisition／normalizer source、tests、benchmark 移至 `crates/providers/teralion`。
- Validation：`cargo fmt --all --check`、`cargo check --workspace --all-targets`、`cargo test --workspace`（318 passed / 52 suites）、`cargo clippy --workspace --all-targets --all-features -- -D warnings`；provider fixture regression、synthetic generator／compact verifier、acceptance Python tests（8 passed）、standalone fixture builder check 均通過。未設定 `TERALION_API_KEY` 的 offline flow 完成 `data verify`、cache reuse、replay、backtest、inspect；replay event checksum `4668d8745908ed6475347c8e6d435fbcf865939a000498adee809f1a4b212860`、final-state checksum `a53c1e969d3b74398384527bf9c73e3ac8d0b30aa42a3c057aa720314c49a02`。
- Rust LOC after / delta：dirty-working-tree file-list cloc：111 files / 3,596 blank / 267 comment / 41,769 code；相較 before 為 `+3 files / +27 blank / +3 comment / +120 code`。
- Workspace crates after：12，淨減 2（刪 3 normalizer crates、新增 1 provider crate）。
- Breaking changes：`SourceId` enum 改為 validated string value，`SOURCE_PARTITION_KEY_VERSION` 升至 2；config planning 改由 composition root 傳 mapping resolver；data-sync 不再提供 provider normalization dispatch；crate path、source/cache identity 與 provider partition layout 可變，舊 derived cache 需由 verified source rebuild；未保留 legacy wrapper 或 dual pipeline。
- 剩餘風險：目前只實作 Teralion provider；其他 SourceId 可被 core 建立 partition identity，但在 composition root 會回報 unsupported source，尚未有第二 provider 的實際 wire certification。未引入 provider registry、dynamic loading、DI 或通用 framework。
- 下一步：004-unified-market-state.md
