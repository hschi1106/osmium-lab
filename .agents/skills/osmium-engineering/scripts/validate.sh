#!/usr/bin/env bash
set -euo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)"
cd "$repo_root"

usage() {
  echo "usage: $0 focused <market|md|replay|strategy|execution|runner|config> | full" >&2
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
      md)
        cargo test --locked -p osmium-types -p osmium-md
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
        cargo test --locked -p run-planner
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
    ;;
  *)
    usage
    ;;
esac
