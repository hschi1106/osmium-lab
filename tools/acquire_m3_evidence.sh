#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
trading_date=2026-07-20
destination=${1:-"$root/raw/teralion/taifex/$trading_date/evidence"}
staging="$destination.staging"
base_url=${TERALION_BASE_URL:-https://app.teraliontech.com}
kinds=book,close,stats,trade
limit=5000

require_command() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "required command is unavailable: $1" >&2
        exit 2
    }
}

require_command curl
require_command jq
require_command shasum

if [ -z "${TERALION_API_KEY:-}" ]; then
    echo "TERALION_API_KEY is required" >&2
    exit 2
fi

if [ -e "$destination" ]; then
    echo "refusing to overwrite completed acquisition: $destination" >&2
    exit 2
fi
if [ -e "$staging" ]; then
    echo "remove or inspect the previous incomplete acquisition: $staging" >&2
    exit 2
fi

mkdir -p "$staging/discovery" "$staging/partitions"

fetch() {
    output=$1
    shift
    temporary="$output.tmp"
    curl --fail --silent --show-error --connect-timeout 10 --max-time 120 \
        --retry 4 --retry-all-errors --retry-delay 2 \
        -H "X-API-Key: $TERALION_API_KEY" \
        --get "$@" \
        -o "$temporary"
    jq -e . "$temporary" >/dev/null
    mv "$temporary" "$output"
}

fetch "$staging/discovery/coverage.json" \
    "$base_url/api/feed/coverage" \
    --data-urlencode "start=$trading_date" \
    --data-urlencode "end=$trading_date"

fetch "$staging/discovery/instruments.json" \
    "$base_url/api/feed/instruments" \
    --data-urlencode "market=taifex_fut" \
    --data-urlencode "date=$trading_date" \
    --data-urlencode "limit=$limit"

jq -e '
    .next_cursor == null
    and (.items | type == "array")
    and any(.items[]; .market == "taifex_fut")
' "$staging/discovery/instruments.json" >/dev/null

fetch_instrument() {
    symbol=$1
    market=$2

    fetch "$staging/discovery/instrument-$symbol.json" \
        "$base_url/api/feed/instruments/$symbol" \
        --data-urlencode "date=$trading_date"
    fetch "$staging/discovery/range-$symbol.json" \
        "$base_url/api/feed/range/$symbol"

    jq -e --arg symbol "$symbol" --arg market "$market" --arg date "$trading_date" '
        .symbol == $symbol
        and .market == $market
        and .trading_date == $date
    ' "$staging/discovery/instrument-$symbol.json" >/dev/null
    jq -e --arg symbol "$symbol" '
        .symbol == $symbol and .available == true
    ' "$staging/discovery/range-$symbol.json" >/dev/null
}

fetch_instrument TXFH6 taifex_fut
fetch_instrument CDFH6 taifex_fut
fetch_instrument CAFH6 taifex_fut
fetch_instrument 2330 twse

summarize_page() {
    page=$1
    checksum=$(shasum -a 256 "$page" | awk '{print $1}')
    jq \
        --arg path "${page#"$staging/"}" \
        --arg sha256 "$checksum" '
        {
          path: $path,
          sha256: $sha256,
          records: (.items | length),
          source_types: (
            .items
            | group_by(.type)
            | map({key: .[0].type, value: length})
            | from_entries
          ),
          source_formats: (
            .items
            | group_by(.format)
            | map({key: .[0].format, value: length})
            | from_entries
          ),
          first_match_time: (.items | map(.match_time) | min),
          last_match_time: (.items | map(.match_time) | max),
          first_received_at: (.items | map(.received_at) | min),
          last_received_at: (.items | map(.received_at) | max)
        }
    ' "$page"
}

