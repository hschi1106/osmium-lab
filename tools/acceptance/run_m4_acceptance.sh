#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/acceptance/run_m4_acceptance.sh --output <directory> [--network-runner auto|sandbox-exec]

The harness validates the committed TPEx fixture and executes the five-instrument
offline M4 path together with the existing TWSE and TAIFEX fixture partitions.
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
[ ! -e "$output_path" ] || { echo "acceptance output already exists: $output_path" >&2; exit 2; }

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "tracked working tree must be clean before acceptance" >&2
    exit 2
fi

case "$network_runner" in
    auto)
        case "$(uname -s)" in
            Darwin) network_runner=sandbox-exec ;;
            *) echo "automatic network denial is only defined for Darwin" >&2; exit 2 ;;
        esac
        ;;
    sandbox-exec) ;;
    *) echo "unsupported network runner: $network_runner" >&2; exit 2 ;;
esac

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
    command -v sandbox-exec >/dev/null 2>&1 || {
        echo "sandbox-exec is required" >&2
        exit 2
    }
    env -u TERALION_API_KEY OSMIUM_STREAM_OPEN_AUDIT="$audit" \
        sandbox-exec -p '(version 1)(allow default)(deny network*)' \
        "$@" >"$staging/test-results/$name.log" 2>&1
}

fixture_root=$root/fixtures/teralion
config=$staging/m4-tpex.yaml
sed "s#target/m4-data#$staging/data#g" config/m4-tpex.yaml >"$config"
binary=$root/target/release/osmium
fixture_builder=$root/target/release/osmium_fixture_data

run_logged fmt cargo fmt --check
run_logged clippy cargo clippy --workspace --all-targets -- -D warnings
run_logged workspace-debug cargo test --workspace
run_logged workspace-release cargo test --workspace --release
run_logged debug-build cargo build -p osmium-cli -p osmium-config
run_logged release-build cargo build --release -p osmium-cli -p osmium-config
run_logged fixture-builder env CARGO_TARGET_DIR="$root/target" cargo build --release \
    --manifest-path "$root/tools/acceptance/osmium_fixture_data/Cargo.toml"
run_logged fixture-integrity "$root/tools/acceptance/verify_m4_fixture.sh"
run_logged tpex-normalizer-tests cargo test -p tpex-normalizer --tests

run_logged fixture-data "$fixture_builder" \
    --config "$config" --fixtures "$fixture_root" --data-root "$staging/data"
run_offline_logged plan /dev/null "$binary" plan --config "$config"
run_offline_logged verify /dev/null "$binary" data verify --config "$config"
run_offline_logged replay "$staging/replay.log" "$binary" replay --config "$config"
run_offline_logged backtest "$staging/stream-open.log" "$binary" backtest \
    --config "$config" --output "$staging/runs/run-a"
run_offline_logged inspect /dev/null "$binary" inspect --run "$staging/runs/run-a"

compare_dirs() {
    left=$1
    right=$2
    find "$left" -type f -print | sed "s#^$left/##" | LC_ALL=C sort \
        >"$staging/test-results/.left-files"
    find "$right" -type f -print | sed "s#^$right/##" | LC_ALL=C sort \
        >"$staging/test-results/.right-files"
    cmp "$staging/test-results/.left-files" "$staging/test-results/.right-files"
    while IFS= read -r file; do
        cmp "$left/$file" "$right/$file"
    done <"$staging/test-results/.left-files"
}

stream_count=$(wc -l <"$staging/stream-open.log" | tr -d ' ')
[ "$stream_count" -eq 5 ] || {
    echo "expected five opened streams, got $stream_count" >&2
    exit 1
}
for symbol in 2330 6488 CAFH6 CDFH6 TXFH6; do
    rg -q "symbol=$symbol " "$staging/stream-open.log" || {
        echo "stream-open audit is missing $symbol" >&2
        exit 1
    }
