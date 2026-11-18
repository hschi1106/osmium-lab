#!/usr/bin/env bash
set -euo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)"
cd "$repo_root"

usage() {
  echo "usage: $0 focused <types|md|book|replay|matching|ledger|strategy|runner|cli|examples> | full" >&2
  exit 2
}

mode="${1:-}"

case "$mode" in
  focused)
    area="${2:-}"
    [[ $# -eq 2 ]] || usage
    case "$area" in
      types)
        cargo test --locked -p osmium-types
        ;;
      md)
        cargo test --locked -p osmium-types -p osmium-md
        ;;
      book)
        cargo test --locked -p osmium-types -p osmium-book
        ;;
      replay)
        cargo test --locked -p osmium-types -p osmium-replay
        ;;
      matching)
        cargo test --locked -p osmium-types -p osmium-book -p osmium-matching
        ;;
      ledger)
        cargo test --locked -p osmium-types -p osmium-ledger
        ;;
      strategy)
        cargo test --locked -p osmium-strategy -p osmium-examples
        ;;
      runner)
        cargo test --locked -p osmium-fixture -p osmium-runner -p osmium-examples
        ;;
      cli)
        cargo test --locked -p osmium-cli
        ;;
      examples)
        cargo test --locked -p osmium-examples
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
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
    ;;
  *)
    usage
    ;;
esac
