#!/bin/sh

set -eu

usage() {
    cat <<'EOF'
Usage: tools/acceptance/verify_fixture_bundle.sh --bundle <directory> [--manifest <file>] [--report <file>]

Verifies a private/internal or synthetic fixture bundle without contacting Teralion.
The manifest records metadata and the payload checksum; credentials are never read.
EOF
}

bundle=
manifest=
report=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --bundle)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            bundle=$2
            shift 2
            ;;
        --manifest)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            manifest=$2
            shift 2
            ;;
        --report)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            report=$2
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

[ -n "$bundle" ] || { usage >&2; exit 2; }
bundle=$(CDPATH= cd -- "$bundle" && pwd)
[ -n "$manifest" ] || manifest=$bundle/manifest.yaml
[ -f "$manifest" ] || {
    echo "fixture bundle manifest is missing: $manifest" >&2
    exit 2
}

command -v ruby >/dev/null 2>&1 || {
    echo "required command is unavailable: ruby" >&2
    exit 2
}

ruby -ryaml -rjson -rdigest -rdate -rpathname -rfileutils -e '
root, manifest_path, report_path = ARGV
root = File.expand_path(root)
manifest = YAML.safe_load(
  File.read(manifest_path),
  permitted_classes: [Date],
  aliases: false
)

abort("fixture bundle manifest must be a mapping") unless manifest.is_a?(Hash)
abort("unsupported fixture bundle format") unless manifest["bundle_format_version"] == 1
scope = manifest.fetch("distribution_scope")
unless ["private-internal-review-only", "synthetic-redistributable"].include?(scope)
  abort("unsupported fixture distribution scope: #{scope}")
end
if scope == "private-internal-review-only"
  authorization = manifest.fetch("authorization")
  abort("private fixture bundle must require authorization") unless authorization["required"] == true
  abort("private fixture bundle must declare a token environment") if authorization["token_env"].to_s.empty?
end
entries = manifest.fetch("entries")
abort("fixture bundle has no entries") unless entries.is_a?(Array) && !entries.empty?

def safe_path(root, relative)
  path = Pathname.new(relative.to_s)
  abort("fixture path must be relative: #{relative}") if path.absolute?
  clean = path.cleanpath.to_s
  abort("fixture path escapes bundle: #{relative}") if clean == ".." || clean.start_with?("../")
  full = File.expand_path(File.join(root, clean))
  prefix = "#{root}#{File::SEPARATOR}"
  abort("fixture path escapes bundle: #{relative}") unless full.start_with?(prefix)
  full
end

def fixture_digest(files)
  digest = Digest::SHA256.new
  files.each { |file| digest.update(File.binread(file)) }
  digest.hexdigest
end

verified = []
entries.each do |entry|
  abort("fixture entry must be a mapping") unless entry.is_a?(Hash)
  id = entry.fetch("id")
  path = safe_path(root, entry.fetch("path"))
  abort("fixture path is missing: #{id}") unless File.directory?(path)
  files = Dir.glob(File.join(path, "**", "*.jsonl")).sort
  abort("fixture has no JSONL payload: #{id}") if files.empty?

  records = 0
  files.each do |file|
    relative_file = Pathname.new(file).relative_path_from(Pathname.new(root)).to_s
    abort("forbidden fixture path: #{relative_file}") if relative_file.split(File::SEPARATOR).include?("raw")
    File.foreach(file) do |line|
      record = JSON.parse(line)
      abort("fixture record is not an object: #{relative_file}") unless record.is_a?(Hash)
      expected_market = entry["source_market"] || entry["market"]
      abort("fixture market mismatch: #{id}") if expected_market && record["market"] != expected_market
      abort("fixture symbol mismatch: #{id}") if entry["symbol"] && record["symbol"] != entry["symbol"]
      abort("fixture record is missing match_time: #{id}") unless record["match_time"].is_a?(String)
      abort("fixture record is missing received_at: #{id}") unless record["received_at"].is_a?(String)
      forbidden = record.keys.grep(/\A(?:authorization|api[_-]?key|cookie|password|secret|token|next_cursor)\z/i)
      abort("fixture payload contains forbidden field #{forbidden.first}: #{id}") unless forbidden.empty?
      records += 1
    end
  end
  if entry["record_count"] && records != entry["record_count"]
    abort("fixture record count mismatch for #{id}: #{records} != #{entry["record_count"]}")
  end
  actual = fixture_digest(files)
  abort("fixture checksum mismatch for #{id}: #{actual}") unless actual == entry.fetch("fixture_set_sha256")
  verified << { "id" => id, "records" => records, "fixture_set_sha256" => actual }
  puts "fixture=#{id} records=#{records} sha256=#{actual}"
end

if report_path && !report_path.empty?
  FileUtils.mkdir_p(File.dirname(File.expand_path(report_path)))
  File.write(
    report_path,
    {
      "verification_version" => 1,
      "status" => "passed",
      "bundle_id" => manifest.fetch("bundle_id"),
      "distribution_scope" => scope,
      "entries" => verified
    }.to_yaml
  )
end
' "$bundle" "$manifest" "$report"

echo "fixture_bundle=verified"
