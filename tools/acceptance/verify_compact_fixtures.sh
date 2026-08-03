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

abort("compact fixture manifest must be a mapping") unless manifest.is_a?(Hash)
entries = manifest.fetch("entries")
abort("compact fixture manifest has no entries") unless entries.is_a?(Array) && !entries.empty?

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

def fail_if_date_0727(path)
  abort("removed TWSE fixture date is still present: #{path}") if path.include?("2026-07-27")
end

manifest_paths = []
total_bytes = 0
total_records = 0
jsonl_files = []

entries.each do |entry|
  abort("fixture entry must be a mapping") unless entry.is_a?(Hash)
  abort("compact manifest entry must set complete_day=false") unless entry["complete_day"] == false
  relative = entry.fetch("path").to_s
  abort("compact fixture must live under fixtures/teralion: #{relative}") unless relative.start_with?("fixtures/teralion/")
  path = safe_path(root, relative)
  fail_if_date_0727(relative)
  abort("compact fixture path is missing: #{relative}") unless File.directory?(path)
  manifest_paths << relative

  metadata_path = File.join(path, "metadata.yaml")
  abort("compact fixture metadata is missing: #{relative}") unless File.file?(metadata_path)
  metadata = YAML.safe_load(File.read(metadata_path), permitted_classes: [Date], aliases: false)
  abort("compact fixture metadata must be a mapping: #{relative}") unless metadata.is_a?(Hash)
  abort("fixture_scope must be representative_slice: #{relative}") unless metadata["fixture_scope"] == "representative_slice"
  abort("metadata complete_day must be false: #{relative}") unless metadata["complete_day"] == false
  abort("metadata artifact.full_day must be false: #{relative}") unless metadata.dig("artifact", "full_day") == false

  files = Dir.glob(File.join(path, "**", "*.jsonl")).sort
  abort("compact fixture has no JSONL payload: #{relative}") if files.empty?
  records_for_entry = 0
  files.each do |file|
    relative_file = Pathname.new(file).relative_path_from(Pathname.new(root)).to_s
    fail_if_date_0727(relative_file)
    bytes = File.size(file)
    abort("compact session shard exceeds 512 KiB: #{relative_file}") if bytes > MAX_SESSION_BYTES
    count = 0
    File.foreach(file) do |line|
      record = JSON.parse(line)
      abort("compact fixture record must be an object: #{relative_file}") unless record.is_a?(Hash)
      abort("compact fixture record is missing match_time: #{relative_file}") unless record["match_time"].is_a?(String)
      abort("compact fixture record is missing received_at: #{relative_file}") unless record["received_at"].is_a?(String)
      count += 1
      records_for_entry += 1
    end
    abort("compact session shard exceeds 512 records: #{relative_file}") if count > MAX_RECORDS_PER_SESSION
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

  Dir.glob(File.join(path, "**", "*"), File::FNM_DOTMATCH).each do |file|
    next unless File.file?(file)
    relative_file = Pathname.new(file).relative_path_from(Pathname.new(root)).to_s
    fail_if_date_0727(relative_file)
    if relative_file.include?("/twse/")
      abort("TWSE fixture payload contains removed date: #{relative_file}") if File.binread(file).include?("2026-07-27")
    end
  end
end

metadata_paths = Dir.glob(File.join(root, "fixtures/teralion/**/metadata.yaml")).map do |file|
  Pathname.new(File.dirname(file)).relative_path_from(Pathname.new(root)).to_s
end.sort
abort("manifest and compact fixture directories differ") unless metadata_paths == manifest_paths.sort
abort("compact fixture tree exceeds 10 MiB: #{total_bytes}") if total_bytes > MAX_TOTAL_BYTES
abort("TWSE 2330 2026-07-20 representative fixture is missing") unless manifest_paths.include?("fixtures/teralion/twse/2330/2026-07-20")
puts "compact_fixtures=verified entries=#{entries.length} jsonl_files=#{jsonl_files.length} records=#{total_records} bytes=#{total_bytes}"
' "$root" "$manifest"
