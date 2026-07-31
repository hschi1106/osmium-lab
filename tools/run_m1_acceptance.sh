#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: tools/run_m1_acceptance.sh --output <directory> [--network-runner auto|sandbox-exec|docker]

The output directory must not exist. The harness builds and tests with normal
host access, then runs the M1 replay binary with networking denied.
EOF
}

output=
network_runner=auto
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
        --network-runner)
            [ "$#" -ge 2 ] || {
                usage >&2
                exit 2
            }
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

[ -n "$output" ] || {
    usage >&2
    exit 2
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
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

case "$network_runner" in
    auto)
        case "$(uname -s)" in
            Darwin) network_runner=sandbox-exec ;;
            Linux) network_runner=docker ;;
            *)
                echo "no automatic network-denial runner for $(uname -s)" >&2
                exit 2
                ;;
        esac
        ;;
    sandbox-exec|docker) ;;
    *)
        echo "unsupported network runner: $network_runner" >&2
        exit 2
        ;;
esac

parent=$(dirname -- "$output_path")
base=$(basename -- "$output_path")
mkdir -p "$parent"
staging=$parent/.$base.staging.$$
repeat=$parent/.$base.repeat.$$
cleanup() {
    rm -rf "$repeat"
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
binary=$root/target/release/osmium
policy='(version 1)(allow default)(deny network*)'
container_image=ubuntu:24.04

run_logged fmt cargo fmt --check
run_logged clippy cargo clippy --workspace --all-targets -- -D warnings
run_logged workspace-debug cargo test --workspace
run_logged workspace-release cargo test --workspace --release
run_logged release-build cargo build --release -p osmium-cli

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

run_network_disabled() {
    destination=$1
    log=$2
    case "$network_runner" in
        sandbox-exec)
            command -v sandbox-exec >/dev/null 2>&1 || {
                echo "sandbox-exec is required" >&2
                exit 2
            }
            env -u TERALION_API_KEY sandbox-exec -p "$policy" \
                "$binary" replay \
                --fixture "$root/$fixture" \
                --output "$destination" >"$log" 2>&1
            ;;
        docker)
            command -v docker >/dev/null 2>&1 || {
                echo "docker is required" >&2
                exit 2
            }
            if ! docker image inspect "$container_image" >/dev/null 2>&1; then
                docker pull "$container_image" \
                    >"$staging/test-results/container-image.log" 2>&1
            fi
            docker run --rm --network none --cap-drop ALL \
                --security-opt no-new-privileges \
                --user "$(id -u):$(id -g)" \
                --volume "$root:$root" \
                --workdir "$root" \
                "$container_image" \
                "$binary" replay \
                --fixture "$root/$fixture" \
                --output "$destination" >"$log" 2>&1
            ;;
    esac
}

echo "running network-disabled replay"
run_network_disabled "$staging/artifacts" \
    "$staging/test-results/network-disabled-replay.log"
run_network_disabled "$repeat" \
    "$staging/test-results/network-disabled-repeat.log"

artifacts='
event-stream.blake3
final-state.blake3
fixture-metadata.yaml
fixture-set.sha256
normalized-events.bin
run-summary.yaml
strategy-output.bin
strategy-output.blake3
warnings.yaml
'
for artifact in $artifacts; do
    cmp "$staging/artifacts/$artifact" "$repeat/$artifact"
done
echo "all replay artifacts are byte-identical" \
    >"$staging/test-results/artifact-comparison.log"

for artifact in $artifacts; do
    mv "$staging/artifacts/$artifact" "$staging/$artifact"
done
rmdir "$staging/artifacts"
rm -rf "$repeat"

git_commit=$(git rev-parse HEAD)
rust_toolchain=$(rustc --version)
host=$(uname -sm)
approved_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
normalized_checksum=$(
    awk '/normalized_events_blake3:/ { print $2 }' "$staging/run-summary.yaml"
)

cat >"$staging/acceptance-report.yaml" <<EOF
acceptance_contract_version: 1
verification_plan_version: 2
status: Passed
git_commit: $git_commit
fixture_identity: teralion/twse/2330/2026-07-27/regular
fixture_checksum:
  algorithm: sha256
  value: $actual_fixture_checksum
strategy_identity:
  id: example.twse-post-state-observer
  version: "1"
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
network_policy:
  status: disabled
  runner: $network_runner
  policy: deny network
credential_policy:
  TERALION_API_KEY: absent
tests:
EOF

for id in \
    001 002 003 004 005 006 007 008 009 \
    010 011 012 013 014 015 016 017 \
    020 021 022 023 024 025 \
    030 031 032 033 034 035 036 \
    040 041 042 043 044 045 046 047 048 049
do
    printf '  M1-T%s: { status: Passed, evidence: test-results/workspace-debug.log + test-results/workspace-release.log }\n' \
        "$id" >>"$staging/acceptance-report.yaml"
done
cat >>"$staging/acceptance-report.yaml" <<EOF
  M1-T050: { status: Passed, evidence: run-summary.yaml }
  M1-T051: { status: Passed, evidence: test-results/artifact-comparison.log }
  M1-T052: { status: Passed, evidence: test-results/network-disabled-replay.log }
  M1-T053: { status: Passed, evidence: test-results/network-disabled-replay.log }
  M1-T054: { status: Passed, evidence: run-summary.yaml + warnings.yaml }
artifact_checksums:
  fixture_set_sha256: $actual_fixture_checksum
  normalized_events_blake3: $normalized_checksum
  event_stream_blake3: $(tr -d '[:space:]' <"$staging/event-stream.blake3")
  final_state_blake3: $(tr -d '[:space:]' <"$staging/final-state.blake3")
  strategy_output_blake3: $(tr -d '[:space:]' <"$staging/strategy-output.blake3")
approver: tools/run_m1_acceptance.sh
approved_at: "$approved_at"
EOF

mv "$staging" "$output_path"
trap - EXIT HUP INT TERM
echo "M1 acceptance passed"
echo "output=$output_path"
