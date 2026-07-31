#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
fixture="$root/fixtures/teralion/tpex/6488/2026-07-20"

ruby -ryaml -rjson -rdigest -e '
fixture = ARGV.fetch(0)
metadata_path = File.join(fixture, "metadata.yaml")
metadata = YAML.safe_load(File.read(metadata_path), permitted_classes: [], aliases: false)
abort("market mismatch") unless metadata.fetch("market") == "tpex"
abort("symbol mismatch") unless metadata.fetch("symbol") == "6488"
abort("trading date mismatch") unless metadata.fetch("exchange_trading_date") == "2026-07-20"

daily_path = File.join(fixture, "daily.json")
daily_bytes = File.binread(daily_path)
source = metadata.fetch("source_acquisition")
abort("daily checksum mismatch") unless Digest::SHA256.hexdigest(daily_bytes) == source.fetch("daily_instrument_sha256")
daily = JSON.parse(daily_bytes)
abort("daily identity mismatch") unless [daily.fetch("market"), daily.fetch("symbol"), daily.fetch("trading_date")] == ["tpex", "6488", "2026-07-20"]

segment = metadata.fetch("segments").fetch("regular")
shards = segment.fetch("shards")
abort("shard count mismatch") unless shards.length == 17
shards.each do |shard|
  path = File.join(fixture, shard.fetch("path"))
  bytes = File.binread(path)
  abort("#{path}: byte count mismatch") unless bytes.bytesize == shard.fetch("bytes")
  abort("#{path}: record count mismatch") unless bytes.count("\n") == shard.fetch("records")
  abort("#{path}: checksum mismatch") unless Digest::SHA256.hexdigest(bytes) == shard.fetch("sha256")
end

bytes = shards.sort_by { |shard| shard.fetch("path") }.map { |shard| File.binread(File.join(fixture, shard.fetch("path"))) }.join
artifact = metadata.fetch("artifact")
abort("artifact byte count mismatch") unless bytes.bytesize == artifact.fetch("byte_count")
abort("artifact record count mismatch") unless bytes.count("\n") == artifact.fetch("record_count")
abort("artifact checksum mismatch") unless Digest::SHA256.hexdigest(bytes) == artifact.fetch("sha256")
golden = File.read(File.join(fixture, "golden/fixture-set.sha256")).strip
abort("golden checksum mismatch") unless golden == artifact.fetch("sha256")
puts "M4 fixture integrity: passed"
puts "shards=#{shards.length} records=#{bytes.count("\n")} sha256=#{artifact.fetch("sha256")}"
' "$fixture"

if rg -n -i \
    '"(authorization|api[_-]?key|cookie|password|secret|token|next_cursor)"[[:space:]]*:' \
    "$fixture"; then
    echo "M4 fixture contains a forbidden field" >&2
    exit 1
fi

echo "M4 fixture secret scan: passed"
