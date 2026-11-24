# 驗證

## Rust quality gates

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

只有 command 實際以 exit 0 完成才可宣稱 PASS。Focused crate tests 可先跑，但不能取代 final
workspace gate。

Repository 不 commit 交易所真實資料；需要真實 marketdata 的驗證只在可讀取 `/srv/md/v1` 的機器上
執行，並在回報中寫明實際使用的日期與 feed。

## 合成資料

`osmium-runner`、`osmium-examples` 與 `osmium-cli` 的測試不需要真實資料：`osmium-fixture` 在暫存
目錄寫出一棵小的 marketdata 樹（日曆、manifest、`instruments`、`books`、`trades`、`daily`、
`daily_ext`、`corp_actions`、`events`），格式與真實資料相同，由 `osmium-md` 以同樣的檢查讀取。
`osmium_fixture::sample` 是兩個交易日、一檔商品的樣本市場，每一則訊息都寫在程式裡，測試的預期
值可以手算。

| 測試 | 檢查的內容 |
| --- | --- |
| `crates/osmium-runner/tests/run.rs` | 一個完整的 run：策略收到的每一次呼叫（golden log）、每個輸出檔的內容、兩次 run 位元組相同、不覆寫既有結果、缺少 feed 的日子、沒有交易日的範圍 |
| `crates/osmium-runner/tests/config.rs` | 設定檔的預設值、每個 section、錯誤訊息指出 key |
| `crates/osmium-examples/tests/examples.rs` | 三個範例策略在樣本市場的委託事件與權益 |
| `crates/osmium-cli/tests/cli.rs` | 每個指令的輸出與 exit status |

修改撮合、帳務或 runner 而讓這些 golden 改變時，先確認新的數字是對的（手算），再更新 golden 與
[對應的版本](../reference/versions.md)。

## 真實資料

真實資料測試以 `#[ignore]` 標記，不在 workspace gate 內執行：

```sh
OSMIUM_MD_ROOT=/srv/md/v1 cargo test --locked -p osmium-md --release -- --ignored --nocapture
OSMIUM_MD_ROOT=/srv/md/v1 cargo test --locked -p osmium-replay --release -- --ignored --nocapture
OSMIUM_MD_ROOT=/srv/md/v1 cargo test --locked -p osmium-matching --release --test marketdata -- --ignored --nocapture
OSMIUM_MD_ROOT=/srv/md/v1 cargo test --locked -p osmium-ledger --release --test marketdata -- --ignored --nocapture
```

`OSMIUM_MD_ROOT` 未設定時使用 `/srv/md/v1`。這些測試只讀取資料，不寫入 marketdata 目錄。

完整的 run 以 CLI 對真實資料執行，輸出放在 repository 之外：

```sh
cargo run --release -p osmium-cli -- check run.toml
cargo run --release -p osmium-cli -- run run.toml --out /path/outside/the/repository/run-1
```

同一個設定跑兩次，兩份 `run.json` 的 `files`（每個輸出檔的 `blake3`）必須相同。只改變速度或記憶體
的修改，修改前後的 `files` 也必須相同。

| Crate | 檢查的內容 |
| --- | --- |
| `osmium-md` | 每個 feed 的訊息種類、列數對帳、`observed_time`、指定商品與整個 feed 讀到相同的訊息；參考表（`tests/reference.rs`）：日曆、2022 年以來全部的公司行動與事件、每年數個交易日的盤後價格與信用限額都能精確轉換 |
| `osmium-replay` | 四個 feed 合併後的全序與時間線；每本 book 恰有一列開盤與一列收盤撮合列、撮合方式的推論與 `match_interval_s` 相同、每一則訊息的 `phase` 與 [`osmium-book` 的模式](../reference/matching.md)一致 |
| `osmium-matching` | 見下方 |
| `osmium-ledger` | 見下方 |

`osmium-replay` 的測試同時載入數個 feed 的一整天，約需 1.2 GB 記憶體。

### 撮合

`osmium-matching` 的真實資料測試（`crates/osmium-matching/tests/marketdata.rs`）檢查
[撮合規格](../reference/matching.md)第 11 節的性質。每個測試會印出統計（`--nocapture`）；修改撮合
規則後要比較這些數字，不只看是否通過。

| 測試 | 做法 | 日期與 feed |
| --- | --- | --- |
| `real_continuous_trades_stay_inside_the_price_band` | 以真實成交餵 `PriceBand`，每一筆逐筆成交在成交前的參考價下必須在 ±3.5 % 內。主動方由成交前的五檔判斷 | 2026-10-08、2025-04-07 至 04-11、2024-08-05 至 08-07 的 `TWSE_F6`、`TPEX_F6`（後兩段是大量觸發暫緩撮合的日子） |
| `a_real_aggressive_order_sent_again_gets_the_same_fills` | 取每本 book 第 2、10、50、250、1000 次真實的主動成交（一筆委託吃一檔以上），在它之前以同方向、同量的 `IOC` 重下，成交必須與真實的逐檔相同。每一次用獨立的 engine | 2026-10-08 `TWSE_F6`、`TPEX_F6`；2025-04-07 `TWSE_F6`；2024-08-05 `TPEX_F6` |
| `resting_orders_are_filled_only_by_what_the_data_shows` | 在每本 book 於 09:20（被動）、11:00（可立即成交）、13:26（被動）各下一筆，逐一稽核每個委託事件：成交價、觸發它的訊息種類、數量不超過該訊息的真實成交量或揭示量；每筆委託恰好以全部成交或失效結束 | 2026-10-08 四個 feed；2025-04-07 `TWSE_F6`；兩種 `auction_marginal` |
| `the_same_inputs_give_the_same_events` | 同一天跑兩次，委託事件完全相同 | 2026-10-08 `TPEX_F6` |

