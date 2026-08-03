#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/release/verify_reproducibility.sh --output <new-directory>

Builds two archives with the same SOURCE_DATE_EPOCH, compares their complete bytes,
and runs the clean-machine installer smoke test against the result.
EOF
}

output=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
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
[ -n "$output" ] || { usage >&2; exit 2; }

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
case "$output" in
    /*) output_path=$output ;;
    *) output_path=$root/$output ;;
esac
case "$output_path" in
    "$root"/*) ;;
    *) echo "reproducibility output must be inside the repository" >&2; exit 2 ;;
esac
[ ! -e "$output_path" ] || { echo "reproducibility output already exists: $output_path" >&2; exit 2; }
mkdir -p "$output_path"

epoch=${SOURCE_DATE_EPOCH:-0}
SOURCE_DATE_EPOCH=$epoch "$root/tools/release/package.sh" \
    --output "$output_path/first.tar.gz"
SOURCE_DATE_EPOCH=$epoch "$root/tools/release/package.sh" \
    --output "$output_path/second.tar.gz"
cmp "$output_path/first.tar.gz" "$output_path/second.tar.gz"
cmp "$output_path/first.tar.gz.sha256" "$output_path/second.tar.gz.sha256"

"$root/tools/release/smoke_clean_machine.sh" \
    --archive "$output_path/first.tar.gz" \
    --checksum "$output_path/first.tar.gz.sha256"

echo "reproducibility=byte-identical"
