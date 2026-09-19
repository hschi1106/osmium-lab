# Goals：精簡 osmium-lab 架構、完成 provider 解耦與回測能力閉環

本目錄是一組可由 **Luna max long run** 依序執行的破壞性重構目標。

最終目的不是單純壓低 LOC，而是讓 `osmium-lab`：

1. 與任何特定歷史行情供應商完全解耦；Teralion 只是目前內建 provider。
2. 保留並驗證產品需求中聲稱支援的全部回測能力：資料準備、deterministic replay、strategy、一般與 scheduled execution、各支援商品的帳務、run artifacts 與 inspect。
3. 移除已知無用能力、舊相容路徑、重複 orchestration、重複 domain 判斷與無必要 abstraction。
4. 使主要資料流可以由人順著讀完，crate/module ownership 清楚。
5. 使用本機 `~/cloc/cloc` 量化 Rust code footprint；LOC 是觀察指標，不是犧牲正確性的 KPI。

本 goals 明確授權 breaking refactor；不要求舊 API、config、cache、artifact 或 crate path 相容。

---

## 1. 執行模型：Luna max 單一 long-run executor

這套 goals 是為 **Luna / max** 作為主執行者設計，不再要求 Astra / Sol 分工，也不要因舊 goal 內容提到其他模型而切換模型。

Luna max 負責：

- 讀取 repo、確認現況與依賴。
- 選擇目標範圍內最簡單有效方案。
- 修改 production code、tests、fixtures、docs。
- 執行 focused / workspace tests、benchmark、cloc。
- review 自己的 diff 與驗證結果。
- 更新當前 goal 的執行紀錄與 status。

不要建立 subagent orchestration framework。若環境本身提供 subagent，可不用；本計畫不依賴它。

### 長跑規則

- 一次只執行一個 goal，依編號前進。
- 前一 goal 未 `done`，不得跳到依賴它的下一 goal。
- 每一 goal 開始時重新讀取目前 working tree；不能假設前一份摘要仍是最新事實。
- 不自行 commit、push、rebase、reset、clean 或刪除使用者原始行情。
- 不等待人工確認 breaking changes；本目錄已授權。
- 必要驗收真的無法完成時，將 goal 標成 `blocked` 並停止依賴鏈；不得假裝通過。
- 發現 scope 外問題，只在執行紀錄留下短候選；不要順手展開。
- 不為 long run 建立 task DB、dashboard、scheduler、agent registry 或 recovery framework。

---

## 2. 開始每個 goal 前必讀

1. 根目錄 `AGENTS.md`
2. `docs/product-requirements.md`
3. 本 `goals/README.md`
4. 當前 goal

其他文件與 code 依實際資料流再讀，不要求每個 goal 全 repo 重讀。

`docs/product-requirements.md` 是產品範圍基準；goal 可以改變其架構描述與已明確授權刪除的功能，但不得未經 goal 指示自行新增或刪除產品能力。

### 「所有回測能力」的定義

本計畫中的「所有回測能力」指 **執行當時修訂後的產品需求聲稱支援的能力全部有 production path 與驗證**。

它不自動代表新增：

- IOC / FOK 或所有交易所 order type
- 盤中零股、盤後零股、盤後定價、鉅額交易
- 即時交易
- 完整 exchange matching engine
- queue position / hidden liquidity
- 交易所未由資料證明的內部狀態

若產品需求目前把某能力列為 out-of-scope，就不要在 cleanup 計畫中偷偷加回來。

---

## 3. 精簡原則

優先順序：

1. 刪除沒有產品用途的功能與 dead code。
2. 刪除舊版 compatibility path。
3. 把 provider-specific code 收回 provider 邊界。
4. 合併語義相同的重複判斷與重複 orchestration。
5. 移除純 forwarding wrapper、不必要 DTO 轉換與 one-call-site abstraction。
6. 只有在能刪掉實際 coupling / duplication 時才新增 abstraction。

禁止為「未來可能」新增：

- generic framework
- plugin registry
- dependency injection container
- event bus
- rule engine
- DSL
- capability graph
- provider registration macro
- dynamic loading
- state-machine framework

不要把語義不同、只是長得像的 code 硬合併。

### 必須保留的核心正確性

- deterministic replay
- `match_time` 與資料可見性 / no-look-ahead
- firm vs indicative market data isolation
- verified source / derived cache boundary
- provider wire / DomainEvent boundary
- strategy read-only market state
- order origin / subsequent-event causality
- scheduled control time 與 replay event stream 分離
- exact fee / tax / cash / position / P&L accounting
- reconciliation
- immutable run artifacts 與 inspect lineage

---

## 4. Breaking-change 政策

允許直接修改：

- public Rust API
- CLI
- config schema
- cache / source identity
- canonical encoding / version
- module / crate boundary
- local derived-data layout
- artifact version

原則：

