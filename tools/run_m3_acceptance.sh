#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/run_m3_acceptance.sh --output <directory> [--network-runner auto|sandbox-exec|docker]

The harness validates the committed TWSE and TAIFEX fixtures and executes both the
three-instrument and full four-instrument offline paths.
EOF
}

output=
network_runner=auto
allow_blocked=0
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
        --allow-blocked)
            # Kept as a compatibility flag for old reproduction commands. The
            # committed TWSE companion fixture now makes the full gate mandatory.
            allow_blocked=1
            shift
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

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
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
            Linux) network_runner=docker ;;
            *) echo "no automatic network-denial runner for $(uname -s)" >&2; exit 2 ;;
        esac
        ;;
    sandbox-exec|docker) ;;
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
    case "$network_runner" in
        sandbox-exec)
            command -v sandbox-exec >/dev/null 2>&1 || { echo "sandbox-exec is required" >&2; exit 2; }
            env -u TERALION_API_KEY OSMIUM_STREAM_OPEN_AUDIT="$audit" \
                sandbox-exec -p '(version 1)(allow default)(deny network*)' \
                "$@" >"$staging/test-results/$name.log" 2>&1
            ;;
        docker)
            command -v docker >/dev/null 2>&1 || { echo "docker is required" >&2; exit 2; }
            docker run --rm --network none --cap-drop ALL \
                --security-opt no-new-privileges \
                --user "$(id -u):$(id -g)" \
                --volume "$root:$root" --workdir "$root" ubuntu:24.04 \
                env -u TERALION_API_KEY OSMIUM_STREAM_OPEN_AUDIT="$audit" "$@" \
                >"$staging/test-results/$name.log" 2>&1
            ;;
    esac
}

fixture_root=$root/fixtures/teralion
three_config=$staging/m3-taifex-three.yaml
sed "s#target/m3-taifex-data#$staging/data#g" \
    config/m3-taifex-three.yaml >"$three_config"
binary=$root/target/release/osmium
fixture_builder=$root/target/release/m3_fixture_data

run_logged fmt cargo fmt --check
run_logged clippy cargo clippy --workspace --all-targets -- -D warnings
run_logged workspace-debug cargo test --workspace
run_logged workspace-release cargo test --workspace --release
run_logged debug-build cargo build -p osmium-cli -p m3-config
run_logged release-build cargo build --release -p osmium-cli -p m3-config
run_logged fixture-integrity "$root/tools/verify_m3_fixtures.sh"
run_logged taifex-fixture-tests cargo test -p taifex-normalizer --test fixtures
run_logged source-contract-tests cargo test -p data-sync taifex_query_accepts_wire_kinds_and_market

run_logged fixture-data "$fixture_builder" \
    --config "$three_config" --fixtures "$fixture_root" --data-root "$staging/data"
run_offline_logged plan /dev/null "$binary" plan --config "$three_config"
run_offline_logged verify /dev/null "$binary" verify --config "$three_config"
run_offline_logged replay "$staging/replay-a-open.log" "$binary" replay --config "$three_config"
run_offline_logged backtest-a "$staging/stream-open-a.log" "$binary" backtest \
    --config "$three_config" --output "$staging/runs/run-a"
run_offline_logged inspect-a /dev/null "$binary" inspect --run "$staging/runs/run-a"

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

stream_count=$(wc -l <"$staging/stream-open-a.log" | tr -d ' ')
[ "$stream_count" -eq 3 ] || { echo "expected three opened streams, got $stream_count" >&2; exit 1; }
for symbol in CAFH6 CDFH6 TXFH6; do
    rg -q "symbol=$symbol " "$staging/stream-open-a.log" || {
        echo "stream-open audit is missing $symbol" >&2
        exit 1
    }
done
sort "$staging/stream-open-a.log" >"$staging/test-results/stream-open-audit.log"

for run_number in 01 02 03 04 05 06 07 08 09 10; do
    run_offline_logged "repeat-$run_number" "$staging/stream-open-$run_number.log" \
        "$binary" backtest --config "$three_config" \
        --output "$staging/runs/repeat-$run_number"
    compare_dirs "$staging/runs/run-a" "$staging/runs/repeat-$run_number"
