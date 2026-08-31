#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
expected_license=AGPL-3.0-only
expected_exception="Strategy Linking Exception"

workspace_license=$(awk -F'"' '/^license = / { print $2; exit }' "$root/Cargo.toml")
[ "$workspace_license" = "$expected_license" ] || {
    echo "workspace license must be $expected_license, got: $workspace_license" >&2
    exit 1
}

[ -f "$root/LICENSE" ] || {
    echo "repository license is missing" >&2
    exit 1
}
grep -Fq "GNU AFFERO GENERAL PUBLIC LICENSE" "$root/LICENSE" || {
    echo "repository license is not AGPL-3.0-only" >&2
    exit 1
}
grep -Fiq "$expected_exception" "$root/LICENSE" || {
    echo "repository license is missing $expected_exception" >&2
    exit 1
}

check_manifest() {
    manifest=$1
    metadata=$(cargo metadata \
        --manifest-path "$manifest" \
        --locked \
        --format-version 1 \
        --no-deps)
    printf '%s\n' "$metadata" | python3 -c '
import json
import sys

metadata = json.load(sys.stdin)
bad = [
    str(package["name"]) + "=" + repr(package.get("license"))
    for package in metadata["packages"]
    if package.get("source") is None and package.get("license") != "AGPL-3.0-only"
]
if bad:
    raise SystemExit("local Cargo packages have unexpected licenses: " + ", ".join(bad))
'
}

check_manifest "$root/Cargo.toml"
check_manifest "$root/tools/acceptance/osmium_fixture_data/Cargo.toml"

old_permissive_license=$(printf 'M%s' 'IT')
if git -C "$root" grep -n -i -E \
    "$old_permissive_license License|license = \"$old_permissive_license\"|License-$old_permissive_license|\\[$old_permissive_license License\\]" \
    -- ':!Cargo.lock' >/dev/null 2>&1; then
    echo "repository still contains an old permissive license declaration" >&2
    exit 1
fi

echo "license=verified expression=$expected_license exception=$expected_exception"
