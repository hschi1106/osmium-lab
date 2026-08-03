# Smoke fixture

這組資料是 repository-owned synthetic fixture，只用於 CI、安裝後 smoke test 與
quickstart verification；它不是 Teralion payload，也不代表任何真實市場資料或
redistribution permission。

資料內容刻意只包含兩筆 TWSE `2330` quote，並保留完整五檔、`match_time`、
`received_at`、format 與 flags。較大的 M1–M5 acceptance payload 仍依
`fixtures/acceptance/manifest.yaml` 的 private authorization policy 另行取得。
