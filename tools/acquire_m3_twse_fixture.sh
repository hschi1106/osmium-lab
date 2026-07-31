#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
trading_date=2026-07-20
symbol=2330
start=2026-07-20T08:55:00+08:00
end=2026-07-20T13:35:00+08:00
kinds=quote
limit=5000
destination=${1:-"$root/raw/teralion/twse/$trading_date/$symbol/complete"}
staging="$destination.staging"
env_file=${TERALION_ENV_FILE:-"$root/.env"}

require_command() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "required command is unavailable: $1" >&2
        exit 2
    }
}

require_command curl
require_command jq
require_command shasum

# Parse only KEY=value lines. Do not source .env as shell code and never print the value.
env_value() {
    key=$1
    [ -f "$env_file" ] || return 0
    awk -v wanted="$key" '
        /^[[:space:]]*#/ { next }
        {
            line = $0
            sub(/^[[:space:]]*/, "", line)
            prefix = wanted "="
            spaced_prefix = wanted " ="
            if (index(line, prefix) == 1) {
                value = substr(line, length(prefix) + 1)
            } else if (index(line, spaced_prefix) == 1) {
                value = substr(line, length(spaced_prefix) + 1)
                sub(/^[[:space:]]*/, "", value)
            } else {
                next
            }
            sub(/[[:space:]]+$/, "", value)
            if (value ~ /^\".*\"$/ || value ~ /^\047.*\047$/) {
                value = substr(value, 2, length(value) - 2)
            }
            print value
            exit
        }
    ' "$env_file"
}

api_key=${TERALION_API_KEY:-}
if [ -z "$api_key" ]; then
    api_key=$(env_value TERALION_API_KEY)
fi
if [ -z "$api_key" ]; then
    echo "TERALION_API_KEY is required in .env or the environment" >&2
    exit 2
fi

base_url=${TERALION_BASE_URL:-}
if [ -z "$base_url" ]; then
    base_url=$(env_value TERALION_BASE_URL)
fi
base_url=${base_url:-https://app.teraliontech.com}

if [ -e "$destination" ]; then
    echo "refusing to overwrite completed acquisition: $destination" >&2
    exit 2
fi
if [ -e "$staging" ]; then
    echo "remove or inspect the previous incomplete acquisition: $staging" >&2
    exit 2
fi

mkdir -p "$staging/discovery" "$staging/pages"

fetch() {
    output=$1
    shift
    temporary="$output.tmp"
    curl --fail --silent --show-error --connect-timeout 10 --max-time 120 \
        --retry 4 --retry-all-errors --retry-delay 2 \
        -H "X-API-Key: $api_key" \
        --get "$@" \
        -o "$temporary"
    jq -e . "$temporary" >/dev/null
    mv "$temporary" "$output"
}

fetch "$staging/discovery/coverage.json" \
    "$base_url/api/feed/coverage" \
    --data-urlencode "start=$trading_date" \
    --data-urlencode "end=$trading_date"
jq -e --arg date "$trading_date" '
    .next_cursor == null
    and any(.items[]; .market == "twse" and .date == $date)
' "$staging/discovery/coverage.json" >/dev/null

fetch "$staging/discovery/instrument.json" \
    "$base_url/api/feed/instruments/$symbol" \
    --data-urlencode "date=$trading_date"
jq -e --arg symbol "$symbol" --arg date "$trading_date" '
    .symbol == $symbol
    and .market == "twse"
    and .trading_date == $date
    and (.session | type == "object")
' "$staging/discovery/instrument.json" >/dev/null

fetch "$staging/discovery/range.json" \
    "$base_url/api/feed/range/$symbol"
jq -e --arg symbol "$symbol" '
    .symbol == $symbol and .available == true
' "$staging/discovery/range.json" >/dev/null

jq -n \
    --arg symbol "$symbol" \
    --arg trading_date "$trading_date" \
    --arg start "$start" \
    --arg end "$end" \
    --arg kinds "$kinds" \
    --argjson limit "$limit" '
    {
      source: "teralion-feed-archive",
      endpoint: "/api/feed/ticks/{symbol}",
      market: "twse",
      symbol: $symbol,
      trading_date: $trading_date,
      filter_clock: "received_at",
      received_at_window: {start: $start, end: $end},
      kinds: ($kinds | split(",")),
      limit: $limit
    }
    ' >"$staging/request.json"

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
          source_formats: (
            .items
            | map(.format // "<missing>")
            | sort
            | group_by(.)
            | map({key: .[0], value: length})
            | from_entries
          ),
          first_match_time: (.items | map(.match_time) | min),
          last_match_time: (.items | map(.match_time) | max),
          first_received_at: (.items | map(.received_at) | min),
          last_received_at: (.items | map(.received_at) | max)
        }
    ' "$page"
}

