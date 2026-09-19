# ADR 0006：外部 compiled strategy 與通用 cash charge

## 狀態

Accepted

## 決策

`osmium-cli` 提供 `StrategyRegistryProvider` 與 `run_with_registry_provider`。外部 Rust binary 在 compile time 註冊 strategy factories；argument parsing、commands、runner 與錯誤分類仍由 lab 實作。不採 dynamic library、runtime plugin 或 ABI boundary。

非 fill 現金成本由 capability-gated `CashChargeRequest` 表達。Request 只包含非負 exact decimal `amount`、非空 `category` 與 `reference`；時間由 runner 使用 callback 的 `match_time` 指派。Stable identity 由 callback origin 和 output sequence 雜湊產生，同一 callback 的 requests 先全部驗證，再於新 orders 的資金判定前原子寫入 `MultiLedger`。

Lab 不解讀 category，也不實作任何特定融資、借貸或策略政策。這些規則及參數由外部 strategy 擁有。

每筆 fill 同步保存 `fee_delta` 與 `tax_delta`。後續成交觸發當沖稅重算時，負的 `tax_delta` 是返還先前多計稅額的 adjustment。Scheduled artifacts 額外發布 `fill-costs.json`、`cash-charges.json` 及 checksum；ledger、performance 與 reconciliation 均包含 cash charges。

## 理由

此邊界讓私人策略只依賴固定 lab revision，不需複製平台 source，同時維持 deterministic replay、明確 capability、exact accounting 與可追溯 artifacts。

## 相容性

Built-in registry 與預設 `osmium` binary 行為不變。新增 cash charge 使 strategy output 升為 version 3、accounting 升為 version 8、run manifest 升為 version 3；舊版本不會被靜默當成新格式解讀。
