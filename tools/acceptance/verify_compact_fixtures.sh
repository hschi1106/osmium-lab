#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
manifest=$root/fixtures/acceptance/manifest.yaml

"$root/tools/acceptance/verify_fixture_bundle.sh" \
    --bundle "$root" \
    --manifest "$manifest"

command -v ruby >/dev/null 2>&1 || {
    echo "required command is unavailable: ruby" >&2
    exit 2
}

ruby -ryaml -rjson -rdate -rpathname -e '
root, manifest_path = ARGV
root = File.expand_path(root)
manifest = YAML.safe_load(
  File.read(manifest_path),
  permitted_classes: [Date],
  aliases: false
)

abort("synthetic fixture manifest must be a mapping") unless manifest.is_a?(Hash)
abort("fixture manifest must be synthetic-redistributable") unless manifest["distribution_scope"] == "synthetic-redistributable"
entries = manifest.fetch("entries")
abort("synthetic fixture manifest has no entries") unless entries.is_a?(Array) && !entries.empty?

MAX_TOTAL_BYTES = 10 * 1024 * 1024
MAX_SESSION_BYTES = 512 * 1024
MAX_RECORDS_PER_SESSION = 512

def safe_path(root, relative)
  path = Pathname.new(relative.to_s)
  abort("fixture path must be relative: #{relative}") if path.absolute?
  clean = path.cleanpath.to_s
  abort("fixture path escapes repository: #{relative}") if clean == ".." || clean.start_with?("../")
  full = File.expand_path(File.join(root, clean))
  prefix = "#{root}#{File::SEPARATOR}"
  abort("fixture path escapes repository: #{relative}") unless full.start_with?(prefix)
  full
end

manifest_paths = []
total_bytes = 0
total_records = 0
jsonl_files = []

entries.each do |entry|
  abort("fixture entry must be a mapping") unless entry.is_a?(Hash)
  abort("synthetic manifest entry must set complete_day=false") unless entry["complete_day"] == false
  abort("synthetic fixture symbol must use SYNTH- prefix") unless entry.fetch("symbol").to_s.start_with?("SYNTH-")
  abort("fixture entry must be synthetic-redistributable") unless entry["redistribution"] == "synthetic-redistributable"
  relative = entry.fetch("path").to_s
  abort("synthetic fixture must live under fixtures/teralion: #{relative}") unless relative.start_with?("fixtures/teralion/")
  path = safe_path(root, relative)
  abort("synthetic fixture path is missing: #{relative}") unless File.directory?(path)
  manifest_paths << relative

  metadata_path = File.join(path, "metadata.yaml")
  abort("synthetic fixture metadata is missing: #{relative}") unless File.file?(metadata_path)
  metadata = YAML.safe_load(File.read(metadata_path), permitted_classes: [Date], aliases: false)
  abort("synthetic fixture metadata must be a mapping: #{relative}") unless metadata.is_a?(Hash)
  abort("fixture_scope must be synthetic_scenario: #{relative}") unless metadata["fixture_scope"] == "synthetic_scenario"
  abort("fixture provenance must be repository-owned-synthetic: #{relative}") unless metadata["provenance"] == "repository-owned-synthetic"
  abort("metadata complete_day must be false: #{relative}") unless metadata["complete_day"] == false
  abort("metadata artifact.full_day must be false: #{relative}") unless metadata.dig("artifact", "full_day") == false

  files = Dir.glob(File.join(path, "**", "*.jsonl")).sort
  abort("compact fixture has no JSONL payload: #{relative}") if files.empty?
  records_for_entry = 0
  files.each do |file|
    relative_file = Pathname.new(file).relative_path_from(Pathname.new(root)).to_s
    bytes = File.size(file)
    abort("synthetic session shard exceeds 512 KiB: #{relative_file}") if bytes > MAX_SESSION_BYTES
    count = 0
    File.foreach(file) do |line|
      record = JSON.parse(line)
      abort("synthetic fixture record must be an object: #{relative_file}") unless record.is_a?(Hash)
      abort("synthetic fixture record is missing match_time: #{relative_file}") unless record["match_time"].is_a?(String)
      abort("synthetic fixture record is missing received_at: #{relative_file}") unless record["received_at"].is_a?(String)
      count += 1
      records_for_entry += 1
    end
    abort("synthetic session shard exceeds 512 records: #{relative_file}") if count > MAX_RECORDS_PER_SESSION
    total_bytes += bytes
    total_records += count
    jsonl_files << relative_file
  end
  expected = metadata.dig("artifact", "record_count")
  abort("metadata record count mismatch: #{relative}") unless expected == records_for_entry

  golden = File.join(path, "golden")
  if File.directory?(golden)
    Dir.children(golden).each do |name|
      abort("full-day golden artifact is not allowed: #{relative}/golden/#{name}") unless name == "fixture-set.sha256"
    end
  end

end

metadata_paths = Dir.glob(File.join(root, "fixtures/teralion/**/metadata.yaml")).map do |file|
  Pathname.new(File.dirname(file)).relative_path_from(Pathname.new(root)).to_s
end.sort
abort("manifest and synthetic fixture directories differ") unless metadata_paths == manifest_paths.sort
abort("synthetic fixture tree exceeds 10 MiB: #{total_bytes}") if total_bytes > MAX_TOTAL_BYTES
required = [
  "fixtures/teralion/twse/SYNTH-TWSE-EQ/2026-07-20",
  "fixtures/teralion/twse/SYNTH-TWSE-W/2026-07-20",
  "fixtures/teralion/tpex/SYNTH-TPEX-EQ/2026-07-20",
  "fixtures/teralion/tpex/SYNTH-TPEX-W/2026-07-20",
  "fixtures/teralion/taifex/SYNTH-FUT/2026-07-20",
  "fixtures/teralion/taifex/SYNTH-OPT/2026-07-20"
]
abort("synthetic fixture matrix is incomplete") unless (required - manifest_paths).empty?
puts "synthetic_fixtures=verified entries=#{entries.length} jsonl_files=#{jsonl_files.length} records=#{total_records} bytes=#{total_bytes}"
' "$root" "$manifest"