summaries="$staging/page-summaries.jsonl"
: >"$summaries"
cursor=
page_number=1
while :; do
    page=$(printf '%s/%04d.json' "$staging/pages" "$page_number")
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
          and .market == "twse"
          and .type == "quote"
          and (.format == "STOCK_SNAPSHOT" or .format == "STOCK_REALTIME" or .format == "INTRADAY_ODDLOT_REALTIME")
          and .received_at >= $start
          and .received_at < $end
        )
    ' "$page" >/dev/null

    summarize_page "$page" >>"$summaries"
    next_cursor=$(jq -r '.next_cursor // empty' "$page")
    if [ -z "$next_cursor" ]; then
        break
    fi
    if [ "$next_cursor" = "$cursor" ]; then
        echo "cursor repeated at page $page_number" >&2
        exit 1
    fi
    cursor=$next_cursor
    page_number=$((page_number + 1))
    if [ "$page_number" -gt 10000 ]; then
        echo "pagination exceeded 10000 pages" >&2
        exit 1
    fi
done

jq -s --slurpfile request "$staging/request.json" '
    def merge_counts:
      reduce .[] as $page ({};
        reduce ($page.source_formats | to_entries[]) as $entry (.;
          .[$entry.key] = ((.[$entry.key] // 0) + $entry.value)
        )
      );
    {
      request: $request[0],
      cursor_complete: true,
      page_count: length,
      record_count: (map(.records) | add // 0),
      source_formats: merge_counts,
      first_match_time: (map(.first_match_time) | map(select(. != null)) | min),
      last_match_time: (map(.last_match_time) | map(select(. != null)) | max),
      first_received_at: (map(.first_received_at) | map(select(. != null)) | min),
      last_received_at: (map(.last_received_at) | map(select(. != null)) | max),
      pages: map({path, sha256, records})
    }
' "$summaries" >"$staging/summary.json"
rm "$summaries"

jq -n \
    --slurpfile request "$staging/request.json" \
    --slurpfile coverage "$staging/discovery/coverage.json" \
    --slurpfile instrument "$staging/discovery/instrument.json" \
    --slurpfile range "$staging/discovery/range.json" \
    --slurpfile summary "$staging/summary.json" '
    ($summary[0]) as $s |
    {
      state: "complete",
      source: "teralion-feed-archive",
      endpoint: "/api/feed/ticks/{symbol}",
      market: "twse",
      symbol: "2330",
      trading_date: "2026-07-20",
      filter_clock: "received_at",
      received_at_window: $request[0].received_at_window,
      kinds: $request[0].kinds,
      limit: $request[0].limit,
      cursor_complete: $s.cursor_complete,
      page_count: $s.page_count,
      record_count: $s.record_count,
      source_formats: $s.source_formats,
      first_match_time: $s.first_match_time,
      last_match_time: $s.last_match_time,
      first_received_at: $s.first_received_at,
      last_received_at: $s.last_received_at,
      coverage: $coverage[0],
      instrument: $instrument[0],
      range_at_download: $range[0]
    }
' >"$staging/acquisition.json"

(
    cd "$staging"
    find . -type f ! -name checksums.sha256 -print |
        LC_ALL=C sort |
        while IFS= read -r path; do
            shasum -a 256 "$path"
        done >checksums.sha256
)

if grep -E -i -R \
    '"(authorization|api[_-]?key|cookie|password|secret)"[[:space:]]*:' \
    "$staging" >/dev/null; then
    echo "secret scan found a forbidden field" >&2
    exit 1
fi

mkdir -p "$(dirname "$destination")"
mv "$staging" "$destination"
echo "M3 TWSE acquisition complete: $destination"
