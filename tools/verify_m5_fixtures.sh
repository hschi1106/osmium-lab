#!/bin/sh

set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
write_goldens=false
if [ "$#" -gt 0 ] && [ "$1" = "--write-goldens" ]; then
    write_goldens=true
elif [ "$#" -ne 0 ]; then
    echo "usage: tools/verify_m5_fixtures.sh [--write-goldens]" >&2
    exit 2
fi

command -v ruby >/dev/null 2>&1 || {
    echo "required command is unavailable: ruby" >&2
    exit 2
}
command -v rg >/dev/null 2>&1 || {
    echo "required command is unavailable: rg" >&2
    exit 2
}

ruby -ryaml -rjson -rdigest -rfileutils -rdate -e '
root, write_goldens = ARGV

fixtures = [
  {
    path: File.join(root, "fixtures/teralion/tpex/72328U/2026-07-20"),
    market: "tpex", source_market: "tpex", symbol: "72328U", date: "2026-07-20",
    kind: "warrant", records: 11,
    formats: {"WARRANT_REALTIME" => 4, "WARRANT_SNAPSHOT" => 7}
  },
  {
    path: File.join(root, "fixtures/teralion/twse/03003T/2026-07-20"),
    market: "twse", source_market: "twse", symbol: "03003T", date: "2026-07-20",
    kind: "warrant", records: 111,
    formats: {"WARRANT_REALTIME" => 60, "WARRANT_SNAPSHOT" => 51}
  },
  {
    path: File.join(root, "fixtures/teralion/taifex/TXFH6/2026-07-28"),
    market: "taifex", source_market: "taifex_fut", symbol: "TXFH6", date: "2026-07-28",
    kind: "future", records: 515258, formats: nil
  },
  {
    path: File.join(root, "fixtures/teralion/taifex/TXO24000U6/2026-07-28"),
    market: "taifex", source_market: "taifex_opt", symbol: "TXO24000U6", date: "2026-07-28",
    kind: "option", records: 540,
    formats: {
      "I020" => 2, "I021" => 2, "I022" => 177, "I023" => 3,
      "I030" => 68, "I070" => 4, "I072" => 22, "I080" => 85, "I082" => 177
    }
  }
]

def abort_with(message)
  abort("M5 fixture verification failed: #{message}")
end

fixtures.each do |fixture|
  path = fixture.fetch(:path)
  metadata_path = File.join(path, "metadata.yaml")
  daily_path = File.join(path, "daily.json")
  abort_with("#{path}: metadata is missing") unless File.file?(metadata_path)
  abort_with("#{path}: daily is missing") unless File.file?(daily_path)
  metadata = YAML.safe_load(File.read(metadata_path), permitted_classes: [Date], aliases: false)
  abort_with("#{path}: redistribution scope is not private") unless metadata["redistribution"] == "private-internal-review-only"
  abort_with("#{path}: metadata market mismatch") unless metadata["market"] == fixture[:market]
  abort_with("#{path}: metadata source market mismatch") unless metadata["source_market"] == fixture[:source_market]
  abort_with("#{path}: metadata symbol mismatch") unless metadata["symbol"] == fixture[:symbol]
  abort_with("#{path}: metadata date mismatch") unless metadata["trading_date"].to_s == fixture[:date]
  abort_with("#{path}: metadata kind mismatch") unless metadata["instrument_kind"] == fixture[:kind]

  daily = JSON.parse(File.read(daily_path))
  abort_with("#{daily_path}: source identity mismatch") unless
    [daily["market"], daily["symbol"], daily["trading_date"]] ==
    [fixture[:source_market], fixture[:symbol], fixture[:date]]

  files = Dir.glob(File.join(path, "**/*.jsonl")).sort
  abort_with("#{path}: no JSONL shards") if files.empty?
  counts = Hash.new(0)
  records = 0
  files.each do |file|
    File.foreach(file) do |line|
      record = JSON.parse(line)
      abort_with("#{file}: market mismatch") unless record["market"] == fixture[:source_market]
      abort_with("#{file}: symbol mismatch") unless record["symbol"] == fixture[:symbol]
      abort_with("#{file}: missing match_time") unless record["match_time"].is_a?(String)
      abort_with("#{file}: missing received_at") unless record["received_at"].is_a?(String)
      counts[record.fetch("format")] += 1
      records += 1
    end
  end
  abort_with("#{path}: record count #{records}, expected #{fixture[:records]}") unless records == fixture[:records]
  if fixture[:formats]
    abort_with("#{path}: format counts #{counts.inspect}, expected #{fixture[:formats].inspect}") unless counts == fixture[:formats]
  end

  artifact = files.map { |file| File.binread(file) }.join
  digest = Digest::SHA256.hexdigest(artifact)
  golden_path = File.join(path, "golden/fixture-set.sha256")
  if write_goldens == "true"
    FileUtils.mkdir_p(File.dirname(golden_path))
    File.write(golden_path, "#{digest}\n")
  else
    abort_with("#{golden_path}: missing") unless File.file?(golden_path)
    abort_with("#{golden_path}: checksum mismatch") unless File.read(golden_path).strip == digest
  end
  puts "#{fixture[:kind]} #{fixture[:symbol]}: records=#{records} formats=#{counts.inspect} sha256=#{digest}"
end
' "$root" "$write_goldens"

if rg -n -i \
    '"(authorization|api[_-]?key|cookie|password|secret|token|next_cursor)"[[:space:]]*:' \
    "$root/fixtures/teralion/twse/03003T/2026-07-20" \
    "$root/fixtures/teralion/tpex/72328U/2026-07-20" \
    "$root/fixtures/teralion/taifex/TXFH6/2026-07-28" \
    "$root/fixtures/teralion/taifex/TXO24000U6/2026-07-28"; then
    echo "M5 fixture contains a forbidden field" >&2
    exit 1
fi

echo "M5 fixture integrity and secret scan: passed"