done
echo "repeated_runs=10 byte_identical=true" >"$staging/test-results/repeated-runs.log"

for permutation in 1 2 3; do
    permutation_config=$staging/m3-permutation-$permutation.yaml
    ruby -ryaml -e '
      config = YAML.load_file(ARGV[0])
      instruments = config.fetch("universe").fetch("instruments")
      config["universe"]["instruments"] = case ARGV[2].to_i
        when 1 then instruments.reverse
        when 2 then instruments.rotate(1)
        else instruments.rotate(2)
      end
      File.write(ARGV[1], config.to_yaml)
    ' "$three_config" "$permutation_config" "$permutation"
    run_offline_logged "permutation-$permutation" "/dev/null" "$binary" backtest \
        --config "$permutation_config" --output "$staging/runs/permutation-$permutation"
    compare_dirs "$staging/runs/run-a" "$staging/runs/permutation-$permutation"
done
echo "discovery_permutations=3 byte_identical=true" >"$staging/test-results/discovery-permutations.log"

cp -R "$staging/data" "$staging/rebuild-data"
rm -rf "$staging/rebuild-data/cache"
rebuild_config=$staging/m3-rebuild.yaml
sed "s#${staging}/data#${staging}/rebuild-data#g" "$three_config" >"$rebuild_config"
run_offline_logged cache-rebuild /dev/null "$binary" cache prepare --config "$rebuild_config"
run_offline_logged rebuild-backtest /dev/null "$binary" backtest --config "$rebuild_config" \
    --output "$staging/runs/rebuild"
compare_dirs "$staging/runs/run-a" "$staging/runs/rebuild"
echo "cache_rebuild=byte_identical=true" >"$staging/test-results/cache-rebuild.log"

run_offline_logged debug-backtest /dev/null "$root/target/debug/osmium" backtest \
    --config "$three_config" --output "$staging/runs/debug"
compare_dirs "$staging/runs/run-a" "$staging/runs/debug"
echo "debug_release=byte_identical=true" >"$staging/test-results/debug-release.log"

cp -R "$staging/runs/run-a" "$staging/runs/corrupt"
printf '\001' >>"$staging/runs/corrupt/ledger.bin"
if run_offline_logged corruption-rejected /dev/null "$binary" inspect --run "$staging/runs/corrupt"; then
    echo "inspect accepted a corrupted attachment" >&2
    exit 1
fi
echo "ledger.bin mutation rejected=true" >"$staging/test-results/corruption.log"

start_seconds=$(date +%s)
run_offline_logged performance /dev/null "$binary" backtest --config "$three_config" \
    --output "$staging/runs/performance"
