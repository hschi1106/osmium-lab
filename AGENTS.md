# Agent Instructions

This repository is `osmium-lab`, a Rust market replay and backtesting platform. The project specification is [spec.md](spec.md). Treat that file as the source of truth for product scope, architecture, terminology, and delivery priorities.

## Start Here

Before making project changes, read `spec.md` and align the work with it. If a requested change appears to conflict with the spec, call out the conflict before editing code.

Use the existing workspace structure and Rust conventions. Keep changes scoped to the crate or module that owns the behavior being changed.

## Change Scope

Do not modify the project at large scale in a single pass. Split work into small, reviewable commits or commit-sized changes.

Prefer this workflow:

1. Identify the smallest useful step.
2. Implement only that step.
3. Run focused validation.
4. Explain what changed and how to test it.
5. Continue with the next step only after the previous step is clear.

Avoid broad rewrites, speculative abstractions, large file moves, and unrelated formatting churn unless the user explicitly asks for them.

## Spec Alignment

Preserve the main architecture described in `spec.md`:

- Permanent raw archive is the canonical source.
- Derived replay datasets are rebuildable artifacts.
- Teralion wire format and domain events must stay separated.
- Market replay ordering is based primarily on `match_time`, with deterministic tie-breaking.
- Strategies read market state but do not mutate it.
- The replayer should open only the streams required by the strategy universe.

If implementation details are not yet defined, choose the simplest design that keeps these boundaries intact.

## Communication Requirements

For every code or documentation change, tell the user:

- What files changed.
- What behavior, API, or documentation changed.
- Why the change was kept to this scope.
- How to test or inspect the change.

If tests cannot be run, explain why and provide the closest manual verification step.

## Validation

Use focused checks first. For Rust changes, prefer:

```sh
cargo fmt --check
cargo test
```

Use narrower commands when the change only affects one crate or module. Do not claim a change is verified unless the relevant command actually ran successfully.

## Git Hygiene

Respect existing user changes. Do not revert, overwrite, or clean unrelated work unless the user explicitly asks.

When commits are requested, keep each commit centered on one logical change and use a message that describes the user-visible or architectural effect.
