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

## 真實資料

真實資料測試以 `#[ignore]` 標記，不在 workspace gate 內執行：

```sh
OSMIUM_MD_ROOT=/srv/md/v1 cargo test --locked -p osmium-md --release -- --ignored --nocapture
OSMIUM_MD_ROOT=/srv/md/v1 cargo test --locked -p osmium-replay --release -- --ignored --nocapture
OSMIUM_MD_ROOT=/srv/md/v1 cargo test --locked -p osmium-matching --release --test marketdata -- --ignored --nocapture
```

`OSMIUM_MD_ROOT` 未設定時使用 `/srv/md/v1`。這些測試只讀取資料，不寫入 marketdata 目錄。

| Crate | 檢查的內容 |
| --- | --- |
| `osmium-md` | 每個 feed 的訊息種類、列數對帳、`observed_time`、指定商品與整個 feed 讀到相同的訊息 |
| `osmium-replay` | 四個 feed 合併後的全序與時間線；每本 book 恰有一列開盤與一列收盤撮合列、撮合方式的推論與 `match_interval_s` 相同、每一則訊息的 `phase` 與 [`osmium-book` 的模式](../reference/matching.md)一致 |

| `osmium-matching` | 見下方 |

`osmium-replay` 的測試同時載入數個 feed 的一整天，約需 6 GB 記憶體。

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

整組約需 8 GB 記憶體與數分鐘。

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
`TWSE_F6`（[marketdata 介面](../reference/marketdata.md)第 6 節的形狀）、`instruments` 是推導資料
的日子，以及指定商品的讀取。輸出檔放在 repository 之外。