- 只保留一套新實作。
- 不留 deprecated alias、wrapper crate、fallback parser、dual reader、legacy encoder。
- 舊 derived cache 可拒絕並重建。
- 不自動刪除或覆寫 user-owned raw source。
- 如果舊格式需要被拒絕，保留**最小版本檢查**即可；不要保留舊版讀取能力。

---

## 5. LOC 計量：強制使用 `~/cloc/cloc`

### 5.1 Repo baseline

Goal-000 必須執行：

```sh
~/cloc/cloc --vcs=git --include-lang=Rust .
```

保存 Rust `code` 欄位。

### 5.2 Dirty working tree 的準確計數

long run 中新增檔案可能尚未被 Git track；因此每個 goal 結束時，另外以目前 working tree 的 Rust 檔案清單計數：

```sh
git ls-files -co --exclude-standard '*.rs' \
  | while IFS= read -r f; do
      [ -f "$f" ] && printf '%s\n' "$f"
    done \
  | sort -u > /tmp/osmium-rust-files.txt

~/cloc/cloc \
  --list-file=/tmp/osmium-rust-files.txt \
  --include-lang=Rust
```

記錄：

```text
Rust LOC before
Rust LOC after
delta
```

必要時對主要受影響 crate 再跑 `~/cloc/cloc --include-lang=Rust <path>`，但 repo total 仍是主要比較指標。

### LOC 解讀

- 不設定硬性「每個 goal 都必須下降」。
- 新 contract / correctness tests 可能暫時增加 LOC。
- 不用壓縮格式、刪必要測試、刪註解或合併語義不同 code 來灌水。
- Goal-012 最後比較 Goal-000 baseline 與 final Rust LOC，並解釋增減來源。

---

## 6. 每個 goal 的固定工作迴圈

1. `git rev-parse HEAD`、`git status --short`。
2. 讀當前 code，不把 goal 中的舊 symbol 當成仍存在的事實。
3. 執行本 goal 的 before-cloc。
4. 建立必要 baseline tests / checksums / behavior evidence。
5. 只修改當前 scope。
6. focused tests。
7. `cargo fmt --all --check`
8. 必要時 `cargo clippy --workspace --all-targets -- -D warnings`
9. goal 完成前 `cargo test --workspace`
10. after-cloc。
11. review diff：確認舊路徑真的刪掉，而不是新舊雙軌。
12. 更新 goal 執行紀錄；所有 acceptance 成立才改 `Status: done`。

長程序使用正常 blocking wait 或有 timeout 的腳本；不要高頻輪詢。

---

## 7. 驗證政策

Rust goal 的最低完成 gate：

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

若 repo 當時既有 CI / acceptance 有更適合的 focused command，先跑 focused 再跑 workspace。

涉及 replay / market-state / execution / accounting / cache / provider mapping 時，不能只看 compile pass；必須跑相應 domain cases。

若 schema / canonical encoding / ordering / identity 刻意改變：

- 不要求跨版本 checksum 相同。
- 必須解釋差異。
- 新版本重跑必須 deterministic。
- 不能只更新 golden 值就宣稱正確。

---

## 8. Goal 順序

```text
000 baseline-and-inventory
  ↓
001 remove-tui
  ↓
002 provider-neutral-validation
  ↓
003 decouple-teralion-provider
  ↓
004 unified-market-state
  ↓
005 close-market-background-gap
  ↓
006 backtest-capability-closure
  ↓
007 consolidate-execution-paths
  ↓
008 simplify-accounting-economics
  ↓
009 simplify-config-planner-runner
  ↓
010 remove-legacy-compatibility
  ↓
011 final-redundancy-cleanup
  ↓
012 final-acceptance
```

Goal-006 是後續精簡的 functional firewall；Goal-011 才做全 repo 最後 cleanup。

---

## 9. Status

每份 goal：

- `pending`
- `in_progress`
- `blocked`
- `done`

執行紀錄只保留恢復需要的資訊：

- baseline revision / working tree
- before / after Rust LOC
- 決定與改動
- 驗證 command / exit result
- breaking changes
- 剩餘風險
- 下一步

不要保存逐輪思考日誌。

---

## 10. 最終成功條件

Goal-012 完成時：

- TUI 與專用負擔消失。
- 核心 market semantics provider-neutral。
- Teralion acquisition + wire mapping 集中在 provider boundary。
- 核心 source/cache/replay/strategy/sim/accounting 不依賴 Teralion schema。
- 產品需求內聲稱支援的回測能力全部有 production path + tests。
- 普通 / scheduled execution 不保留可證明的重複 market interpretation / order lifecycle。
- economics / accounting config 到 runtime 沒有不必要重複模型。
- config → plan → run data flow 可順著理解。
- legacy compatibility production path 清除。
- final full-repo cleanup 完成。
- workspace tests / clippy / fmt / smoke / capability gates 通過。
- final Rust LOC、crate count、主要依賴與 benchmark 結果和 baseline 一起回報。
