# 002：建立 provider-neutral 驗證邊界與 repo-contained fixtures

Status: pending
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

- [ ] provider wire fixtures 位於 `fixtures/providers/teralion/`。
- [ ] fixtures 全為 repo-owned synthetic。
- [ ] core tests 不依賴 Teralion fixture 才能驗核心語義。
- [ ] expected semantics 是 contract assertions。
- [ ] `/data`/network 非 test dependency。
- [ ] benchmark seed 可 deterministic 擴增。
- [ ] generator/manifest/docs path 一致。
- [ ] workspace fmt/test/clippy 通過。
- [ ] cloc 已記錄。
- [ ] 無新 fixture platform。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- Fixture tree before/after：
- `/data` research evidence：
- Validation：
- Benchmark seed：
- Rust LOC after / delta：
- 剩餘風險：
- 下一步：003-decouple-teralion-provider.md
