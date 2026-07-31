#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
fixture_root="$root/fixtures/teralion/taifex"
evidence="$root/docs/verification/evidence/m3/source-selection-2026-07-31.yaml"

command -v ruby >/dev/null 2>&1 || {
    echo "required command is unavailable: ruby" >&2
    exit 2
}
command -v rg >/dev/null 2>&1 || {
    echo "required command is unavailable: rg" >&2
    exit 2
}

ruby -ryaml -rdigest -e '
fixture_root, evidence_path = ARGV

metadata_paths = Dir.glob("#{fixture_root}/*/2026-07-20/metadata.yaml").sort
abort("expected three M3 fixture metadata files") unless metadata_paths.length == 3

metadata_paths.each do |path|
  metadata = YAML.safe_load(
    File.read(path),
    permitted_classes: [],
    aliases: false
  )
  root = File.dirname(path)
  shards = metadata.fetch("segments")
    .values
    .flat_map { |segment| segment.fetch("shards") }

  shards.each do |shard|
    file = File.join(root, shard.fetch("path"))
    bytes = File.binread(file)
    abort("#{file}: byte count mismatch") unless bytes.bytesize == shard.fetch("bytes")
    abort("#{file}: record count mismatch") unless bytes.count("\n") == shard.fetch("records")
    abort("#{file}: checksum mismatch") unless Digest::SHA256.hexdigest(bytes) == shard.fetch("sha256")
  end

  bytes = shards
    .sort_by { |shard| shard.fetch("path") }
    .map { |shard| File.binread(File.join(root, shard.fetch("path"))) }
    .join
  artifact = metadata.fetch("artifact")
  abort("#{path}: artifact byte count mismatch") unless bytes.bytesize == artifact.fetch("byte_count")
  abort("#{path}: artifact record count mismatch") unless bytes.count("\n") == artifact.fetch("record_count")
  abort("#{path}: artifact checksum mismatch") unless Digest::SHA256.hexdigest(bytes) == artifact.fetch("sha256")

  golden = File.read(File.join(root, "golden/fixture-set.sha256")).strip
  abort("#{path}: golden checksum mismatch") unless golden == artifact.fetch("sha256")
end

evidence = YAML.safe_load(
  File.read(evidence_path),
  permitted_classes: [],
  aliases: false
)
files = Dir.glob("#{fixture_root}/**/*.jsonl").sort
bytes = files.map { |path| File.binread(path) }.join
fixture = evidence.fetch("committed_fixture")

abort("global shard count mismatch") unless files.length == fixture.fetch("shard_count")
abort("global byte count mismatch") unless bytes.bytesize == fixture.fetch("byte_count")
abort("global record count mismatch") unless bytes.count("\n") == fixture.fetch("record_count")
abort("global checksum mismatch") unless Digest::SHA256.hexdigest(bytes) == fixture.fetch("sha256")

puts "M3 fixture integrity: passed"
puts "symbols=#{metadata_paths.length}"
puts "shards=#{files.length}"
puts "records=#{bytes.count("\n")}"
puts "sha256=#{Digest::SHA256.hexdigest(bytes)}"
' "$fixture_root" "$evidence"

if rg -n -i \
    '"(authorization|api[_-]?key|cookie|password|secret|token|next_cursor)"[[:space:]]*:' \
    "$fixture_root"; then
    echo "M3 fixture contains a forbidden field" >&2
    exit 1
fi

echo "M3 fixture secret scan: passed"
