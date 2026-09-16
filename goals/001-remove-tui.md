# 001：移除 TUI

Status: pending
Depends on: none

## 目標

徹底移除 osmium-lab 的互動式行情 TUI、`osmium display` 命令，以及僅為這項功能存在的程式碼、依賴、測試與文件負擔。保留非互動式 CLI 與完整的資料準備、replay、backtest、inspect 工作流程。

這是使用者明確指定的產品能力刪除，不是評估是否值得移除。即使既有 `docs/product-requirements.md` 或其他文件要求提供 TUI，也應在本次同步更新；不得因此標為需求衝突、保留功能或再次請求授權。不要求舊版相容，不提供替代 UI、feature flag、deprecated command、alias 或相容 shim。

遵循 `goals/README.md`：Astra low 負責決策與驗收，Sol medium 負責 implementation 與測試程式，Luna max 負責驗證執行與輪詢。這是刪除任務，不需要另建架構提案或新的工作框架。

## 問題與證據

以下是撰寫時確認的定位入口，實作前仍須核對目前 working tree 與所有呼叫端；不是完整刪除清單。

- `crates/osmium-cli/src/lib.rs`：宣告 `market_replay`／`market_replay_ui`，公開 `MarketReplay`、`PlaybackSpeed`、`PlaybackStatus`、`ReplayHistory` 等型別，並提供 `display` help 與 `ParsedCommand::MarketReplay` dispatch。
- `crates/osmium-cli/src/market_replay_ui.rs`：TUI 繪製、終端控制與輸入處理入口。
- `crates/osmium-cli/src/market_replay.rs`：供 TUI 使用的播放控制、wall-clock pacing、播放歷史與圖表資料；不是核心 `replay-engine`。
- `crates/osmium-cli/Cargo.toml`：直接依賴 `ratatui` 與 `crossterm`。
- `docs/product-requirements.md` 的 OPS-01，以及 README、CLI／使用指南與架構文件：仍將 TUI 列為產品功能。

## 範圍與不做事項

### 必須移除

1. **命令入口與公開介面**：刪除 `display` 的 parsing、dispatch、help、專用 command／error 型別、exports，以及只因 TUI 存在的分支，例如互動模式對輸出選項的特殊限制。舊命令直接走一般未知命令／usage error，不留「已淘汰」專用處理。
2. **TUI 與專用支援**：刪除 UI、鍵盤事件、terminal raw mode／alternate screen、播放倍率、暫停、標的切換、wall-clock pacing、圖表與顯示歷史等專用實作。以移除上述兩個專用模組為預期結果；若發現仍被非 TUI 功能使用的部分，只保留確有現行用途的最小邏輯，不把整個 controller 搬到別處假裝完成。
3. **依賴與死碼**：移除 `ratatui`、`crossterm` 等 TUI 專用直接依賴，以及刪除功能後確實未使用的其他 dependencies、imports、helpers 與常數。透過 Cargo 更新 `Cargo.lock`，移除不再需要的傳遞依賴；不順便升級無關套件。若某項依賴仍有非 TUI 用途，保留並說明實際使用位置。
4. **測試與工具**：刪除只驗證 TUI 的測試、fixtures、範例與工具分支；保留非 TUI 功能仍使用的資料。若 TUI 測試同時是某項核心領域行為的唯一覆蓋，將必要案例留在對應模組，不能連同有效覆蓋一起消失。更新受影響的 CLI 測試、CI／release checks；沒有受影響就不改。
5. **文件與契約**：同步更新 README、產品需求、架構、CLI／使用／支援文件，以及其他實際受影響的範例、traceability 與連結，不再宣稱支援 TUI。CLI contract version 依現有機制處理；不要無故變動 config、event、cache 或 accounting 版本。歷史 changelog 可以保留舊事實，並新增移除紀錄；歷史決策文件必要時標註已被取代，不必抹除歷史。

### 必須保留／本次不做

- 保留非互動式 CLI，包括 `init`、`config check`、`plan`、`data sync/verify`、`cache prepare`、`replay`、`backtest`、`run`、`inspect`、`version` 與一般 help、human／JSON output、錯誤分類、compiled strategy 注入。
- 不因刪除 TUI 而改變 deterministic replay、事件時間與資料可見性、MarketState、source／cache 邊界、strategy、simulation 或 accounting 語義；不修改資料格式或刪除使用者行情資料。
- 不把 `std::fmt::Display`、`Path::display()`、一般 CLI 輸出，或核心行情的價格／成交量／五檔資料誤當作 TUI 刪除目標。依實際用途判斷，不依名稱全域刪除。
- 不建立 Web UI、GUI、新的 visualization/export API、可選 UI crate、通用 renderer、event bus 或額外抽象；不順便重構整個 CLI、runner 或市場狀態模型。

## 驗收

- [ ] `osmium --help` 不再列出 `display`；相關 parsing／dispatch 與 TUI 公開介面已移除。
- [ ] 有精簡的 CLI regression test 證明 `display` 被當成未知命令拒絕，走一般 usage error，不進入 config 載入、資料準備或終端初始化；其餘 CLI 契約測試仍通過。
- [ ] TUI 及其專用控制／歷史／圖表邏輯、依賴與死碼已刪除，沒有 feature flag、相容層或搬家保留的替代實作。
- [ ] 以目前 repo 既有 synthetic fixtures／smoke 流程驗證非互動式 replay、backtest 與 inspect 仍可離線完成。不新增測試框架、不下載真實行情、不為此跑全市場 benchmark；核心語義未改時，相關 event／final-state checksum 與 baseline 一致，不要求所有 artifact bytes 相同。
- [ ] 相關測試、workspace tests 與下列靜態檢查通過；未執行或失敗的必要驗證已明確記錄，不能視為完成。
- [ ] 現行文件與實作一致。搜尋 TUI／`display`／已刪 symbols 的殘留並逐項判讀；本 goal、刪除驗證、移除公告、歷史紀錄與非 TUI 的同名用途不算殘留功能，不要求關鍵字零命中。

先跑 focused validation，完成前再做 workspace 驗證：

```sh
cargo fmt --all --check
cargo test -p osmium-cli
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Astra 在刪除前確認相關 baseline 與實際 smoke 入口；Sol 修改，Luna 對穩定 working tree 執行驗證並回報命令、exit code 與必要證據，Astra review diff 後判定完成。完成此 goal 就停止；不 commit／push，除非使用者另外要求。

## 執行紀錄

- Baseline revision／working tree：待執行時記錄。
- 已完成：尚未實作；本檔僅定義目標。
- 驗證命令／結果：尚未執行。
- 阻塞／剩餘風險：待核對共享用途與既有測試。
- 下一步：確認目前 TUI 呼叫端與離線 smoke baseline，交由 Sol medium 移除。
