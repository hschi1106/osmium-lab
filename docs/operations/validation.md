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

真實資料測試以 `#[ignore]` 標記，不在 workspace gate 內執行：

```sh
OSMIUM_MD_ROOT=/srv/md/v1 cargo test --locked -p osmium-md --release -- --ignored --nocapture
```

`OSMIUM_MD_ROOT` 未設定時使用 `/srv/md/v1`。這些測試只讀取資料，不寫入 marketdata 目錄。
