# Smoke fixture

這組資料由 `tools/acceptance/generate_synthetic_fixtures.py` 建立，只用於 CI、安裝後
smoke test 與 quickstart verification；它不是 Teralion payload，也不代表任何真實
市場資料。

資料內容刻意只包含兩筆虛構 `SYNTH-SMOKE` quote，並保留 book、`match_time`、
`received_at`、format 與 flags。較大的 M1–M5 acceptance payload 仍依
`fixtures/teralion/` 的 synthetic matrix 進行測試。