done
sort "$staging/stream-open.log" >"$staging/test-results/stream-open-audit.log"

for run_number in 01 02 03 04 05 06 07 08 09 10; do
    run_offline_logged "repeat-$run_number" /dev/null "$binary" backtest \
        --config "$config" --output "$staging/runs/repeat-$run_number"
    compare_dirs "$staging/runs/run-a" "$staging/runs/repeat-$run_number"
done
echo "repeated_runs=10 byte_identical=true" >"$staging/test-results/repeated-runs.log"

for permutation in 1 2 3; do
    permutation_config=$staging/permutation-$permutation.yaml
    ruby -ryaml -e '
      config = YAML.load_file(ARGV[0])
      instruments = config.fetch("universe").fetch("instruments")
      config["universe"]["instruments"] = case ARGV[2].to_i
        when 1 then instruments.reverse
        when 2 then instruments.rotate(1)
        else instruments.rotate(2)
      end
      File.write(ARGV[1], config.to_yaml)
    ' "$config" "$permutation_config" "$permutation"
    run_offline_logged "permutation-$permutation" /dev/null "$binary" backtest \
        --config "$permutation_config" --output "$staging/runs/permutation-$permutation"
    compare_dirs "$staging/runs/run-a" "$staging/runs/permutation-$permutation"
done
echo "discovery_permutations=3 byte_identical=true" \
    >"$staging/test-results/discovery-permutations.log"

cp -R "$staging/data" "$staging/rebuild-data"
rm -rf "$staging/rebuild-data/cache"
rebuild_config=$staging/rebuild.yaml
sed "s#$staging/data#$staging/rebuild-data#g" "$config" >"$rebuild_config"
run_offline_logged cache-rebuild /dev/null "$binary" cache prepare --config "$rebuild_config"
run_offline_logged rebuild-backtest /dev/null "$binary" backtest \
    --config "$rebuild_config" --output "$staging/runs/rebuild"
compare_dirs "$staging/runs/run-a" "$staging/runs/rebuild"
echo "cache_rebuild=byte_identical=true" >"$staging/test-results/cache-rebuild.log"

run_offline_logged debug-backtest /dev/null "$root/target/debug/osmium" backtest \
    --config "$config" --output "$staging/runs/debug"
compare_dirs "$staging/runs/run-a" "$staging/runs/debug"
echo "debug_release=byte_identical=true" >"$staging/test-results/debug-release.log"

cp -R "$staging/runs/run-a" "$staging/runs/corrupt"
printf '\001' >>"$staging/runs/corrupt/ledger.bin"
if run_offline_logged corruption-rejected /dev/null "$binary" inspect \
    --run "$staging/runs/corrupt"; then
    echo "inspect accepted a corrupted ledger" >&2
    exit 1
fi
echo "ledger.bin mutation rejected=true" >"$staging/test-results/corruption.log"

start_seconds=$(date +%s)
run_offline_logged performance /dev/null "$binary" backtest --config "$config" \
    --output "$staging/runs/performance"
