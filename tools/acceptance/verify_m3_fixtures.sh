#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
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

ruby -ryaml -rjson -rdigest -e '
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
  daily_path = File.join(root, "daily.json")
  abort("#{daily_path}: missing committed daily instrument") unless File.file?(daily_path)
  daily_bytes = File.binread(daily_path)
  expected_daily = metadata.fetch("source_acquisition").fetch("daily_instrument_sha256")
  abort("#{daily_path}: daily instrument checksum mismatch") unless Digest::SHA256.hexdigest(daily_bytes) == expected_daily
  daily = JSON.parse(daily_bytes)
  abort("#{daily_path}: symbol mismatch") unless daily.fetch("symbol") == metadata.fetch("symbol")
  abort("#{daily_path}: market mismatch") unless daily.fetch("market") == metadata.fetch("source_market")
  abort("#{daily_path}: trading date mismatch") unless daily.fetch("trading_date") == metadata.fetch("exchange_trading_date")

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

twse_fixture_root="$root/fixtures/teralion/twse"
ruby -ryaml -rjson -rdigest -e '
fixture_root = ARGV.fetch(0)
metadata_paths = Dir.glob("#{fixture_root}/2330/2026-07-20/metadata.yaml")
abort("expected the committed TWSE 2330 2026-07-20 fixture") unless metadata_paths == [
  "#{fixture_root}/2330/2026-07-20/metadata.yaml"
]

metadata_paths.each do |path|
  metadata = YAML.safe_load(
    File.read(path),
    permitted_classes: [],
    aliases: false
  )
  root = File.dirname(path)
  daily_path = File.join(root, "daily.json")
  abort("#{daily_path}: missing committed daily instrument") unless File.file?(daily_path)
  daily_bytes = File.binread(daily_path)
  expected_daily = metadata.fetch("source_acquisition").fetch("daily_instrument_sha256")
  abort("#{daily_path}: daily instrument checksum mismatch") unless Digest::SHA256.hexdigest(daily_bytes) == expected_daily
  daily = JSON.parse(daily_bytes)
  abort("#{daily_path}: symbol mismatch") unless daily.fetch("symbol") == metadata.fetch("symbol")
  abort("#{daily_path}: market mismatch") unless daily.fetch("market") == metadata.fetch("source_market")
  abort("#{daily_path}: trading date mismatch") unless daily.fetch("trading_date") == metadata.fetch("exchange_trading_date")

  shards = metadata.fetch("segments").values.flat_map { |segment| segment.fetch("shards") }
  abort("#{path}: expected 22 shards") unless shards.length == 22
  shards.each do |shard|
    file = File.join(root, shard.fetch("path"))
    abort("#{file}: missing shard") unless File.file?(file)
    bytes = File.binread(file)
    abort("#{file}: byte count mismatch") unless bytes.bytesize == shard.fetch("bytes")
    abort("#{file}: record count mismatch") unless bytes.count("\n") == shard.fetch("records")
    abort("#{file}: checksum mismatch") unless Digest::SHA256.hexdigest(bytes) == shard.fetch("sha256")
  end

  bytes = shards.sort_by { |shard| shard.fetch("path") }
    .map { |shard| File.binread(File.join(root, shard.fetch("path"))) }
    .join
  artifact = metadata.fetch("artifact")
  abort("#{path}: artifact byte count mismatch") unless bytes.bytesize == artifact.fetch("byte_count")
  abort("#{path}: artifact record count mismatch") unless bytes.count("\n") == artifact.fetch("record_count")
  abort("#{path}: artifact checksum mismatch") unless Digest::SHA256.hexdigest(bytes) == artifact.fetch("sha256")

  golden = File.read(File.join(root, "golden/fixture-set.sha256")).strip
  abort("#{path}: golden checksum mismatch") unless golden == artifact.fetch("sha256")
end

puts "TWSE fixture integrity: passed"
puts "symbols=1"
puts "shards=22"
puts "records=101869"
' "$twse_fixture_root"

if rg -n -i \
    '"(authorization|api[_-]?key|cookie|password|secret|token|next_cursor)"[[:space:]]*:' \
    "$twse_fixture_root"; then
    echo "TWSE fixture contains a forbidden field" >&2
    exit 1
fi

echo "TWSE fixture secret scan: passed"
