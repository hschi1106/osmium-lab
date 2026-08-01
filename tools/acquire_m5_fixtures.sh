#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
base_url=${TERALION_BASE_URL:-https://app.teraliontech.com}
limit=5000
destination_root=${1:-"$root/raw/teralion"}

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
    echo "TERALION_API_KEY is required; it is never written to an artifact" >&2
    exit 2
fi

fetch() {
    output=$1
    shift
    temporary="$output.tmp"
    curl --fail --silent --show-error --connect-timeout 10 --max-time 120 \
        --retry 4 --retry-all-errors --retry-delay 2 \
        -H "X-API-Key: $TERALION_API_KEY" \
        --get "$@" -o "$temporary"
    jq -e . "$temporary" >/dev/null
    mv "$temporary" "$output"
}

prepare_instrument() {
    market=$1
    date=$2
    symbol=$3
    source_market=$4
    partition="$destination_root/$market/$date/$symbol/complete"
    staging="$partition.staging"
    if [ -e "$partition" ]; then
        echo "refusing to overwrite completed acquisition: $partition" >&2
        exit 2
    fi
    if [ -e "$staging" ]; then
        echo "remove or inspect incomplete acquisition: $staging" >&2
        exit 2
    fi
    mkdir -p "$staging/discovery" "$staging/pages"

    fetch "$staging/discovery/coverage.json" \
        "$base_url/api/feed/coverage" \
        --data-urlencode "start=$date" \
        --data-urlencode "end=$date"
    fetch "$staging/discovery/instrument.json" \
        "$base_url/api/feed/instruments/$symbol" \
        --data-urlencode "date=$date"
    fetch "$staging/discovery/range.json" \
        "$base_url/api/feed/range/$symbol"
    jq -e \
        --arg symbol "$symbol" \
        --arg market "$source_market" \
        --arg date "$date" \
        '.symbol == $symbol and .market == $market and .trading_date == $date' \
        "$staging/discovery/instrument.json" >/dev/null
    jq -e --arg symbol "$symbol" '.symbol == $symbol and .available == true' \
        "$staging/discovery/range.json" >/dev/null

    jq -n \
        --arg market "$source_market" \
        --arg symbol "$symbol" \
        --arg date "$date" \
        '{source: "teralion", source_market: $market, symbol: $symbol,
          trading_date: $date, cursor_policy: "opaque_cursor_to_terminal",
          api_key_written: false, redistribution: "private-internal-review-only"}' \
        >"$staging/acquisition.json"
}

summarize_page() {
    page=$1
    checksum=$(shasum -a 256 "$page" | awk '{print $1}')
    jq \
        --arg path "${page#"$current_staging/"}" \
        --arg sha256 "$checksum" \
        '{path: $path, sha256: $sha256, records: (.items | length),
          source_types: (.items | group_by(.type) | map({key: .[0].type, value: length}) | from_entries),
          source_formats: (.items | group_by(.format) | map({key: .[0].format, value: length}) | from_entries),
          first_match_time: (.items | map(.match_time) | min),
          last_match_time: (.items | map(.match_time) | max),
          first_received_at: (.items | map(.received_at) | min),
          last_received_at: (.items | map(.received_at) | max)}' "$page"
}