end_seconds=$(date +%s)
event_count=$(awk '/^events:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
source_bytes=$(du -sk "$staging/data/source" | awk '{print $1 * 1024}')
cache_bytes=$(du -sk "$staging/data/cache" | awk '{print $1 * 1024}')
cat >"$staging/test-results/performance.yaml" <<EOF
dataset: m4-tpex-five-instrument
events: $event_count
source_bytes: $source_bytes
cache_bytes: $cache_bytes
elapsed_seconds: $((end_seconds - start_seconds))
EOF

git_commit=$(git rev-parse HEAD)
rust_toolchain=$(rustc --version)
event_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/event-stream.blake3")
state_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/final-state.blake3")
strategy_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/strategy-output.blake3")
orders_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/orders.blake3")
fills_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/fills.blake3")
ledger_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/ledger.blake3")
orders=$(awk '/^orders:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
fills=$(awk '/^fills:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
cat >"$staging/test-results/artifact-checksums.yaml" <<EOF
events: $event_count
orders: $orders
fills: $fills
event_stream_blake3: $event_checksum
final_state_blake3: $state_checksum
strategy_output_blake3: $strategy_checksum
orders_blake3: $orders_checksum
fills_blake3: $fills_checksum
ledger_blake3: $ledger_checksum
EOF

cat >"$staging/acceptance-report.yaml" <<EOF
acceptance_contract_version: 1
verification_plan_version: 3
milestone: M4
status: Passed
completion_quality: full
git_commit: $git_commit
rust_toolchain: "$rust_toolchain"
schema_versions:
  market_types: 3
  event_schema: 3
  canonical_event: 3
  market_state: 3
  canonical_market_state: 3
  canonical_final_state_set: 3
scope:
  tpex_regular_equity: passed
  twse_and_taifex_regression: passed
fixture_gate:
  tpex_fixture: passed
  daily_instrument: passed
  source_checksums: passed
  secret_scan: passed
network_policy:
  status: disabled
  runner: $network_runner
  credential: absent
run:
  config: config/m4-tpex.yaml
  events: $event_count
  opened_streams: $stream_count
  orders: $orders
  fills: $fills
  event_stream_blake3: $event_checksum
  final_state_blake3: $state_checksum
  strategy_output_blake3: $strategy_checksum
  orders_blake3: $orders_checksum
  fills_blake3: $fills_checksum
  ledger_blake3: $ledger_checksum
determinism:
  repeated_runs: 10
  repeated_run_directories_byte_identical: true
  discovery_permutations: 3
  discovery_permutations_byte_identical: true
  cache_rebuild_byte_identical: true
  debug_release_byte_identical: true
inspection:
  inspect_success: passed
  corruption_rejection: passed
performance:
  evidence: test-results/performance.yaml
  threshold: not_set
acceptance:
  M4-AC-01: { status: Passed, evidence: test-results/fixture-integrity.log + fixture metadata }
  M4-AC-02: { status: Passed, evidence: fixture-data.log + replay.log }
  M4-AC-03: { status: Passed, evidence: tpex-normalizer-tests.log }
  M4-AC-04: { status: Passed, evidence: tpex-normalizer-tests.log + artifact-checksums.yaml }
  M4-AC-05: { status: Passed, evidence: cache-rebuild.log + fixture-data.log }
  M4-AC-06: { status: Passed, evidence: tpex-normalizer-tests.log + replay.log }
  M4-AC-07: { status: Passed, evidence: stream-open-audit.log + replay.log }
  M4-AC-08: { status: Passed, evidence: stream-open-audit.log }
  M4-AC-09: { status: Passed, evidence: artifact-checksums.yaml + workspace-debug.log }
  M4-AC-10: { status: Passed, evidence: repeated-runs.log + discovery-permutations.log + debug-release.log }
  M4-AC-11: { status: Passed, evidence: plan.log + verify.log + replay.log + backtest.log + inspect.log }
  M4-AC-12: { status: Passed, evidence: corruption.log + fixture-integrity.log + performance.yaml }
approver: tools/acceptance/run_m4_acceptance.sh
EOF

# Source/cache and repeated-run directories are derived and intentionally not committed.
# The report, test logs and canonical artifact checksums are the durable evidence.
rm -rf "$staging/data" "$staging/rebuild-data" "$staging/runs"
find "$staging" -maxdepth 1 -name 'permutation-*.yaml' -delete
find "$staging" -maxdepth 1 -name 'rebuild.yaml' -delete
find "$staging/test-results" -maxdepth 1 -name '.left-files' -delete
find "$staging/test-results" -maxdepth 1 -name '.right-files' -delete

mv "$staging" "$output_path"
trap - EXIT HUP INT TERM
echo "M4 acceptance completed: five-instrument scope passed"
echo "output=$output_path"
