#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/release/smoke_clean_machine.sh --archive <osmium.tar.gz> [--checksum <file>]

Installs an archive into an isolated temporary prefix and checks version, help, JSON
output, quiet mode, and the packaged neutral example without network or credentials.
EOF
}

archive=
checksum=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --archive)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            archive=$2
            shift 2
            ;;
        --checksum)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            checksum=$2
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
[ -n "$archive" ] || { usage >&2; exit 2; }

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
staging=$(mktemp -d "${TMPDIR:-/tmp}/osmium-clean-machine.XXXXXX")
cleanup() {
    rm -rf "$staging"
}
trap cleanup EXIT HUP INT TERM

if [ -n "$checksum" ]; then
    "$root/tools/release/install.sh" --archive "$archive" --checksum "$checksum" --prefix "$staging/prefix"
else
    "$root/tools/release/install.sh" --archive "$archive" --prefix "$staging/prefix"
fi

binary=$staging/prefix/bin/osmium

version_output=$(cd "$staging" && env -i PATH="$staging/prefix/bin:/usr/bin:/bin" LANG=C "$binary" version)
printf '%s\n' "$version_output" | grep -Eq '^osmium [0-9]+\.[0-9]+\.[0-9]+$'
printf '%s\n' "$version_output" | grep -Eq '^config_schema=2$'

help_output=$(cd "$staging" && env -i PATH="$staging/prefix/bin:/usr/bin:/bin" LANG=C "$binary" --help)
printf '%s\n' "$help_output" | grep -Eq 'osmium data sync\|verify'

json_output=$(cd "$staging" && env -i PATH="$staging/prefix/bin:/usr/bin:/bin" LANG=C "$binary" version --format json)
printf '%s\n' "$json_output" | grep -Eq '"status":"success"'
printf '%s\n' "$json_output" | grep -Eq '"config_schema":2'

quiet_output=$(cd "$staging" && env -i PATH="$staging/prefix/bin:/usr/bin:/bin" LANG=C "$binary" version --quiet)
[ -z "$quiet_output" ] || { echo "quiet mode emitted output" >&2; exit 1; }

env -i PATH="$staging/prefix/bin:/usr/bin:/bin" LANG=C "$binary" \
    config check --config "$staging/prefix/examples/config.yaml" --format json >/dev/null

echo "clean_machine=passed"
