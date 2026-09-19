# 002：建立 provider-neutral 驗證邊界與 repo-contained fixtures

Status: done
Depends on: 001-remove-tui.md

## 目標

建立後續 provider/market/execution 重構的安全網：

- core correctness 不依賴 Teralion wire fixture。
- provider-specific fixture 位於 provider namespace。
- 真實 `/data` 可作研究證據，但 tests 不依賴它。
- performance workload 可由小 synthetic seed deterministic 放大。

本 goal 不重構 production provider architecture；Goal-003 才做。

## Fixture 結構

```text
fixtures/
├── README.md
├── acceptance/
├── providers/
│   └── teralion/
│       ├── README.md
│       ├── twse/
│       ├── tpex/
│       └── taifex/
└── smoke/
```

把一般 adapter coverage 的 `fixtures/teralion/` 收斂到 `fixtures/providers/teralion/`。

`fixtures/smoke/` 是 end-to-end bundle；不要只為美觀重排。

### Provider-neutral tests

DomainEvent 已是 Rust domain type；core tests 直接建 Rust values/table-driven cases。

不要新增 `fixtures/domain`、`market`、`neutral`、`expected`，除非 production interface 本身真的讀檔。

Expected semantics 放 assertions，不錄現有 implementation output 當答案。

## `/data` 與網路

允許唯讀本機 `/data` 與官方市場文件，找 edge-case shape、理解前後文、幫助 synthetic fixture。

禁止：

- real raw payload copy/anonymize 後 commit
- test 依賴 `/data`
- fixture service / bundle DB
- 用網路內容補造缺失 tick

## Synthetic Teralion fixture

只負責：

```text
synthetic Teralion wire
-> current Teralion normalizer
-> existing neutral DomainEvent semantics
```

此 goal 不預先實作 Goal-004 新 taxonomy。

## Benchmark seed

沿用既有 benchmark harness；以小 seed deterministic repeat/instrument offset/time offset/interleave 產生固定 event count/checksum。大 workload 不進 Git。

## 不做

- 不新增 provider trait/registry。
- 不搬 data-sync transport。
- 不合併 normalizer crates。
- 不改 SourceId。
- 不新增第二 provider。
- 不建 fixture framework。

## 驗收

- [x] provider wire fixtures 位於 `fixtures/providers/teralion/`。
- [x] fixtures 全為 repo-owned synthetic。
- [x] core tests 不依賴 Teralion fixture 才能驗核心語義。
- [x] expected semantics 是 contract assertions。
- [x] `/data`/network 非 test dependency。
- [x] benchmark seed 可 deterministic 擴增。
- [x] generator/manifest/docs path 一致。
- [x] workspace fmt/test/clippy 通過。
- [x] cloc 已記錄。
- [x] 無新 fixture platform。

## 執行紀錄

- Baseline revision / working tree：`3ac3c21`（Goal 001 完成後 clean）。
- Rust LOC before：108 files / 3,569 blank / 264 comment / 41,614 code。
- Fixture tree before/after：一般 adapter coverage 從
  `fixtures/teralion/{twse,tpex,taifex}` 搬至
  `fixtures/providers/teralion/{twse,tpex,taifex}`；`fixtures/smoke/` 保持為獨立
  end-to-end bundle；acceptance manifest、normalizer tests、benchmark include path、
  interface docs 與 generator/verification scripts 已同步。
- `/data` research evidence：本 goal 沒有讀取或提交 `/data` 內容；fixture generator
  只建立 repository-owned synthetic wire payload。測試/fixture 搜尋沒有 `/data`、HTTP
  client 或 API key 依賴；production `data-sync` transport 的 reqwest/Teralion URL
  維持不變。
- Validation：`cargo fmt --all --check`、`cargo test --workspace`
  （317 passed / 56 suites）、`cargo clippy --workspace --all-targets --all-features
  -- -D warnings`、`tools/acceptance/verify_compact_fixtures.sh` 通過；neutral
  `replay-engine` contract test 通過，且移除其 `twse-normalizer` dev-dependency。
  生成器重新執行後 manifest 與 fixture checksums 維持一致，沒有舊
  `fixtures/teralion/` path 殘留。
- Benchmark seed：沿用既有 Rust-native harness。`canonical_hot_path` 以固定
  `fixture_event` 加 match-time/sequence offset 產生 50,000 events；
  `multi_stream_merge` 以固定 payload、instrument offset、global sequence/time
  interleave 產生 1/8/32 streams、49,152 events，並跨 rounds assertion event/final
  state checksum；`full_multi_backtest` 沿用 8 instruments 的 deterministic seed。
- Rust LOC after / delta：108 files / 3,569 blank / 264 comment / 41,649 code，
  code `+35`。
- 剩餘風險：目前仍只有 Teralion provider wire coverage；provider boundary 與
  taxonomy 解耦留待 Goal 003/004，未在本 goal 預先改動 production architecture。
- 下一步：003-decouple-teralion-provider.md
