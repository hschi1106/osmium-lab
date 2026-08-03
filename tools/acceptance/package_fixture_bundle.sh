#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/acceptance/package_fixture_bundle.sh --source <repository-root> --output <bundle.tar.gz> [--manifest <file>]

Creates an access-controlled fixture bundle. The bundle contains only the manifest and
the payload paths explicitly listed by that manifest; it never reads credentials.
EOF
}

source_root=
output=
manifest=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --source)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            source_root=$2
            shift 2
            ;;
        --output)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            output=$2
            shift 2
            ;;
        --manifest)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            manifest=$2
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

[ -n "$source_root" ] && [ -n "$output" ] || { usage >&2; exit 2; }
source_root=$(CDPATH= cd -- "$source_root" && pwd)
[ -n "$manifest" ] || manifest=$source_root/fixtures/acceptance/manifest.yaml
[ -f "$manifest" ] || { echo "manifest is missing: $manifest" >&2; exit 2; }

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
case "$output" in
    /*) output_path=$output ;;
    *) output_path=$root/$output ;;
esac
case "$output_path" in
    "$root"/*) ;;
    *) echo "bundle output must be inside the repository" >&2; exit 2 ;;
esac
[ ! -e "$output_path" ] || { echo "bundle already exists: $output_path" >&2; exit 2; }

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

staging=$(mktemp -d "${TMPDIR:-/tmp}/osmium-fixture-bundle.XXXXXX")
cleanup() {
    rm -rf "$staging"
}
trap cleanup EXIT HUP INT TERM

command -v ruby >/dev/null 2>&1 || { echo "required command is unavailable: ruby" >&2; exit 2; }
ruby -ryaml -rfileutils -rpathname -e '
source_root, manifest_path, destination = ARGV
manifest = YAML.safe_load(File.read(manifest_path), aliases: false)
entries = manifest.fetch("entries")
FileUtils.cp(manifest_path, File.join(destination, "manifest.yaml"))
entries.each do |entry|
  relative = entry.fetch("path").to_s
  abort("manifest path must be relative") if Pathname.new(relative).absolute?
  source = File.expand_path(File.join(source_root, relative))
  prefix = "#{source_root}#{File::SEPARATOR}"
  abort("manifest path escapes source root") unless source.start_with?(prefix)
  abort("fixture path is missing: #{relative}") unless File.exist?(source)
  destination_path = File.join(destination, relative)
  FileUtils.mkdir_p(File.dirname(destination_path))
  FileUtils.cp_r(source, destination_path)
end
' "$source_root" "$manifest" "$staging"

"$root/tools/acceptance/verify_fixture_bundle.sh" --bundle "$staging"

(
    cd "$staging"
    find . -type f ! -name checksums.sha256 -print \
        | sed 's#^\./##' \
        | LC_ALL=C sort \
        | while IFS= read -r file; do
            printf '%s  %s\n' "$(sha256_file "$file")" "$file"
        done >checksums.sha256
)

mkdir -p "$(dirname -- "$output_path")"
tar -czf "$output_path" -C "$staging" .
sha256_file "$output_path" >"$output_path.sha256"
echo "bundle=$output_path"
echo "checksum=$output_path.sha256"
