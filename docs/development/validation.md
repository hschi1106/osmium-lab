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
```

`OSMIUM_MD_ROOT` 未設定時使用 `/srv/md/v1`。這些測試只讀取資料，不寫入 marketdata 目錄。

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
