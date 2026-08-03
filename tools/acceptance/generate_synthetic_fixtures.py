#!/usr/bin/env python3
"""Generate repository-owned synthetic Teralion-wire fixtures.

The emitted records are authored test scenarios. They are not sampled, transformed,
or replayed from any market-data feed.
"""

from __future__ import annotations

import hashlib
import json
import shutil
from dataclasses import dataclass
from pathlib import Path


DATE = "2026-07-20"


@dataclass(frozen=True)
class Profile:
    market: str
    source_market: str
    symbol: str
    kind: str
    sessions: dict[str, list[dict]]


def quote(
    market: str,
    symbol: str,
    wire_format: str,
    time: str,
    price: float,
    *,
    status: int = 0x10,
    intermediate: bool = False,
    cumulative_volume: int = 100,
) -> dict:
    bids = [] if intermediate else [
        {"price": price - 1, "quantity": 7},
        {"price": price - 2, "quantity": 9},
    ]
    asks = [] if intermediate else [
        {"price": price, "quantity": 6},
        {"price": price + 1, "quantity": 8},
    ]
    return {
        "asks": asks,
        "bids": bids,
        "cum_volume": cumulative_volume,
        "deal": {"price": price, "quantity": 1},
        "format": wire_format,
        "intermediate_print": intermediate,
        "limit_flags": 0,
        "market": market,
        "match_time": f"{DATE}T{time}+08:00",
        "received_at": f"{DATE}T{time}.001000+08:00" if "." not in time else f"{DATE}T{time}+08:00",
        "status_flags": status,
        "symbol": symbol,
        "type": "quote",
    }


def taifex_record(
    symbol: str,
    source_market: str,
    wire_format: str,
    record_type: str,
    match_time: str,
    **payload: object,
) -> dict:
    return {
        "format": wire_format,
        "market": source_market,
        "match_time": match_time,
        "received_at": match_time.replace("+08:00", ".001000+08:00"),
        "symbol": symbol,
        "type": record_type,
        **payload,
    }


def quote_profiles() -> list[Profile]:
    profiles = []
    for market in ("twse", "tpex"):
        equity = f"SYNTH-{market.upper()}-EQ"
        profiles.append(Profile(
            market,
            market,
            equity,
            "equity",
            {"regular-quotes": [
                quote(market, equity, "STOCK_SNAPSHOT", "08:59:00", 100, status=0x80, cumulative_volume=0),
                quote(market, equity, "STOCK_SNAPSHOT", "09:00:00", 100, cumulative_volume=100),
                quote(market, equity, "STOCK_REALTIME", "09:00:01", 100.5, intermediate=True, cumulative_volume=101),
                quote(market, equity, "STOCK_REALTIME", "09:00:01", 101, cumulative_volume=102),
                quote(market, equity, "STOCK_SNAPSHOT", "13:29:00", 102, status=0x80, cumulative_volume=200),
            ]},
        ))
        warrant = f"SYNTH-{market.upper()}-W"
        profiles.append(Profile(
            market,
            market,
            warrant,
            "warrant",
            {"regular-quotes": [
                quote(market, warrant, "WARRANT_REALTIME", "09:01:00", 5, cumulative_volume=10),
                quote(market, warrant, "WARRANT_SNAPSHOT", "08:59:00", 4.9, status=0x80, cumulative_volume=0),
                quote(market, warrant, "WARRANT_REALTIME", "13:29:00", 5.1, status=0x80, cumulative_volume=20),
            ]},
        ))
    return profiles


def futures_profile() -> Profile:
    symbol = "SYNTH-FUT"
    market = "taifex_fut"
    records = [
        taifex_record(symbol, market, "I022", "trade", f"{DATE}T08:40:00+08:00", first_packet=True, trades=[{"price": 0, "quantity": 0}]),
        taifex_record(symbol, market, "I020", "trade", f"{DATE}T09:00:00+08:00", first_packet=True, trades=[{"price": 200, "quantity": 2}]),
        taifex_record(symbol, market, "I080", "book", f"{DATE}T09:00:01+08:00", bids=[{"price": 199, "quantity": 3}], asks=[{"price": 201, "quantity": 4}]),
        taifex_record(symbol, market, "I030", "stats", f"{DATE}T09:00:02+08:00"),
    ]
    return Profile("taifex", market, symbol, "future", {"regular": records})