fetch_partition() {
    symbol=$1
    segment=$2
    start=$3
    end=$4
    partition="$staging/partitions/$symbol/$segment"
    pages="$partition/pages"
    summaries="$partition/page-summaries.jsonl"
    cursor=
    page_number=1

    mkdir -p "$pages"
    jq -n \
        --arg symbol "$symbol" \
        --arg trading_date "$trading_date" \
        --arg segment "$segment" \
        --arg start "$start" \
        --arg end "$end" \
        --arg kinds "$kinds" \
        --argjson limit "$limit" '
        {
          symbol: $symbol,
          market: "taifex_fut",
          trading_date: $trading_date,
          segment: $segment,
          filter_clock: "received_at",
          start: $start,
          end: $end,
          kinds: ($kinds | split(",")),
          limit: $limit
        }
    ' >"$partition/request.json"

    : >"$summaries"
    while :; do
        page=$(printf '%s/%04d.json' "$pages" "$page_number")
        if [ -n "$cursor" ]; then
            fetch "$page" \
                "$base_url/api/feed/ticks/$symbol" \
                --data-urlencode "start=$start" \
                --data-urlencode "end=$end" \
                --data-urlencode "kinds=$kinds" \
                --data-urlencode "limit=$limit" \
                --data-urlencode "cursor=$cursor"
        else
            fetch "$page" \
                "$base_url/api/feed/ticks/$symbol" \
                --data-urlencode "start=$start" \
                --data-urlencode "end=$end" \
                --data-urlencode "kinds=$kinds" \
                --data-urlencode "limit=$limit"
        fi

        jq -e \
            --arg symbol "$symbol" \
            --arg start "$start" \
            --arg end "$end" \
            --arg kinds "$kinds" '
            (.items | type == "array")
            and (.next_cursor == null or (.next_cursor | type == "string" and length > 0))
            and all(
              .items[];
              .symbol == $symbol
              and .market == "taifex_fut"
              and (.type as $type | ($kinds | split(",") | index($type)) != null)
              and .received_at >= $start
              and .received_at <= $end
            )
        ' "$page" >/dev/null

        summarize_page "$page" >>"$summaries"
        cursor=$(jq -r '.next_cursor // empty' "$page")
        [ -n "$cursor" ] || break
        page_number=$((page_number + 1))
    done

    jq -s \
        --slurpfile request "$partition/request.json" '
        def merge_counts($field):
          reduce .[] as $page ({};
            reduce ($page[$field] | to_entries[]) as $entry (.;
              .[$entry.key] = ((.[$entry.key] // 0) + $entry.value)
            )
          );
        {
          request: $request[0],
          cursor_complete: true,
          page_count: length,
          record_count: (map(.records) | add // 0),
          source_types: merge_counts("source_types"),
          source_formats: merge_counts("source_formats"),
          first_match_time: (map(.first_match_time) | map(select(. != null)) | min),
          last_match_time: (map(.last_match_time) | map(select(. != null)) | max),
          first_received_at: (map(.first_received_at) | map(select(. != null)) | min),
          last_received_at: (map(.last_received_at) | map(select(. != null)) | max),
          pages: map({path, sha256, records})
        }
    ' "$summaries" >"$partition/summary.json"
    rm "$summaries"

    count=$(jq -r '.record_count' "$partition/summary.json")
    echo "$symbol $segment: $count records in $page_number pages"
}

# Trading date 2026-07-20 follows the Friday 2026-07-17 after-hours session.
fetch_partition TXFH6 after_hours \
    2026-07-17T14:55:00+08:00 2026-07-18T05:05:00+08:00
fetch_partition TXFH6 regular \
    2026-07-20T08:40:00+08:00 2026-07-20T13:50:00+08:00
fetch_partition CDFH6 after_hours \
    2026-07-17T17:20:00+08:00 2026-07-18T05:05:00+08:00
fetch_partition CDFH6 regular \
    2026-07-20T08:40:00+08:00 2026-07-20T13:50:00+08:00
fetch_partition CAFH6 regular \
    2026-07-20T08:40:00+08:00 2026-07-20T13:50:00+08:00

(
    cd "$staging"
    find . -type f ! -name checksums.sha256 -print |
        LC_ALL=C sort |
        while IFS= read -r path; do
            shasum -a 256 "$path"
        done >checksums.sha256
)

if grep -E -i -R \
    '"(authorization|api[_-]?key|cookie|password|secret|token)"[[:space:]]*:' \
    "$staging" >/dev/null; then
    echo "secret scan found a forbidden field" >&2
    exit 1
fi

mv "$staging" "$destination"
echo "M3 evidence acquisition complete: $destination"
