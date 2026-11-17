# ADR 0007：marketdata-native 架構

## 狀態

Accepted（2026-11-17）

## 背景

舊架構以 Teralion Feed Archive 為來源，包含 online sync、staging、verified source revision、
replay cache、credential 邊界與 provider wire normalizer。團隊的 marketdata 資料表
（`/srv/md/v1`）已提供交易所原生、正規化、排序好、以 manifest 原子發布的 parquet：

- `trades`／`books`：一天 × 一個 feed，依 `(symbol, seq)` 排序；同 feed 依 `seq` 合併即為交易所
  當天送出的每一則訊息與其順序。
- `instruments`：每日商品主檔（每張股數、參考價、漲跌停、撮合循環、TAIFEX contract metadata）。
- `manifest`、`qc/*`：每日每 job 的完整性、跳號、累計量與官方比對結果。

## 決策

1. **移除 Teralion 與 source lifecycle**：刪除 `providers/teralion`、`data-sync`、sync／verify／cache
   commands、credential、fixtures、acceptance 與 release tooling、AGPL／Strategy Linking Exception。
2. **marketdata 是唯一資料源**：osmium 直接讀 parquet，不建立第二份 replay cache。資料根目錄
   `md_root` 可設定（預設 `/srv/md/v1`）。
3. **排序**：同一 feed 內依 `seq`（交易所揭示順序）合併；跨 feed 依 `exch_ts` 加固定 feed rank。
   Replay clock 定義為「至今看過的最大 `exch_ts`」，保證單調；event 保留自己的 `match_time`。
   理由（2026-01-29 TWSE_F6 實測）：同 symbol 內 `seq` 與 `exch_ts` 順序完全一致；跨 symbol
   的揭示順序與撮合時間差中位數 4 ms、p99 14 ms，13:30 收盤集合競價結果分散約 16.6 秒才
   揭示完。依撮合時間跨 symbol 合併會讓跨商品策略看見尚未揭示的結果。
4. **數值**：行情價格使用 i64（1e-4 單位），由 marketdata float64 轉換時必須是精確整數；ledger
   與 charge 仍使用 exact decimal。
5. **Economics 來自資料**：每張股數、乘數、參考價、漲跌停由 `instruments` 提供並保存 provenance；
   config 只能覆寫，與資料矛盾時拒絕。
6. **資料品質 gate**：planner 讀 `manifest` 與 `qc/*`；strict 模式拒絕狀態非 `ok` 的 feed 與 universe
   內受跳號影響的 symbol；degraded 必須明確開啟並寫入 run manifest。
7. **可重現**：run manifest 記錄每個讀取檔案的 marketdata manifest identity（`rows`、`code_hash`、
   `inputs_hash`、`git_commit`、`schema_version`），讀取前後比對以偵測更新中的替換。
8. **範圍**：TWSE／TPEx 一般股票（F6）、權證（F17）、盤中零股（F23）、TAIFEX（待 marketdata 提供），
   以及 `sinopac_ticks`、`bars_1m`、`daily` 等其他資料表，依 phase 逐步支援。
9. **Repo 架構**：crates 收斂為 `osmium-types`、`osmium-md`、`osmium-replay`、`osmium-strategy`、
   `osmium-sim`、`osmium-plan`、`osmium-runner`、`osmium-cli` 與 `example-strategy`。
10. **版本**：所有 version identity 於新架構重新起算，不相容舊 cache、run 與 config。

## 後果

- 不再需要網路、credential、staging 或 cache rebuild；資料修正由 marketdata manifest identity 追蹤。
- Repository 不 commit 交易所真實資料；tests 使用 synthetic parquet，真實資料 golden tests 只在可讀
  `/srv/md/v1` 的機器上執行。
