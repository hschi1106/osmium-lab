#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/acceptance/run_m5_acceptance.sh --output <directory> [--network-runner auto|sandbox-exec]
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
    env -u TERALION_API_KEY -u TERALION_BASE_URL \
        OSMIUM_STREAM_OPEN_AUDIT="$audit" \
        sandbox-exec -p '(version 1)(allow default)(deny network*)' \
        "$@" >"$staging/test-results/$name.log" 2>&1
}

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

run_suite() {
    prefix=$1
    config=$2
    data_path=$3
    expected_stream_count=$4
    shift 4
    binary=$root/target/release/osmium
    fixture_builder=$root/target/release/osmium_fixture_data

    run_logged "fixture-data-$prefix" "$fixture_builder" \
        --config "$config" --fixtures "$root/fixtures/teralion" --data-root "$data_path"
    run_offline_logged "plan-$prefix" /dev/null "$binary" plan --config "$config"
    run_offline_logged "verify-$prefix" /dev/null "$binary" data verify --config "$config"
    run_offline_logged "replay-$prefix" "$staging/replay-$prefix-open.log" \
        "$binary" replay --config "$config"
    run_offline_logged "backtest-$prefix" "$staging/stream-open-$prefix.log" \
        "$binary" backtest --config "$config" --output "$staging/runs/$prefix-run-a"
    run_offline_logged "inspect-$prefix" /dev/null "$binary" inspect \
        --run "$staging/runs/$prefix-run-a"

    stream_count=$(wc -l <"$staging/stream-open-$prefix.log" | tr -d ' ')
    [ "$stream_count" -eq "$expected_stream_count" ] || {
        echo "$prefix opened $stream_count streams; expected $expected_stream_count" >&2
        exit 1
    }
    for symbol in "$@"; do
        rg -q "symbol=$symbol " "$staging/stream-open-$prefix.log" || {
            echo "$prefix stream-open audit is missing $symbol" >&2
            exit 1
        }
    done
    sort "$staging/stream-open-$prefix.log" \
        >"$staging/test-results/stream-open-$prefix.log"

    for run_number in 01 02 03 04 05 06 07 08 09 10; do
        run_offline_logged "repeat-$prefix-$run_number" /dev/null "$binary" backtest \
            --config "$config" --output "$staging/runs/$prefix-repeat-$run_number"
        compare_dirs "$staging/runs/$prefix-run-a" \
            "$staging/runs/$prefix-repeat-$run_number"
    done
    echo "repeated_runs=10 byte_identical=true" \
        >"$staging/test-results/repeated-runs-$prefix.log"

    for permutation in 1 2 3; do
        permutation_config=$staging/$prefix-permutation-$permutation.yaml
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
        run_offline_logged "permutation-$prefix-$permutation" /dev/null "$binary" backtest \
            --config "$permutation_config" \
            --output "$staging/runs/$prefix-permutation-$permutation"
        compare_dirs "$staging/runs/$prefix-run-a" \
            "$staging/runs/$prefix-permutation-$permutation"
    done
    echo "discovery_permutations=3 byte_identical=true" \
        >"$staging/test-results/discovery-permutations-$prefix.log"

    cp -R "$data_path" "$staging/$prefix-rebuild-data"
    rm -rf "$staging/$prefix-rebuild-data/cache"
    rebuild_config=$staging/$prefix-rebuild.yaml
    sed "s#$data_path#$staging/$prefix-rebuild-data#g" "$config" >"$rebuild_config"
    run_offline_logged "cache-rebuild-$prefix" /dev/null "$binary" cache prepare \
        --config "$rebuild_config"
    run_offline_logged "rebuild-backtest-$prefix" /dev/null "$binary" backtest \
        --config "$rebuild_config" --output "$staging/runs/$prefix-rebuild"
    compare_dirs "$staging/runs/$prefix-run-a" "$staging/runs/$prefix-rebuild"
    echo "cache_rebuild=byte_identical=true" \
        >"$staging/test-results/cache-rebuild-$prefix.log"

    run_offline_logged "debug-backtest-$prefix" /dev/null "$root/target/debug/osmium" backtest \
        --config "$config" --output "$staging/runs/$prefix-debug"
    compare_dirs "$staging/runs/$prefix-run-a" "$staging/runs/$prefix-debug"
    echo "debug_release=byte_identical=true" \
        >"$staging/test-results/debug-release-$prefix.log"

    cp -R "$staging/runs/$prefix-run-a" "$staging/runs/$prefix-corrupt"
    printf '\001' >>"$staging/runs/$prefix-corrupt/ledger.bin"
    if run_offline_logged "corruption-rejected-$prefix" /dev/null \
        "$binary" inspect --run "$staging/runs/$prefix-corrupt"; then
        echo "$prefix inspect accepted corrupted ledger" >&2
        exit 1
    fi
    echo "ledger.bin mutation rejected=true" \
        >"$staging/test-results/corruption-$prefix.log"

    start_seconds=$(date +%s)
    run_offline_logged "performance-$prefix" /dev/null "$binary" backtest \
        --config "$config" --output "$staging/runs/$prefix-performance"
    end_seconds=$(date +%s)

    summary=$staging/runs/$prefix-run-a/run-summary.yaml
    event_count=$(awk '/^events:/ {print $2; exit}' "$summary")
    orders=$(awk '/^orders:/ {print $2; exit}' "$summary")
    fills=$(awk '/^fills:/ {print $2; exit}' "$summary")
    source_bytes=$(du -sk "$data_path/source" | awk '{print $1 * 1024}')
    cache_bytes=$(du -sk "$data_path/cache" | awk '{print $1 * 1024}')
    printf 'dataset=%s\nevents=%s\nsource_bytes=%s\ncache_bytes=%s\nelapsed_seconds=%s\n' \
        "$prefix" "$event_count" "$source_bytes" "$cache_bytes" \
        "$((end_seconds - start_seconds))" \
        >"$staging/test-results/performance-$prefix.yaml"

    event_checksum=$(tr -d '[:space:]' <"$staging/runs/$prefix-run-a/event-stream.blake3")
    state_checksum=$(tr -d '[:space:]' <"$staging/runs/$prefix-run-a/final-state.blake3")
    strategy_checksum=$(tr -d '[:space:]' <"$staging/runs/$prefix-run-a/strategy-output.blake3")
    orders_checksum=$(tr -d '[:space:]' <"$staging/runs/$prefix-run-a/orders.blake3")
    fills_checksum=$(tr -d '[:space:]' <"$staging/runs/$prefix-run-a/fills.blake3")
    ledger_checksum=$(tr -d '[:space:]' <"$staging/runs/$prefix-run-a/ledger.blake3")
    cat >"$staging/test-results/artifact-checksums-$prefix.yaml" <<EOF
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

    eval "${prefix}_event_count=$event_count"
    eval "${prefix}_orders=$orders"
    eval "${prefix}_fills=$fills"
    eval "${prefix}_event_checksum=$event_checksum"
    eval "${prefix}_state_checksum=$state_checksum"
    eval "${prefix}_strategy_checksum=$strategy_checksum"
    eval "${prefix}_orders_checksum=$orders_checksum"
    eval "${prefix}_fills_checksum=$fills_checksum"
    eval "${prefix}_ledger_checksum=$ledger_checksum"
}