fetch_partition() {
    market=$1
    date=$2
    symbol=$3
    source_market=$4
    segment=$5
    start=$6
    end=$7
    kinds=$8
    current_staging="$destination_root/$market/$date/$symbol/complete.staging"
    partition="$current_staging/partitions/$segment"
    pages="$partition/pages"
    summaries="$partition/page-summaries.jsonl"
    mkdir -p "$pages"
    jq -n \
        --arg market "$source_market" --arg symbol "$symbol" --arg date "$date" \
        --arg segment "$segment" --arg start "$start" --arg end "$end" \
        --arg kinds "$kinds" --argjson limit "$limit" \
        '{market: $market, symbol: $symbol, trading_date: $date, segment: $segment,
          filter_clock: "received_at", start: $start, end: $end,
          kinds: ($kinds | split(",")), limit: $limit,
          cursor_complete: true}' >"$partition/request.json"
    : >"$summaries"
    cursor=
    page_number=1
    while :; do
        page=$(printf '%s/%04d.json' "$pages" "$page_number")
        if [ -n "$cursor" ]; then
            fetch "$page" "$base_url/api/feed/ticks/$symbol" \
                --data-urlencode "start=$start" --data-urlencode "end=$end" \
                --data-urlencode "kinds=$kinds" --data-urlencode "limit=$limit" \
                --data-urlencode "cursor=$cursor"
        else
            fetch "$page" "$base_url/api/feed/ticks/$symbol" \
                --data-urlencode "start=$start" --data-urlencode "end=$end" \
                --data-urlencode "kinds=$kinds" --data-urlencode "limit=$limit"
        fi
        jq -e \
            --arg symbol "$symbol" --arg market "$source_market" \
            --arg start "$start" --arg end "$end" --arg kinds "$kinds" '
            (.items | type == "array")
            and (.next_cursor == null or (.next_cursor | type == "string" and length > 0))
            and all(.items[];
              .symbol == $symbol and .market == $market
              and (.type as $type | ($kinds | split(",") | index($type)) != null)
              and .received_at >= $start and .received_at <= $end)' "$page" >/dev/null
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
              .[$entry.key] = ((.[$entry.key] // 0) + $entry.value)));
        {request: $request[0], cursor_complete: true, page_count: length,
         record_count: (map(.records) | add // 0),
         source_types: merge_counts("source_types"), source_formats: merge_counts("source_formats"),
         first_match_time: (map(.first_match_time) | map(select(. != null)) | min),
         last_match_time: (map(.last_match_time) | map(select(. != null)) | max),
         pages: map({path, sha256, records})}' "$summaries" >"$partition/summary.json"
    rm "$summaries"
}

finish_instrument() {
    market=$1
    date=$2
    symbol=$3
    current_staging="$destination_root/$market/$date/$symbol/complete.staging"
    jq -n \
        --slurpfile instrument "$current_staging/discovery/instrument.json" \
        --slurpfile range "$current_staging/discovery/range.json" \
        --slurpfile coverage "$current_staging/discovery/coverage.json" \
        '{instrument: $instrument[0], range: $range[0], coverage: $coverage[0],
          partitions: [inputs]}' \
        < /dev/null >"$current_staging/discovery/manifest.json"
    (
        cd "$current_staging"
        find . -type f ! -name checksums.sha256 -print |
            LC_ALL=C sort |
            while IFS= read -r path; do shasum -a 256 "$path"; done >checksums.sha256
    )
    if grep -E -i -R \
        '"(authorization|api[_-]?key|cookie|password|secret|token)"[[:space:]]*:' \
        "$current_staging" >/dev/null; then
        echo "secret scan found a forbidden field" >&2
        exit 1
    fi
    mv "$current_staging" "$destination_root/$market/$date/$symbol/complete"
}

# M5-W: TWSE warrant, regular session only.
prepare_instrument twse 2026-07-20 03003T twse
current_staging="$destination_root/twse/2026-07-20/03003T/complete.staging"
fetch_partition twse 2026-07-20 03003T twse regular \
    2026-07-20T08:55:00+08:00 2026-07-20T13:35:00+08:00 quote
finish_instrument twse 2026-07-20 03003T

# M5-O: TAIFEX index option, one cross-day query covering both sessions.
prepare_instrument taifex_opt 2026-07-28 TXO24000U6 taifex_opt
current_staging="$destination_root/taifex_opt/2026-07-28/TXO24000U6/complete.staging"
fetch_partition taifex_opt 2026-07-28 TXO24000U6 taifex_opt combined \
    2026-07-27T14:55:00+08:00 2026-07-28T13:50:00+08:00 book,close,stats,trade
finish_instrument taifex_opt 2026-07-28 TXO24000U6

echo "M5 fixture acquisition complete: $destination_root"