end_seconds=$(date +%s)
event_count=$(awk '/^events:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
source_bytes=$(du -sk "$staging/data/source" | awk '{print $1 * 1024}')
cache_bytes=$(du -sk "$staging/data/cache" | awk '{print $1 * 1024}')
printf 'dataset=taifex-three\nevents=%s\nsource_bytes=%s\ncache_bytes=%s\nelapsed_seconds=%s\n' \
    "$event_count" "$source_bytes" "$cache_bytes" "$((end_seconds - start_seconds))" \
    >"$staging/test-results/performance.yaml"

full_config=$staging/m3-taifex-multi.yaml
sed "s#target/m3-data#$staging/full-data#g" config/m3-taifex-multi.yaml >"$full_config"
run_offline_logged full-plan /dev/null "$binary" plan --config "$full_config"
run_logged full-fixture-data "$fixture_builder" \
    --config "$full_config" --fixtures "$fixture_root" --data-root "$staging/full-data"
run_offline_logged full-verify /dev/null "$binary" verify --config "$full_config"
run_offline_logged full-replay "$staging/full-replay.log" "$binary" replay --config "$full_config"
run_offline_logged full-backtest "$staging/full-stream-open-a.log" "$binary" backtest \
    --config "$full_config" --output "$staging/runs/full-run-a"
run_offline_logged full-inspect /dev/null "$binary" inspect --run "$staging/runs/full-run-a"

full_stream_count=$(wc -l <"$staging/full-stream-open-a.log" | tr -d ' ')
[ "$full_stream_count" -eq 4 ] || {
    echo "expected four opened streams, got $full_stream_count" >&2
    exit 1
}
for symbol in 2330 CAFH6 CDFH6 TXFH6; do
    rg -q "symbol=$symbol " "$staging/full-stream-open-a.log" || {
        echo "full stream-open audit is missing $symbol" >&2
        exit 1
    }
done
sort "$staging/full-stream-open-a.log" >"$staging/test-results/full-stream-open-audit.log"
cat >"$staging/test-results/four-instrument-gate.yaml" <<EOF
status: passed
fixture: fixtures/teralion/twse/2330/2026-07-20
source_cache_lineage: verified
opened_streams: 4
synthetic_substitution: forbidden
EOF

for run_number in 01 02 03 04 05 06 07 08 09 10; do
    run_offline_logged "full-repeat-$run_number" "$staging/full-stream-open-$run_number.log" \
        "$binary" backtest --config "$full_config" \
        --output "$staging/runs/full-repeat-$run_number"
    compare_dirs "$staging/runs/full-run-a" "$staging/runs/full-repeat-$run_number"
done
echo "repeated_runs=10 byte_identical=true" >"$staging/test-results/full-repeated-runs.log"

for permutation in 1 2 3; do
    permutation_config=$staging/m3-full-permutation-$permutation.yaml
    ruby -ryaml -e '
      config = YAML.load_file(ARGV[0])
      instruments = config.fetch("universe").fetch("instruments")
      config["universe"]["instruments"] = case ARGV[2].to_i
        when 1 then instruments.reverse
        when 2 then instruments.rotate(1)
        else instruments.rotate(2)
      end
      File.write(ARGV[1], config.to_yaml)
    ' "$full_config" "$permutation_config" "$permutation"
    run_offline_logged "full-permutation-$permutation" "/dev/null" "$binary" backtest \
        --config "$permutation_config" --output "$staging/runs/full-permutation-$permutation"
    compare_dirs "$staging/runs/full-run-a" "$staging/runs/full-permutation-$permutation"
done
echo "discovery_permutations=3 byte_identical=true" >"$staging/test-results/full-discovery-permutations.log"

cp -R "$staging/full-data" "$staging/full-rebuild-data"
rm -rf "$staging/full-rebuild-data/cache"
full_rebuild_config=$staging/m3-full-rebuild.yaml
sed "s#${staging}/full-data#${staging}/full-rebuild-data#g" "$full_config" >"$full_rebuild_config"
run_offline_logged full-cache-rebuild /dev/null "$binary" cache prepare \
    --config "$full_rebuild_config"
run_offline_logged full-rebuild-backtest /dev/null "$binary" backtest \
    --config "$full_rebuild_config" --output "$staging/runs/full-rebuild"
compare_dirs "$staging/runs/full-run-a" "$staging/runs/full-rebuild"
echo "cache_rebuild=byte_identical=true" >"$staging/test-results/full-cache-rebuild.log"

run_offline_logged full-debug-backtest /dev/null "$root/target/debug/osmium" backtest \
    --config "$full_config" --output "$staging/runs/full-debug"
compare_dirs "$staging/runs/full-run-a" "$staging/runs/full-debug"
echo "debug_release=byte_identical=true" >"$staging/test-results/full-debug-release.log"

cp -R "$staging/runs/full-run-a" "$staging/runs/full-corrupt"
printf '\001' >>"$staging/runs/full-corrupt/ledger.bin"
if run_offline_logged full-corruption-rejected /dev/null "$binary" inspect \
    --run "$staging/runs/full-corrupt"; then
    echo "full inspect accepted a corrupted attachment" >&2
    exit 1
fi
echo "ledger.bin mutation rejected=true" >"$staging/test-results/full-corruption.log"

full_start_seconds=$(date +%s)
run_offline_logged full-performance /dev/null "$binary" backtest --config "$full_config" \
    --output "$staging/runs/full-performance"
full_end_seconds=$(date +%s)
full_event_count=$(awk '/^events:/ {print $2; exit}' "$staging/runs/full-run-a/run-summary.yaml")
full_source_bytes=$(du -sk "$staging/full-data/source" | awk '{print $1 * 1024}')
full_cache_bytes=$(du -sk "$staging/full-data/cache" | awk '{print $1 * 1024}')
cat >>"$staging/test-results/performance.yaml" <<EOF
four_instrument:
  events: $full_event_count
  source_bytes: $full_source_bytes
  cache_bytes: $full_cache_bytes
  elapsed_seconds: $((full_end_seconds - full_start_seconds))
EOF

git_commit=$(git rev-parse HEAD)
rust_toolchain=$(rustc --version)
event_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/event-stream.blake3")
state_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/final-state.blake3")
strategy_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/strategy-output.blake3")
orders_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/orders.blake3")
fills_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/fills.blake3")
ledger_checksum=$(tr -d '[:space:]' <"$staging/runs/run-a/ledger.blake3")
cat >"$staging/test-results/artifact-checksums.yaml" <<EOF
event_stream_blake3: $event_checksum
final_state_blake3: $state_checksum
strategy_output_blake3: $strategy_checksum
orders_blake3: $orders_checksum
fills_blake3: $fills_checksum
ledger_blake3: $ledger_checksum
events: $event_count
orders: $(awk '/^orders:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
fills: $(awk '/^fills:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
final_cash_atoms: $(awk '/^final_cash_atoms:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
realized_pnl_atoms: $(awk '/^realized_pnl_atoms:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
unrealized_pnl_atoms: $(awk '/^unrealized_pnl_atoms:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
EOF
full_event_checksum=$(tr -d '[:space:]' <"$staging/runs/full-run-a/event-stream.blake3")
full_state_checksum=$(tr -d '[:space:]' <"$staging/runs/full-run-a/final-state.blake3")
full_strategy_checksum=$(tr -d '[:space:]' <"$staging/runs/full-run-a/strategy-output.blake3")
full_orders_checksum=$(tr -d '[:space:]' <"$staging/runs/full-run-a/orders.blake3")
full_fills_checksum=$(tr -d '[:space:]' <"$staging/runs/full-run-a/fills.blake3")
full_ledger_checksum=$(tr -d '[:space:]' <"$staging/runs/full-run-a/ledger.blake3")
cat >"$staging/test-results/full-artifact-checksums.yaml" <<EOF
event_stream_blake3: $full_event_checksum
final_state_blake3: $full_state_checksum
strategy_output_blake3: $full_strategy_checksum
orders_blake3: $full_orders_checksum
fills_blake3: $full_fills_checksum
ledger_blake3: $full_ledger_checksum
events: $full_event_count
orders: $(awk '/^orders:/ {print $2; exit}' "$staging/runs/full-run-a/run-summary.yaml")
fills: $(awk '/^fills:/ {print $2; exit}' "$staging/runs/full-run-a/run-summary.yaml")
final_cash_atoms: $(awk '/^final_cash_atoms:/ {print $2; exit}' "$staging/runs/full-run-a/run-summary.yaml")
realized_pnl_atoms: $(awk '/^realized_pnl_atoms:/ {print $2; exit}' "$staging/runs/full-run-a/run-summary.yaml")
unrealized_pnl_atoms: $(awk '/^unrealized_pnl_atoms:/ {print $2; exit}' "$staging/runs/full-run-a/run-summary.yaml")
EOF
cat >"$staging/acceptance-report.yaml" <<EOF
acceptance_contract_version: 1
verification_plan_version: 3
milestone: M3
status: Passed
completion_quality: full
git_commit: $git_commit
rust_toolchain: "$rust_toolchain"
scope:
  taifex_three_instrument: passed
  four_instrument: passed
fixture_gate:
  taifex_selected_fixture: passed
  taifex_daily_instruments: passed
  twse_selected_fixture: passed
  twse_daily_instrument: passed
  secret_scan: passed
network_policy:
  status: disabled
  runner: $network_runner
  credential: absent
three_instrument_run:
  config: config/m3-taifex-three.yaml
  events: $event_count
  opened_streams: $stream_count
  orders: $(awk '/^orders:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
  fills: $(awk '/^fills:/ {print $2; exit}' "$staging/runs/run-a/run-summary.yaml")
  event_stream_blake3: $event_checksum
  final_state_blake3: $state_checksum
  strategy_output_blake3: $strategy_checksum
  orders_blake3: $orders_checksum
  fills_blake3: $fills_checksum
  ledger_blake3: $ledger_checksum
four_instrument_run:
  config: config/m3-taifex-multi.yaml
  events: $full_event_count
  opened_streams: $full_stream_count
  orders: $(awk '/^orders:/ {print $2; exit}' "$staging/runs/full-run-a/run-summary.yaml")
  fills: $(awk '/^fills:/ {print $2; exit}' "$staging/runs/full-run-a/run-summary.yaml")
  event_stream_blake3: $full_event_checksum
  final_state_blake3: $full_state_checksum
  strategy_output_blake3: $full_strategy_checksum
  orders_blake3: $full_orders_checksum
  fills_blake3: $full_fills_checksum
  ledger_blake3: $full_ledger_checksum
determinism:
  three_instrument:
    repeated_runs: 10
    repeated_run_directories_byte_identical: true
    discovery_permutations: 3
    discovery_permutations_byte_identical: true
    cache_rebuild_byte_identical: true
    debug_release_byte_identical: true
  four_instrument:
    repeated_runs: 10
    repeated_run_directories_byte_identical: true
    discovery_permutations: 3
    discovery_permutations_byte_identical: true
    cache_rebuild_byte_identical: true
    debug_release_byte_identical: true
inspection:
  three_instrument_inspect_success: passed
  three_instrument_corruption_rejection: passed
  four_instrument_inspect_success: passed
  four_instrument_corruption_rejection: passed
performance:
  evidence: test-results/performance.yaml
  threshold: not_set
acceptance:
  M3-AC-01: { status: Passed, evidence: test-results/fixture-integrity.log + fixture metadata }
  M3-AC-02: { status: Passed, evidence: taifex-fixture-tests.log }
  M3-AC-03: { status: Passed, evidence: workspace-debug.log + fixture-data.log }
  M3-AC-04: { status: Passed, evidence: taifex-fixture-tests.log + cache descriptors }
  M3-AC-05: { status: Passed, evidence: taifex-fixture-tests.log }
  M3-AC-06: { status: Passed, evidence: workspace-debug.log }
  M3-AC-07: { status: Passed, evidence: test-results/full-stream-open-audit.log + full-replay.log }
  M3-AC-08: { status: Passed, evidence: test-results/stream-open-audit.log + test-results/full-stream-open-audit.log }
  M3-AC-09: { status: Passed, evidence: workspace-debug.log }
  M3-AC-10: { status: Passed, evidence: test-results/artifact-checksums.yaml + test-results/full-artifact-checksums.yaml }
  M3-AC-11: { status: Passed, evidence: test-results/artifact-checksums.yaml + test-results/full-artifact-checksums.yaml }
  M3-AC-12: { status: Passed, evidence: test-results/full-artifact-checksums.yaml + config/m3-taifex-multi.yaml }
  M3-AC-13: { status: Passed, evidence: test-results/full-artifact-checksums.yaml + test-results/performance.yaml }
  M3-AC-14: { status: Passed, evidence: full-plan.log + full-verify.log + full-replay.log + full-inspect.log }
  M3-AC-15: { status: Passed, evidence: test-results/full-repeated-runs.log + test-results/full-discovery-permutations.log }
  M3-AC-16: { status: Passed, evidence: test-results/full-corruption.log }
  M3-AC-17: { status: Passed, evidence: test-results/performance.yaml + test-results/full-stream-open-audit.log }
approver: tools/run_m3_acceptance.sh
EOF

# Source/cache and repeated run directories are derived and intentionally not committed.
# The report, test logs and canonical artifact checksums are the durable evidence.
rm -rf "$staging/data" "$staging/rebuild-data" "$staging/runs"

mv "$staging" "$output_path"
trap - EXIT HUP INT TERM
echo "M3 acceptance completed: four-instrument scope passed"
echo "output=$output_path"
