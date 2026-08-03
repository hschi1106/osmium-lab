#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/release/install.sh --archive <osmium.tar.gz> --prefix <absolute-directory> [--checksum <file>]

Installs an already downloaded internal archive without network access. The external
checksum and the archive's internal SHA256SUMS are checked before files are copied.
EOF
}

archive=
prefix=
checksum=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --archive)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            archive=$2
            shift 2
            ;;
        --prefix)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            prefix=$2
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

[ -n "$archive" ] && [ -n "$prefix" ] || { usage >&2; exit 2; }
case "$prefix" in
    /*) ;;
    *) echo "--prefix must be an absolute directory" >&2; exit 2 ;;
esac
[ -f "$archive" ] || { echo "archive is missing: $archive" >&2; exit 2; }

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

verify_archive_paths() {
    tar -tzf "$1" | awk '
        { path=$0; sub(/^\.\//, "", path); if (path ~ /^\// || path ~ /(^|\/)\.\.(\/|$)/) bad=1 }
        END { exit bad ? 1 : 0 }
    '
}

if [ -n "$checksum" ]; then
    [ -f "$checksum" ] || { echo "checksum file is missing: $checksum" >&2; exit 2; }
    expected=$(awk 'NR == 1 { print $1; exit }' "$checksum")
    actual=$(sha256_file "$archive")
    [ "$actual" = "$expected" ] || {
        echo "archive checksum mismatch" >&2
        exit 50
    }
fi
verify_archive_paths "$archive" || {
    echo "archive contains an unsafe path" >&2
    exit 50
}

staging=$(mktemp -d "${TMPDIR:-/tmp}/osmium-install.XXXXXX")
cleanup() {
    rm -rf "$staging"
}
trap cleanup EXIT HUP INT TERM
tar -xzf "$archive" -C "$staging"

archive_root=$(find "$staging" -mindepth 1 -maxdepth 1 -type d -print | head -1)
[ -n "$archive_root" ] || { echo "archive has no package directory" >&2; exit 50; }
[ -f "$archive_root/bin/osmium" ] || { echo "archive binary is missing" >&2; exit 50; }
[ -f "$archive_root/SHA256SUMS" ] || { echo "archive SHA256SUMS is missing" >&2; exit 50; }

(
    cd "$archive_root"
    while IFS='  ' read -r expected file; do
        [ -n "$expected" ] || continue
        [ -f "$file" ] || { echo "archive file is missing: $file" >&2; exit 50; }
        actual=$(sha256_file "$file")
        [ "$actual" = "$expected" ] || {
            echo "archive internal checksum mismatch: $file" >&2
            exit 50
        }
    done <SHA256SUMS
)

mkdir -p "$prefix/bin" "$prefix/examples" "$prefix/docs"
install -m 755 "$archive_root/bin/osmium" "$prefix/bin/osmium"
for file in "$archive_root"/examples/*; do
    [ -f "$file" ] || continue
    install -m 644 "$file" "$prefix/examples/$(basename -- "$file")"
done
for file in "$archive_root"/docs/*; do
    [ -f "$file" ] || continue
    install -m 644 "$file" "$prefix/docs/$(basename -- "$file")"
done
for file in RELEASE-NOTES.md SUPPORT.md LICENSE BUILD-METADATA DEPENDENCIES.txt SBOM.cdx.json THIRD-PARTY-LICENSES.txt fixture-manifest.yaml; do
    [ -f "$archive_root/$file" ] || continue
    install -m 644 "$archive_root/$file" "$prefix/$file"
done

echo "installed=$prefix/bin/osmium"
