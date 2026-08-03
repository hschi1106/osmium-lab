#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/acceptance/run_tpex_warrant_acceptance.sh --output <directory> [--network-runner auto|sandbox-exec]
EOF
}

output=
network_runner=auto
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            output=$2
            shift 2
            ;;
        --network-runner)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            network_runner=$2
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done
[ -n "$output" ] || { usage >&2; exit 2; }

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
case "$output" in
    /*) output_path=$output ;;
    *) output_path=$root/$output ;;
esac
case "$output_path" in
    "$root"/*) ;;
    *) echo "acceptance output must be inside the repository" >&2; exit 2 ;;
esac
[ ! -e "$output_path" ] || {
    echo "acceptance output already exists: $output_path" >&2
    exit 2
}
[ -z "$(git status --porcelain --untracked-files=no)" ] || {
    echo "tracked working tree must be clean before acceptance" >&2
    exit 2
}

case "$network_runner" in
    auto)
        [ "$(uname -s)" = Darwin ] || {
            echo "automatic network denial is only defined for Darwin" >&2
            exit 2
        }
        network_runner=sandbox-exec
        ;;
    sandbox-exec) ;;
    *) echo "unsupported network runner: $network_runner" >&2; exit 2 ;;
esac
command -v sandbox-exec >/dev/null 2>&1 || {
    echo "sandbox-exec is required" >&2
    exit 2
}

parent=$(dirname -- "$output_path")
base=$(basename -- "$output_path")
mkdir -p "$parent"
staging=$parent/.$base.staging.$$
cleanup() {
    if [ -d "$staging" ]; then
        failed=$output_path.failed
        if [ ! -e "$failed" ]; then
            mv "$staging" "$failed"
            echo "failed acceptance diagnostics=$failed" >&2
        else
            echo "failed acceptance diagnostics=$staging" >&2
        fi
    fi
}
trap cleanup EXIT HUP INT TERM
mkdir -p "$staging/test-results" "$staging/runs"

run_logged() {
    name=$1
    shift
    echo "running $name"
    "$@" >"$staging/test-results/$name.log" 2>&1
}

run_offline_logged() {
    name=$1
    audit=$2
    shift 2
    echo "running $name"
    env -u TERALION_API_KEY -u TERALION_BASE_URL OSMIUM_STREAM_OPEN_AUDIT="$audit" sandbox-exec -p '(version 1)(allow default)(deny network*)' "$@" >"$staging/test-results/$name.log" 2>&1
}

compare_dirs() {
    left=$1
    right=$2
    find "$left" -type f -print | sed "s#^$left/##" | LC_ALL=C sort >"$staging/test-results/.left-files"
    find "$right" -type f -print | sed "s#^$right/##" | LC_ALL=C sort >"$staging/test-results/.right-files"
    cmp "$staging/test-results/.left-files" "$staging/test-results/.right-files"
    while IFS= read -r file; do
        cmp "$left/$file" "$right/$file"
    done <"$staging/test-results/.left-files"
}

fixture_root=$root/fixtures/teralion
config=$staging/m5-tpex-warrant.yaml
sed "s#target/m5-tpex-warrant-data#$staging/data#g" config/m5-tpex-warrant.yaml >"$config"
binary=$root/target/release/osmium
fixture_builder=$root/target/release/osmium_fixture_data

run_logged fmt cargo fmt --check
run_logged normalizer-tests cargo test -p tpex-normalizer --test m5_warrant_fixture
run_logged debug-build cargo build -p osmium-cli -p osmium-config
run_logged release-build cargo build --release -p osmium-cli -p osmium-config
run_logged fixture-builder env CARGO_TARGET_DIR="$root/target" cargo build --release \
    --manifest-path "$root/tools/acceptance/osmium_fixture_data/Cargo.toml"
run_logged fixture-integrity "$root/tools/acceptance/verify_m5_fixtures.sh"
run_logged fixture-data "$fixture_builder" --config "$config" --fixtures "$fixture_root" --data-root "$staging/data"

run_offline_logged plan /dev/null "$binary" plan --config "$config"
run_offline_logged verify /dev/null "$binary" data verify --config "$config"
run_offline_logged replay "$staging/replay-open.log" "$binary" replay --config "$config"
run_offline_logged backtest "$staging/stream-open.log" "$binary" backtest --config "$config" --output "$staging/runs/run-a"
run_offline_logged inspect /dev/null "$binary" inspect --run "$staging/runs/run-a"

stream_count=$(wc -l <"$staging/stream-open.log" | tr -d ' ')
[ "$stream_count" -eq 1 ] || {
    echo "expected one opened stream, got $stream_count" >&2
    exit 1
}
rg -q "market=Tpex symbol=72328U " "$staging/stream-open.log" || {
    echo "stream-open audit is missing TPEx warrant 72328U" >&2
    exit 1
}
sort "$staging/stream-open.log" >"$staging/test-results/stream-open.log"

for run_number in 01 02 03 04 05 06 07 08 09 10; do
    run_offline_logged "repeat-$run_number" /dev/null "$binary" backtest --config "$config" --output "$staging/runs/repeat-$run_number"
    compare_dirs "$staging/runs/run-a" "$staging/runs/repeat-$run_number"
done
echo "repeated_runs=10 byte_identical=true" >"$staging/test-results/repeated-runs.log"

cp -R "$staging/data" "$staging/rebuild-data"
rm -rf "$staging/rebuild-data/cache"
rebuild_config=$staging/m5-tpex-warrant-rebuild.yaml
sed "s#$staging/data#$staging/rebuild-data#g" "$config" >"$rebuild_config"
run_offline_logged cache-rebuild /dev/null "$binary" cache prepare --config "$rebuild_config"
run_offline_logged rebuild-backtest /dev/null "$binary" backtest --config "$rebuild_config" --output "$staging/runs/rebuild"
compare_dirs "$staging/runs/run-a" "$staging/runs/rebuild"
echo "cache_rebuild=byte_identical=true" >"$staging/test-results/cache-rebuild.log"

run_offline_logged debug-backtest /dev/null "$root/target/debug/osmium" backtest --config "$config" --output "$staging/runs/debug"
compare_dirs "$staging/runs/run-a" "$staging/runs/debug"
echo "debug_release=byte_identical=true" >"$staging/test-results/debug-release.log"

cp -R "$staging/runs/run-a" "$staging/runs/corrupt"
printf '\001' >>"$staging/runs/corrupt/ledger.bin"
if run_offline_logged corruption-rejected /dev/null "$binary" inspect --run "$staging/runs/corrupt"; then
    echo "inspect accepted a corrupted ledger" >&2
    exit 1
fi
echo "ledger.bin mutation rejected=true" >"$staging/test-results/corruption.log"

summary=$staging/runs/run-a/run-summary.yaml
event_count=$(awk '/^events:/ {print $2; exit}' "$summary")
orders=$(awk '/^orders:/ {print $2; exit}' "$summary")
fills=$(awk '/^fills:/ {print $2; exit}' "$summary")
source_revision=$(awk -F': ' '/^source_revision:/ {print $2; exit}' "$staging/runs/run-a/data-lineage.yaml")
fixture_sha256=$(ruby -ryaml -e 'puts YAML.load_file(ARGV.fetch(0)).fetch("artifact").fetch("sha256")' "$root/fixtures/teralion/tpex/72328U/2026-07-20/metadata.yaml")
event_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/event-stream.blake3")
state_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/final-state.blake3")
strategy_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/strategy-output.blake3")
ledger_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/ledger.blake3")

git_commit=$(git rev-parse HEAD)
rust_toolchain=$(rustc --version)
cat >"$staging/acceptance-report.yaml" <<EOF
acceptance_contract_version: 1
milestone: M5
scope: TPEx warrant extension
status: Passed
completion_quality: focused-real-fixture
git_commit: $git_commit
rust_toolchain: "$rust_toolchain"
fixture:
  path: fixtures/teralion/tpex/72328U/2026-07-20
  record_count: 11
  formats: { WARRANT_REALTIME: 4, WARRANT_SNAPSHOT: 7 }
  sha256: $fixture_sha256
  source_revision: $source_revision
network_policy:
  status: disabled
  runner: $network_runner
  credentials: absent
run:
  config: config/m5-tpex-warrant.yaml
  events: $event_count
  opened_streams: 1
  orders: $orders
  fills: $fills
  event_stream_blake3: $event_checksum
  final_state_blake3: $state_checksum
  strategy_output_blake3: $strategy_checksum
  ledger_blake3: $ledger_checksum
determinism:
  repeated_runs: 10
  repeated_runs_byte_identical: true
  cache_rebuild_byte_identical: true
  debug_release_byte_identical: true
inspection:
  inspect_success: passed
  corruption_rejection: passed
scope_notes:
  trades: fixture contains no deals; TradeBatch support is not claimed
  discovery_permutations: not_applicable_single_stream
EOF

mv "$staging" "$output_path"
trap - EXIT HUP INT TERM
echo "acceptance=passed output=$output_path"
