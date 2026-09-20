# Agent Instructions

`osmium-lab` 是 Rust market replay 與 backtesting platform。Domain truth 以 canonical docs 為準；不確定時從 [`docs/README.md`](docs/README.md) 導航。Code 與 tests 是 executable truth。若需求與 canonical contract 衝突，修改前先指出。

## Skills

- 修改、review、debug、test 或設計 implementation／architecture：使用 `osmium-engineering`。
- 操作既有 build 的 config、data、cache、replay、backtest 或 artifacts：使用 `osmium-operator`。
- 操作問題若需要 Rust、architecture 或 domain semantic change，轉交 `osmium-engineering`。

## Repository rules

- Project documentation 使用繁體中文；code identifiers、API fields、commands 與既有 technical terms 可保留 English。
- 修改限於擁有該行為的 crate／module，採 small reviewable change；避免 broad rewrite、speculative abstraction、無關 formatting 或 file move。
- 保留使用者既有變更；未經明確要求不得 revert、overwrite、clean、rebase 或 force-push。
- 先跑與範圍相符的 focused validation；只有實際成功的 command 才能宣稱已驗證。無法執行時說明原因與最接近的 manual check。
- Commit 維持單一 logical change。回報 changed files、behavior／docs impact、scope 理由與 verification。