已知而且允許的差異：重下的委託少成交（真實委託吃超過五檔，或吃到沒有揭示過的量；不得超過
0.5 %），以及一筆差不到一個升降單位的範圍外成交（撮合規格第 6.6 節）。重下的委託多成交、價格
不同或被拒絕都是錯誤。

整組約需 2.5 GB 記憶體與半分鐘。

### 帳務

`osmium-ledger` 的正確性主要由不需要資料的測試保證：

- `tests/ledger.rs`：每一條[帳務規格](../reference/accounting.md)的規則一個手算的例子。
- `tests/invariants.rs`：隨機交易 40 個交易日、300 個 seed（現股、當沖、融資、融券、資券相抵、
  強制委託、部分成交），每天日結檢查：`cash` 等於期初現金加上已交割的分錄、待交割款等於尚未交割
  的分錄、權益的變動等於已實現加未實現損益、每筆已實現損益等於賣出淨額減成本加利息；以及
  `cash_check = "at_order"` 時，通過檢查的委託不論怎麼成交，交割日的現金都不是負數。

真實資料測試 `real_corporate_actions_keep_the_value_of_a_position`
（`crates/osmium-ledger/tests/marketdata.rs`）對 2022 年以來 `corp_actions` 的每一列：在除權息日
前一個交易日以 `before_price` 買進 10,000 股，除權息日以 `ref_price` 評價，比較兩天的權益。參考價
的定義就是讓持有人在除權息前後的價值相同，所以帳務處理正確時權益不變。`missing_terms` 的兩種
設定各跑一次，價值變動超過 1 % 的列不得超過千分之一。

實測（2026-11）：

| `missing_terms` | 套用 | 缺少事實而停止 | 變動超過 1 % |
| --- | --- | --- | --- |
| `"error"` | 11,164 | `dividend` 缺 `cash_dividend` 232、`reduction_loss` 98、`reduction_cash` 72、`par_change` 1 | 5 |
| `"approximate"` | 11,563 | `reduction_cash` 3、`par_change` 1 | 7 |

超過 1 % 的列都有已知的原因：面額不是 10 元的股票配發股票股利（5314、8932；這個測試不讀
`instruments`，實際的 run 會因為缺少 `par_value` 而停止）、股利金額與價格的幣別不同（00774C）、
`value` 與參考價不一致的特別股（8112A、9941A，只在近似時）。分割、反分割、面額變更與近似的減資，
價值變動都在萬分之八以內。

### 效能修改

只改變速度或記憶體的修改不更新任何版本（[版本](../reference/versions.md)），但必須證明輸出沒有變：

1. 修改前後各建一個 release 執行檔（修改前的可以用 `git archive <commit> | tar -x -C <dir>` 取出
   原始碼，在 repository 之外編譯）。
2. 兩個執行檔跑相同的設定，比較 `run.json` 的 `files`：

   ```sh
   diff <(jq -S .files before/run.json) <(jq -S .files after/run.json)
   ```

   至少涵蓋：全市場四個 feed 的一個交易日、同一個設定連續數個交易日、指定 `symbols` 的多日 run。
3. 修改載入或平行處理時，另外以不同的核心數各跑一次（`taskset -c 0`、`taskset -c 0-3`），`files`
   也必須相同；並重跑上面 `osmium-md`、`osmium-replay` 的真實資料測試與下一節的逐位元組比對。
4. 以 `/usr/bin/time -f "%e s %M KB"` 記錄時間與記憶體峰值，更新[效能](../design/performance.md)
   第 1 節與[執行](../guide/running.md#6-時間與記憶體)的表。

### 與獨立實作逐位元組比對

`osmium-md` 的解碼另有一份以 polars 寫的獨立實作
（[`crates/osmium-md/tools/reference_dump.py`](../../crates/osmium-md/tools/reference_dump.py)，
合併方式與 marketdata 的 `md.read.replay` 相同）。兩者把同一個 feed 的同一天輸出成文字，結果
必須位元組相同：

```sh
cargo run --release -p osmium-md --example dump -- /srv/md/v1 2026-10-08 TWSE_F6 > rust.tsv
~/marketdata/.venv/bin/python -I crates/osmium-md/tools/reference_dump.py /srv/md/v1 2026-10-08 TWSE_F6 python.tsv
cmp rust.tsv python.tsv
```

修改解碼、加入快取或平行處理之後都要重跑。日期至少涵蓋：每一個 feed、2024-04-01 以前的
`TWSE_F6`（[marketdata 介面](../reference/marketdata.md)第 6 節的舊來源）、`instruments` 是推導資料
的日子，以及指定商品的讀取。輸出檔放在 repository 之外。