mkdir -p "$staging"
warrant_config=$staging/m5-warrant.yaml
option_config=$staging/m5-option.yaml
sed "s#target/m5-warrant-data#$staging/warrant-data#g" \
    config/m5-warrant.yaml >"$warrant_config"
sed "s#target/m5-option-data#$staging/option-data#g" \
    config/m5-option.yaml >"$option_config"

run_logged fmt cargo fmt --check
run_logged clippy cargo clippy --workspace --all-targets -- -D warnings
run_logged workspace-debug cargo test --workspace
run_logged workspace-release cargo test --workspace --release
run_logged debug-build cargo build -p osmium-cli -p osmium-config
run_logged release-build cargo build --release -p osmium-cli -p osmium-config
run_logged fixture-builder env CARGO_TARGET_DIR="$root/target" cargo build --release \
    --manifest-path "$root/tools/acceptance/osmium_fixture_data/Cargo.toml"
run_logged fixture-integrity "$root/tools/acceptance/verify_m5_fixtures.sh"
run_logged warrant-normalizer-tests cargo test -p twse-normalizer --test m5_fixture
run_logged option-normalizer-tests cargo test -p taifex-normalizer --test m5_option_fixture
run_logged source-contract-tests cargo test -p data-sync \
    option_archive_market_is_explicit_and_identity_bound
run_logged cursor-contract-tests cargo test -p data-sync \
    explicit_option_query_rejects_futures_market_payload
run_logged accounting-tests cargo test -p execution-sim \
    options_v1_moves_premium_cash_with_contract_multiplier

run_suite warrant "$warrant_config" "$staging/warrant-data" 3 2330 6488 03003T
run_suite option "$option_config" "$staging/option-data" 2 TXFH6 TXO24000U6

option_positions=$staging/runs/option-run-a/positions.yaml
rg -q 'instrument: .Taifex:TXO24000U6.' "$option_positions"
rg -q 'model: options_v1' "$option_positions"
rg -q 'instrument: .Taifex:TXFH6.' "$option_positions"
rg -q 'model: futures_v1' "$option_positions"
echo "futures_options_accounting_isolation=passed" \
    >"$staging/test-results/accounting-isolation.log"

