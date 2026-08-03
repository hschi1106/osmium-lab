# Teralion compact representative fixtures

這些 payload 只保留足以代表 parser／normalizer／replay 狀態的 records。每個 profile
都使用 deterministic selection，保留 session boundary、format、status／limit state
與 first/last records；`metadata.yaml` 的 `complete_day` 固定為 `false`。

| Market | Symbol | Date | Sessions | Scope |
| --- | --- | --- | --- | --- |
| TWSE | `2330` | `2026-07-20` | regular | equity snapshot/realtime、opening／closing states |
| TWSE | `03003T` | `2026-07-20` | regular | warrant |
| TPEx | `6488` | `2026-07-20` | regular | equity |
| TPEx | `72328U` | `2026-07-20` | regular | warrant |
| TAIFEX | `CAFH6` | `2026-07-20` | regular | futures |
| TAIFEX | `CDFH6` | `2026-07-20` | after-hours, regular | futures session split |
| TAIFEX | `TXFH6` | `2026-07-20` | after-hours, regular | futures cross-session |
| TAIFEX | `TXFH6` | `2026-07-28` | after-hours, regular | futures／underlying relationship |
| TAIFEX | `TXO24000U6` | `2026-07-28` | after-hours, regular | option session split |

TWSE 的 `2026-07-27` payload 不在此 tree。需要完整日資料時，請依
`fixtures/acceptance/manifest.yaml` 取得 authorized private bundle；不要把 compact
slice 當成 complete-day input。

每個 profile 具有：

```text
daily.json
metadata.yaml
<session>/0001.jsonl
golden/fixture-set.sha256
```

`tools/acceptance/verify_compact_fixtures.sh` 會驗證 manifest、metadata、JSON、checksum、
日期規則、session size/record budget 與 golden artifact allowlist。
