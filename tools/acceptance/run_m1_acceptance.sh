#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: tools/acceptance/run_m1_acceptance.sh --output <directory>

The harness validates the historical TWSE fixture with the standalone
acceptance runner. It does not use the release CLI or network.
EOF
}

output=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output)
            [ "$#" -ge 2 ] || {
                usage >&2
                exit 2
            }
            output=$2
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

[ -n "$output" ] || {
    usage >&2
    exit 2
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

case "$output" in
    /*) output_path=$output ;;
    *) output_path=$root/$output ;;
esac

case "$output_path" in
    "$root"/*) ;;
    *)
        echo "acceptance output must be inside the repository" >&2
        exit 2
        ;;
esac

[ ! -e "$output_path" ] || {
    echo "acceptance output already exists: $output_path" >&2
    exit 2
}

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "tracked working tree must be clean before acceptance" >&2
    exit 2
fi

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
mkdir -p "$staging/test-results"

run_logged() {
    name=$1
    shift
    echo "running $name"
    "$@" >"$staging/test-results/$name.log" 2>&1
}

fixture=fixtures/teralion/twse/2330/2026-07-27
runner_manifest=tools/acceptance/osmium_m1_runner/Cargo.toml

run_logged fmt cargo fmt --check
run_logged clippy cargo clippy --workspace --all-targets -- -D warnings
run_logged workspace-debug cargo test --workspace
run_logged workspace-release cargo test --workspace --release
run_logged release-build cargo build --release -p osmium-cli
run_logged acceptance-runner cargo test --manifest-path "$runner_manifest" --locked
run_logged acceptance-runner-release cargo test --manifest-path "$runner_manifest" --locked --release

echo "checking fixture integrity"
expected_fixture_checksum=$(tr -d '[:space:]' <"$fixture/golden/fixture-set.sha256")
if command -v sha256sum >/dev/null 2>&1; then
    actual_fixture_checksum=$(
        for shard in "$fixture"/regular-quotes/*.jsonl; do
            cat "$shard"
        done | sha256sum | awk '{print $1}'
    )
else
    actual_fixture_checksum=$(
        for shard in "$fixture"/regular-quotes/*.jsonl; do
            cat "$shard"
        done | shasum -a 256 | awk '{print $1}'
    )
fi
[ "$actual_fixture_checksum" = "$expected_fixture_checksum" ] || {
    echo "fixture checksum mismatch" >&2
    exit 1
}
printf 'fixture_set_sha256=%s\n' "$actual_fixture_checksum" \
    >"$staging/test-results/fixture-integrity.log"

echo "scanning fixture for forbidden secret fields"
if rg -n -i \
    '"(authorization|api[_-]?key|cookie|password|secret|token)"[[:space:]]*:' \
    "$fixture/regular-quotes" >"$staging/test-results/fixture-secret-scan.log"
then
    echo "fixture secret scan found forbidden fields" >&2
    exit 1
fi
echo "findings=0" >"$staging/test-results/fixture-secret-scan.log"

normalized_checksum=$(tr -d '[:space:]' <"$fixture/golden/normalized-events.blake3")
event_checksum=$(tr -d '[:space:]' <"$fixture/golden/event-stream.blake3")
final_state_checksum=$(tr -d '[:space:]' <"$fixture/golden/final-state.blake3")
strategy_output_checksum=$(tr -d '[:space:]' <"$fixture/golden/strategy-output.blake3")
git_commit=$(git rev-parse HEAD)
rust_toolchain=$(rustc --version)
host=$(uname -sm)
approved_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

cat >"$staging/acceptance-report.yaml" <<EOF
acceptance_contract_version: 1
verification_plan_version: 2
status: Passed
git_commit: $git_commit
fixture_identity: teralion/twse/2330/2026-07-27/regular
fixture_checksum:
  algorithm: sha256
  value: $actual_fixture_checksum
runner:
  manifest: $runner_manifest
  network: not_used
  release_cli_fixture_mode: removed
design_versions:
  mapping: 3
  market_types: 1
  event_schema: 1
  canonical_event: 1
  normalized_event_set: 1
  ordering_rule: 2
  replay_engine: 1
  replay_event_stream: 1
  market_state: 1
  state_reducer: 1
  final_state_set: 1
  strategy_api: 1
  strategy_output: 1
rust_toolchain: "$rust_toolchain"
host: "$host"
build_profiles:
  debug: Passed
  release: Passed
tests:
  acceptance-runner: { status: Passed, evidence: test-results/acceptance-runner.log }
  acceptance-runner-release: { status: Passed, evidence: test-results/acceptance-runner-release.log }
  fixture-integrity: { status: Passed, evidence: test-results/fixture-integrity.log }
  fixture-secret-scan: { status: Passed, evidence: test-results/fixture-secret-scan.log }
artifact_checksums:
  fixture_set_sha256: $actual_fixture_checksum
  normalized_events_blake3: $normalized_checksum
  event_stream_blake3: $event_checksum
  final_state_blake3: $final_state_checksum
  strategy_output_blake3: $strategy_output_checksum
approver: tools/acceptance/run_m1_acceptance.sh
approved_at: "$approved_at"
EOF

mv "$staging" "$output_path"
trap - EXIT HUP INT TERM
echo "historical fixture acceptance passed"
echo "output=$output_path"