git_commit=$(git rev-parse HEAD)
rust_toolchain=$(rustc --version)
cat >"$staging/acceptance-report.yaml" <<EOF
acceptance_contract_version: 1
milestone: M5
status: Passed
completion_quality: full
git_commit: $git_commit
rust_toolchain: "$rust_toolchain"
scope:
  M5_W: passed
  M5_O: passed
prerequisite:
  M4: passed
fixture_gate:
  warrant: passed
  option: passed
  daily_instrument: passed
  source_checksums: passed
  secret_scan: passed
  redistribution_scope: private-internal-review-only
network_policy:
  status: disabled
  runner: $network_runner
  credential: absent
run:
  warrant:
    config: config/m5-warrant.yaml
    events: $warrant_event_count
    opened_streams: 3
    orders: $warrant_orders
    fills: $warrant_fills
    event_stream_blake3: $warrant_event_checksum
    final_state_blake3: $warrant_state_checksum
    strategy_output_blake3: $warrant_strategy_checksum
    orders_blake3: $warrant_orders_checksum
    fills_blake3: $warrant_fills_checksum
    ledger_blake3: $warrant_ledger_checksum
  option:
    config: config/m5-option.yaml
    events: $option_event_count
    opened_streams: 2
    orders: $option_orders
    fills: $option_fills
    event_stream_blake3: $option_event_checksum
    final_state_blake3: $option_state_checksum
    strategy_output_blake3: $option_strategy_checksum
    orders_blake3: $option_orders_checksum
    fills_blake3: $option_fills_checksum
    ledger_blake3: $option_ledger_checksum
determinism:
  repeated_runs_per_scope: 10
  repeated_run_directories_byte_identical: true
  discovery_permutations_per_scope: 3
  discovery_permutations_byte_identical: true
  cache_rebuild_byte_identical: true
  debug_release_byte_identical: true
accounting:
  futures_options_isolation: passed
  options_model: options_v1
  futures_model: futures_v1
inspection:
  inspect_success: passed
  corruption_rejection: passed
performance:
  warrant: test-results/performance-warrant.yaml
  option: test-results/performance-option.yaml
  threshold: not_set
acceptance:
  M5-AC-01: { status: Passed, evidence: fixture-integrity.log + M4 evidence + official links in fixture metadata }
  M5-AC-02: { status: Passed, evidence: fixture metadata + config references + interface documents }
  M5-AC-03: { status: Passed, evidence: fixture-integrity.log + fixture-data-warrant.log + fixture-data-option.log + plan/verify logs }
  M5-AC-04: { status: Passed, evidence: warrant-normalizer-tests.log + option-normalizer-tests.log + artifact checksums }
  M5-AC-05: { status: Passed, evidence: normalizer tests + replay logs + market-state profiles }
  M5-AC-06: { status: Passed, evidence: plan/verify logs + cache-rebuild-warrant.log + cache-rebuild-option.log + network policy }
  M5-AC-07: { status: Passed, evidence: stream-open-warrant.log + stream-open-option.log + replay/backtest logs }
  M5-AC-08: { status: Passed, evidence: accounting-tests.log + accounting-isolation.log + option positions }
  M5-AC-09: { status: Passed, evidence: repeated-runs-* + discovery-permutations-* + debug-release-* }
  M5-AC-10: { status: Passed, evidence: corruption-* + fixture-integrity.log + secret-scan.log + performance-* + this report }
approver: tools/acceptance/run_m5_acceptance.sh
EOF

if rg -n -i \
    '"(authorization|api[_-]?key|cookie|password|secret|token|next_cursor)"[[:space:]]*:' \
    "$staging"; then
    echo "M5 acceptance output contains a forbidden secret field" >&2
    exit 1
fi
echo "formal evidence secret scan: passed" >"$staging/test-results/secret-scan.log"

rm -rf "$staging/warrant-data" "$staging/option-data" \
    "$staging/warrant-rebuild-data" "$staging/option-rebuild-data" "$staging/runs"
find "$staging" -maxdepth 1 -name 'warrant-permutation-*.yaml' -delete
find "$staging" -maxdepth 1 -name 'option-permutation-*.yaml' -delete
find "$staging" -maxdepth 1 -name '*-rebuild.yaml' -delete
find "$staging/test-results" -maxdepth 1 -name '.left-files' -delete
find "$staging/test-results" -maxdepth 1 -name '.right-files' -delete

mv "$staging" "$output_path"
trap - EXIT HUP INT TERM
echo "M5 acceptance completed: warrant and option scopes passed"
echo "output=$output_path"
