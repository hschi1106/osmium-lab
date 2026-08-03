#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/acceptance/fetch_fixture_bundle.sh --source <directory|archive|https-url> --output <directory> [--token-env <name>]

Fetches an authorized fixture bundle, verifies its manifest/checksums, and atomically
publishes a new local bundle directory. HTTPS sources require a bearer token in the
named environment variable; local sources are useful for offline verification.
EOF
}

source=
output=
token_env=OSMIUM_FIXTURE_BUNDLE_TOKEN
while [ "$#" -gt 0 ]; do
    case "$1" in
        --source)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            source=$2
            shift 2
            ;;
        --output)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            output=$2
            shift 2
            ;;
        --token-env)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            token_env=$2
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

[ -n "$source" ] && [ -n "$output" ] || { usage >&2; exit 2; }
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
case "$output" in
    /*) output_path=$output ;;
    *) output_path=$root/$output ;;
esac
case "$output_path" in
    "$root"/*) ;;
    *) echo "fixture output must be inside the repository" >&2; exit 2 ;;
esac
[ ! -e "$output_path" ] || { echo "fixture output already exists: $output_path" >&2; exit 2; }

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

verify_archive_paths() {
    archive=$1
    tar -tzf "$archive" | awk '
        { path=$0; sub(/^\.\//, "", path); if (path ~ /^\// || path ~ /(^|\/)\.\.(\/|$)/) bad=1 }
        END { exit bad ? 1 : 0 }
    '
}

staging=$(mktemp -d "${TMPDIR:-/tmp}/osmium-fixture-fetch.XXXXXX")
cleanup() {
    rm -rf "$staging"
}
trap cleanup EXIT HUP INT TERM

case "$source" in
    http://*|https://*)
        command -v curl >/dev/null 2>&1 || { echo "required command is unavailable: curl" >&2; exit 2; }
        token=$(printenv "$token_env" 2>/dev/null || true)
        [ -n "$token" ] || {
            echo "HTTPS fixture source requires token environment variable: $token_env" >&2
            exit 30
        }
        archive=$staging/bundle.tar.gz
        printf 'Authorization: Bearer %s\n' "$token" \
            | curl --fail --location --retry 2 --silent --show-error --header @- "$source" --output "$archive"
        verify_archive_paths "$archive" || {
            echo "fixture archive contains an unsafe path" >&2
            exit 50
        }
        mkdir "$staging/extracted"
        tar -xzf "$archive" -C "$staging/extracted"
        bundle_root=$staging/extracted
        ;;
    *.tar.gz|*.tgz)
        [ -f "$source" ] || { echo "fixture archive is missing: $source" >&2; exit 2; }
        verify_archive_paths "$source" || {
            echo "fixture archive contains an unsafe path" >&2
            exit 50
        }
        mkdir "$staging/extracted"
        tar -xzf "$source" -C "$staging/extracted"
        bundle_root=$staging/extracted
        ;;
    *)
        [ -d "$source" ] || { echo "fixture source directory is missing: $source" >&2; exit 2; }
        bundle_root=$staging/extracted
        mkdir "$bundle_root"
        cp -R "$source"/. "$bundle_root"/
        ;;
esac

[ -f "$bundle_root/manifest.yaml" ] || {
    echo "fixture bundle must contain manifest.yaml at its root" >&2
    exit 50
}

if [ -f "$bundle_root/checksums.sha256" ]; then
    (
        cd "$bundle_root"
        while IFS='  ' read -r expected file; do
            [ -n "$expected" ] || continue
            [ -f "$file" ] || { echo "checksum file is missing: $file" >&2; exit 50; }
            actual=$(sha256_file "$file")
            [ "$actual" = "$expected" ] || {
                echo "bundle checksum mismatch: $file" >&2
                exit 50
            }
        done <checksums.sha256
    )
fi

"$root/tools/acceptance/verify_fixture_bundle.sh" \
    --bundle "$bundle_root" \
    --report "$bundle_root/verification-report.yaml"

mkdir -p "$(dirname -- "$output_path")"
mv "$bundle_root" "$output_path"
trap - EXIT HUP INT TERM
echo "fixture_bundle=$output_path"