def option_profile() -> Profile:
    symbol = "SYNTH-OPT"
    market = "taifex_opt"
    after_hours = [
        taifex_record(symbol, market, "I072", "close", "2026-07-17T15:00:00+08:00"),
        taifex_record(symbol, market, "I020", "trade", "2026-07-17T15:01:00+08:00", first_packet=True, trades=[{"price": 12, "quantity": 1}]),
        taifex_record(symbol, market, "I080", "book", "2026-07-17T15:01:01+08:00", bids=[{"price": 11, "quantity": 2}], asks=[{"price": 13, "quantity": 2}]),
    ]
    regular = [
        taifex_record(symbol, market, "I022", "trade", f"{DATE}T08:40:00+08:00", first_packet=True, trades=[{"price": 0, "quantity": 0}]),
        taifex_record(symbol, market, "I020", "trade", f"{DATE}T09:00:00+08:00", first_packet=True, trades=[{"price": 14, "quantity": 1}]),
        taifex_record(symbol, market, "I082", "book", f"{DATE}T09:00:01+08:00", bids=[{"price": 13, "quantity": 3}], asks=[{"price": 15, "quantity": 3}]),
        taifex_record(symbol, market, "I021", "trade", f"{DATE}T09:00:02+08:00"),
    ]
    return Profile("taifex", market, symbol, "option", {"after-hours": after_hours, "regular": regular})


