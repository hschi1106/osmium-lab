#!/usr/bin/env bash
set -euo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)"
cd "$repo_root"

usage() {
  echo "usage: $0 focused <market|provider|replay|strategy|execution|runner|config> | full" >&2
  exit 2
}

mode="${1:-}"

case "$mode" in
  focused)
    area="${2:-}"
    [[ $# -eq 2 ]] || usage
    case "$area" in
      market)
        cargo test --locked -p market-types -p market-state
        ;;
      provider)
        cargo test --locked -p data-sync -p teralion-provider
        ;;
      replay)
        cargo test --locked -p replay-engine
        ;;
      strategy)
        cargo test --locked -p strategy-api
        ;;
      execution)
        cargo test --locked -p execution-sim
        ;;
      runner)
        cargo test --locked -p osmium-runner
        ;;
      config)
        cargo test --locked -p osmium-config -p run-planner -p osmium-cli
        ;;
      *)
        usage
        ;;
    esac
    ;;
  full)
    [[ $# -eq 1 ]] || usage
    cargo fmt --all --check
    cargo test --workspace --locked
    cargo clippy --workspace --all-targets --locked -- -D warnings
    python3 tools/acceptance/generate_synthetic_fixtures.py
    git diff --exit-code -- fixtures
    tools/acceptance/verify_compact_fixtures.sh
    ;;
  *)
    usage
    ;;
esac
