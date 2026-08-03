#!/usr/bin/env python3
"""Build small, deterministic representative slices from Teralion fixtures."""

from __future__ import annotations

import argparse
import json
import shutil
from collections import defaultdict
from pathlib import Path


PROFILES = {
    ("taifex", "CAFH6", "2026-07-20"): "stock_futures_regular_only",
    ("taifex", "CDFH6", "2026-07-20"): "stock_futures_regular_and_after_hours",
    ("taifex", "TXFH6", "2026-07-20"): "index_futures_cross_day_boundary",
    ("taifex", "TXFH6", "2026-07-28"): "index_futures_with_option_underlying",
    ("taifex", "TXO24000U6", "2026-07-28"): "index_option_cross_session",
    ("tpex", "6488", "2026-07-20"): "equity_regular",
    ("tpex", "72328U", "2026-07-20"): "warrant_regular",
    ("twse", "03003T", "2026-07-20"): "warrant_regular",
    ("twse", "2330", "2026-07-20"): "equity_regular",
}

MAX_RECORDS_PER_SESSION = 512
RECORDS_PER_GROUP = 8


def choose_indices(records: list[dict]) -> list[int]:
    selected: set[int] = set()

    def add(index: int) -> None:
        if 0 <= index < len(records):
            selected.add(index)

    for index in range(min(16, len(records))):
        add(index)
    for index in range(max(0, len(records) - 16), len(records)):
        add(index)

    grouped: dict[tuple[str, object], list[int]] = defaultdict(list)
    for index, record in enumerate(records):
        grouped[("format", record.get("format"))].append(index)
        grouped[("type", record.get("type"))].append(index)
        grouped[("status", record.get("status_flags"))].append(index)
        grouped[("limit", record.get("limit_flags"))].append(index)
        if "first_packet" in record:
            grouped[("first_packet", record["first_packet"])].append(index)

    for indices in grouped.values():
        if not indices:
            continue
        if len(indices) <= RECORDS_PER_GROUP:
            chosen = indices
        else:
            positions = [
                round(position * (len(indices) - 1) / (RECORDS_PER_GROUP - 1))
                for position in range(RECORDS_PER_GROUP)
            ]
            chosen = [indices[position] for position in positions]
        for index in chosen:
            add(index)

    if len(selected) > MAX_RECORDS_PER_SESSION:
        ordered = sorted(selected)
        positions = [
            round(position * (len(ordered) - 1) / (MAX_RECORDS_PER_SESSION - 1))
            for position in range(MAX_RECORDS_PER_SESSION)
        ]
        selected = {ordered[position] for position in positions}

    return sorted(selected)


def load_records(session: Path) -> list[dict]:
    records: list[dict] = []
    for path in sorted(session.glob("*.jsonl")):
        with path.open(encoding="utf-8") as handle:
            for line in handle:
                records.append(json.loads(line))
    if not records:
        raise RuntimeError(f"session has no JSONL records: {session}")
    return records


def yaml_scalar(value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "null"
    return json.dumps(str(value), ensure_ascii=False)


def write_metadata(destination: Path, market: str, symbol: str, date: str, profile: str,
                   session_stats: list[dict]) -> None:
    formats = sorted({fmt for item in session_stats for fmt in item["formats"]})
    sessions = [item["session"] for item in session_stats]
    total_records = sum(item["record_count"] for item in session_stats)
    total_bytes = sum(item["byte_count"] for item in session_stats)
    lines = [
        "metadata_version: 2",
        "fixture_scope: representative_slice",
        "complete_day: false",
        f"market: {yaml_scalar(market)}",
        f"symbol: {yaml_scalar(symbol)}",
        f"trading_date: {yaml_scalar(date)}",
        f"profile: {yaml_scalar(profile)}",
        "selection_policy: deterministic format/status/session-boundary sample",
        "sessions:",
    ]
    lines.extend(f"  - {yaml_scalar(session)}" for session in sessions)
    lines.extend([
        "source_formats:",
    ])
    lines.extend(f"  - {yaml_scalar(fmt)}" for fmt in formats)
    lines.extend([
        "artifact:",
        f"  file_count: {len(session_stats)}",
        f"  record_count: {total_records}",
        f"  byte_count: {total_bytes}",
        "  full_day: false",
        "  source_provenance: retained from the internal Teralion acceptance fixture",
        "sessions_detail:",
    ])
    for item in session_stats:
        lines.extend([
            f"  - session: {yaml_scalar(item['session'])}",
            f"    record_count: {item['record_count']}",
            f"    byte_count: {item['byte_count']}",
            f"    source_record_count: {item['source_record_count']}",
            "    formats:",
        ])
        lines.extend(f"      - {yaml_scalar(fmt)}" for fmt in item["formats"])
    (destination / "metadata.yaml").write_text("\n".join(lines) + "\n", encoding="utf-8")


def compact_profile(source: Path, destination: Path, market: str, symbol: str, date: str,
                    profile: str) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    daily = source / "daily.json"
    if daily.is_file():
        shutil.copy2(daily, destination / "daily.json")

    session_stats = []
    for session in sorted(path for path in source.iterdir() if path.is_dir() and path.name != "golden"):
        records = load_records(session)
        indices = choose_indices(records)
        selected = [records[index] for index in indices]
        output_session = destination / session.name
        output_session.mkdir()
        output_file = output_session / "0001.jsonl"
        with output_file.open("w", encoding="utf-8") as handle:
            for record in selected:
                handle.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n")
        session_stats.append({
            "session": session.name,
            "record_count": len(selected),
            "byte_count": output_file.stat().st_size,
            "source_record_count": len(records),
            "formats": sorted({str(record.get("format")) for record in selected}),
        })

    if not session_stats:
        raise RuntimeError(f"profile has no sessions: {source}")
    write_metadata(destination, market, symbol, date, profile, session_stats)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source = args.source.resolve()
    output = args.output.resolve()
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    for (market, symbol, date), profile in PROFILES.items():
        source_profile = source / market / symbol / date
        if not source_profile.is_dir():
            raise RuntimeError(f"selected fixture profile is missing: {source_profile}")
        compact_profile(
            source_profile,
            output / market / symbol / date,
            market,
            symbol,
            date,
            profile,
        )


if __name__ == "__main__":
    main()