def encode(record: dict) -> bytes:
    return (json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def daily(profile: Profile) -> dict:
    return {
        "symbol": profile.symbol,
        "market": profile.source_market,
        "exchange": profile.market.upper(),
        "name": "repository-owned synthetic fixture",
        "root": "SYNTH",
        "kind": profile.kind,
        "underlying": None,
        "call_put": "C" if profile.kind == "option" else None,
        "strike": 100 if profile.kind == "option" else None,
        "expiry": "2099-12" if profile.kind in {"future", "option"} else None,
        "multiplier": None,
        "currency": "TWD",
        "trading_date": DATE,
        "session": {"reference": 100, "rise_limit": [], "fall_limit": []},
    }


def write_profile(root: Path, profile: Profile) -> tuple[str, int, int]:
    destination = root / profile.market / profile.symbol / DATE
    destination.mkdir(parents=True)
    (destination / "daily.json").write_text(
        json.dumps(daily(profile), ensure_ascii=False, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    digest = hashlib.sha256()
    total_records = 0
    total_bytes = 0
    formats: set[str] = set()
    session_details = []
    for session, records in profile.sessions.items():
        session_dir = destination / session
        session_dir.mkdir()
        payload = b"".join(encode(record) for record in records)
        (session_dir / "0001.jsonl").write_bytes(payload)
        digest.update(payload)
        total_records += len(records)
        total_bytes += len(payload)
        session_formats = sorted({str(record["format"]) for record in records})
        formats.update(session_formats)
        session_details.append((session, len(records), len(payload), session_formats))
    golden = destination / "golden"
    golden.mkdir()
    checksum = digest.hexdigest()
    (golden / "fixture-set.sha256").write_text(checksum + "\n", encoding="utf-8")
    lines = [
        "metadata_version: 3",
        "fixture_scope: synthetic_scenario",
        "complete_day: false",
        f'market: "{profile.market}"',
        f'symbol: "{profile.symbol}"',
        f'trading_date: "{DATE}"',
        f'profile: "{profile.kind}"',
        "provenance: repository-owned-synthetic",
        "generation_policy: authored deterministic scenarios; no market records used",
        "sessions:",
        *[f'  - "{session}"' for session in profile.sessions],
        "source_formats:",
        *[f'  - "{wire_format}"' for wire_format in sorted(formats)],
        "artifact:",
        f"  file_count: {len(profile.sessions)}",
        f"  record_count: {total_records}",
        f"  byte_count: {total_bytes}",
        "  full_day: false",
        "  source_provenance: repository-owned-synthetic",
        "sessions_detail:",
    ]
    for session, records, byte_count, session_formats in session_details:
        lines.extend([
            f'  - session: "{session}"',
            f"    record_count: {records}",
            f"    byte_count: {byte_count}",
            "    formats:",
            *[f'      - "{wire_format}"' for wire_format in session_formats],
        ])
    (destination / "metadata.yaml").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return checksum, total_records, total_bytes


def manifest_entry(profile: Profile, checksum: str, records: int) -> list[str]:
    sessions = ", ".join(profile.sessions)
    lines = [
        f"  - id: synthetic-{profile.market}-{profile.kind}",
        f"    path: fixtures/teralion/{profile.market}/{profile.symbol}/{DATE}",
        f"    market: {profile.market}",
    ]
    if profile.source_market != profile.market:
        lines.append(f"    source_market: {profile.source_market}")
    lines.extend([
        f"    instrument_kind: {profile.kind}",
        f'    symbol: "{profile.symbol}"',
        f'    trading_date: "{DATE}"',
        f"    sessions: [{sessions}]",
        "    complete_day: false",
        f"    record_count: {records}",
        f"    fixture_set_sha256: {checksum}",
        "    redistribution: synthetic-redistributable",
    ])
    return lines


def write_smoke(repository: Path) -> None:
    root = repository / "fixtures/smoke/teralion"
    if root.exists():
        shutil.rmtree(root)
    symbol = "SYNTH-SMOKE"
    profile = Profile("twse", "twse", symbol, "equity", {"regular-quotes": [
        quote("twse", symbol, "STOCK_SNAPSHOT", "09:00:00", 100, cumulative_volume=10),
        quote("twse", symbol, "STOCK_REALTIME", "09:00:01", 101, cumulative_volume=11),
    ]})
    checksum, records, _ = write_profile(root, profile)
    manifest = [
        "bundle_format_version: 1",
        "bundle_id: osmium-synthetic-smoke-v2",
        "distribution_scope: synthetic-redistributable",
        "authorization:",
        "  required: false",
        "  transport: local",
        "payload_policy: repository-owned synthetic scenarios generated without market records",
        "entries:",
        "  - id: synthetic-smoke-twse-equity",
        f"    path: fixtures/smoke/teralion/twse/{symbol}/{DATE}",
        "    market: twse",
        "    instrument_kind: equity",
        f'    symbol: "{symbol}"',
        f'    trading_date: "{DATE}"',
        "    session: regular",
        f"    record_count: {records}",
        f"    fixture_set_sha256: {checksum}",
        "    redistribution: synthetic-redistributable",
    ]
    (repository / "fixtures/smoke/manifest.yaml").write_text("\n".join(manifest) + "\n", encoding="utf-8")


def main() -> None:
    repository = Path(__file__).resolve().parents[2]
    root = repository / "fixtures/teralion"
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    profiles = [*quote_profiles(), futures_profile(), option_profile()]
    entries: list[str] = []
    for profile in profiles:
        checksum, records, _ = write_profile(root, profile)
        entries.extend(manifest_entry(profile, checksum, records))
    (root / "README.md").write_text(
        "# Synthetic Teralion-wire fixtures\n\n"
        "此目錄完全由 `tools/acceptance/generate_synthetic_fixtures.py` 產生。內容是 repository-owned "
        "測試情境，不包含、抽樣或轉換任何真實市場行情。\n",
        encoding="utf-8",
    )
    manifest = [
        "manifest_version: 2",
        "bundle_format_version: 1",
        "bundle_id: osmium-public-synthetic-fixtures-v1",
        "distribution_scope: synthetic-redistributable",
        "authorization:",
        "  required: false",
        "  transport: local",
        "payload_policy: repository-owned synthetic scenarios generated without market records",
        "entries:",
        *entries,
    ]
    (repository / "fixtures/acceptance/manifest.yaml").write_text("\n".join(manifest) + "\n", encoding="utf-8")
    write_smoke(repository)


if __name__ == "__main__":
    main()
