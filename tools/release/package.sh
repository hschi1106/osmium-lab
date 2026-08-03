#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: tools/release/package.sh --output <archive.tar.gz> [--version <version>] [--target <target>]

Builds the osmium release binary archive. The archive contains the binary,
neutral example/config documentation, release notes, license, and metadata only.
Source data, raw dumps, target files, credentials, and acceptance payloads are excluded.
The tar.gz bytes are deterministic when SOURCE_DATE_EPOCH is fixed.
EOF
}

output=
version=
target=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            output=$2
            shift 2
            ;;
        --version)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            version=$2
            shift 2
            ;;
        --target)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            target=$2
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
cd "$root"

if [ -z "$version" ]; then
    version=$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)
fi
[ -n "$version" ] || {
    echo "unable to resolve release version" >&2
    exit 1
}
if [ -z "$target" ]; then
    target=$(rustc -vV | awk '/^host: / { print $2; exit }')
fi
[ -n "$target" ] || {
    echo "unable to resolve Rust target" >&2
    exit 1
}

command -v python3 >/dev/null 2>&1 || {
    echo "required command is unavailable: python3" >&2
    exit 2
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

case "$output" in
    /*) output_path=$output ;;
    *) output_path=$root/$output ;;
esac
case "$output_path" in
    "$root"/*) ;;
    *)
        echo "archive output must be inside the repository" >&2
        exit 2
        ;;
esac
[ ! -e "$output_path" ] || {
    echo "archive already exists: $output_path" >&2
    exit 2
}

package_name=osmium-$version-$target
parent=$(dirname -- "$output_path")
mkdir -p "$parent"
staging=$(mktemp -d "${TMPDIR:-/tmp}/osmium-package.XXXXXX")
cleanup() {
    rm -rf "$staging"
}
trap cleanup EXIT HUP INT TERM

echo "building osmium release binary"
env CARGO_TARGET_DIR="$root/target" cargo build --locked --release -p osmium-cli

package_dir=$staging/$package_name
mkdir -p "$package_dir/bin" "$package_dir/examples" "$package_dir/docs"
cp "$root/target/release/osmium" "$package_dir/bin/osmium"
cp "$root/examples/config.yaml" "$package_dir/examples/config.yaml"
cp "$root/docs/quickstart.md" "$package_dir/docs/quickstart.md"
cp "$root/docs/config-reference.md" "$package_dir/docs/config-reference.md"
cp "$root/docs/data-layout.md" "$package_dir/docs/data-layout.md"
cp "$root/fixtures/acceptance/manifest.yaml" "$package_dir/fixture-manifest.yaml"
cp "$root/LICENSE" "$package_dir/LICENSE"
cp "$root/docs/release/RELEASE-NOTES.md" "$package_dir/RELEASE-NOTES.md"
cp "$root/docs/release/SUPPORT.md" "$package_dir/SUPPORT.md"

cat >"$package_dir/BUILD-METADATA" <<EOF
product: osmium
version: $version
target: $target
distribution: public-release
archive_contents: binary-and-documentation-only
acceptance_payloads: excluded
source_date_epoch: ${SOURCE_DATE_EPOCH:-0}
EOF

(
    cd "$package_dir"
    cargo tree --manifest-path "$root/crates/osmium-cli/Cargo.toml" --locked \
        --prefix none --depth 1 \
        | sed -E 's/ \([^)]*\)//g' >DEPENDENCIES.txt
    python3 "$root/tools/release/generate_inventory.py" \
        --root "$root" \
        --version "$version" \
        --sbom "$package_dir/SBOM.cdx.json" \
        --licenses "$package_dir/THIRD-PARTY-LICENSES.txt"
    find . -type f ! -name SHA256SUMS ! -name SHA256SUMS.tmp -print \
        | LC_ALL=C sort \
        | while IFS= read -r file; do
            printf '%s  %s\n' "$(sha256_file "$file")" "$file"
        done >SHA256SUMS.tmp
    mv SHA256SUMS.tmp SHA256SUMS
)

echo "checking archive contents"
if find "$package_dir" -type f \( -name '.env' -o -path '*/raw/*' -o -path '*/target/*' \) \
    | grep -q .; then
    echo "archive contains forbidden path" >&2
    exit 1
fi

python3 "$root/tools/release/create_archive.py" \
    --root "$package_dir" \
    --name "$package_name" \
    --output "$output_path" \
    --source-date-epoch "${SOURCE_DATE_EPOCH:-0}"
sha256_file "$output_path" >"$output_path.sha256"
echo "archive=$output_path"
echo "checksum=$output_path.sha256"
