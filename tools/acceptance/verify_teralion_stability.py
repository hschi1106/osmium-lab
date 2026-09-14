#!/usr/bin/env python3
"""Certify Teralion TWSE/TPEx stability mapping against a published partition.

Generic partition integrity is delegated to ``source_partition`` and lifecycle
validation to ``stability_lifecycle``. This module owns only Teralion wire-field
interpretation and provider-specific identity checks.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

from source_partition import (
    PartitionVerificationError,
    VerifiedSourcePartition,
    load_published_partition,
)
from stability_lifecycle import (
    LifecycleVerificationError,
    StabilityObservation,
    StabilityPhase,
    find_stability_lifecycle,
)


REGULAR_FORMATS = {"STOCK_REALTIME", "STOCK_SNAPSHOT"}


class TeralionMappingError(Exception):
    pass


def _parse_time(value: Any) -> datetime:
    if not isinstance(value, str):
        raise TeralionMappingError("tick match_time is missing or not text")
    try:
        result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise TeralionMappingError("tick match_time is not ISO-8601") from error
    if result.tzinfo is None or result.utcoffset() is None:
        raise TeralionMappingError("tick match_time must include a UTC offset")
    return result


def _integer(value: Any) -> int | None:
    return value if type(value) is int else None


def _deal_quantity(record: dict[str, Any]) -> int | None:
    deal = record.get("deal")
    if deal is None:
        return None
    if not isinstance(deal, dict):
        raise TeralionMappingError("quote deal must be an object or null")
    return _integer(deal.get("quantity"))


def _valid_book_shape(record: dict[str, Any]) -> bool:
    bids = record.get("bids")
    asks = record.get("asks")
    return (
        isinstance(bids, list)
        and isinstance(asks, list)
        and len(bids) <= 5
        and len(asks) <= 5
        and all(isinstance(level, dict) for level in bids + asks)
    )


def _evidence_digest(record: dict[str, Any]) -> str:
    canonical = json.dumps(
        record,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def classify_record(record: dict[str, Any]) -> StabilityObservation | None:
    """Map one supported Teralion quote into provider-neutral lifecycle evidence."""
    if record.get("type") != "quote" or record.get("format") not in REGULAR_FORMATS:
        return None
    status = _integer(record.get("status_flags"))
    limits = _integer(record.get("limit_flags"))
    volume = _integer(record.get("cum_volume"))
    if status is None or limits is None or volume is None or volume < 0:
        return None
    quantity = _deal_quantity(record)
    match_time = _parse_time(record.get("match_time"))
    stream = str(record["format"])
    common = {
        "stream": stream,
        "match_time": match_time,
        "cumulative_volume": volume,
        "evidence_digest": _evidence_digest(record),
    }
    trend = limits & 0x03
    if (
        status & 0x80 == 0
        and status & 0x10 != 0
        and trend in {0x01, 0x02}
        and quantity == 0
        and record.get("bids") == []
        and record.get("asks") == []
    ):
        return StabilityObservation(
            **common,
            phase=StabilityPhase.TRIGGER,
            direction="down" if trend == 0x01 else "up",
        )
    if status & 0x80 != 0:
        book_has_value = (
            _valid_book_shape(record)
            and isinstance(record.get("bids"), list)
            and isinstance(record.get("asks"), list)
            and bool(record["bids"] or record["asks"])
        )
        return StabilityObservation(
            **common,
            phase=StabilityPhase.INDICATIVE,
            has_indicative_value=quantity is not None and quantity > 0 and book_has_value,
        )
    if quantity is None or quantity <= 0:
        return None
    return StabilityObservation(
        **common,
        phase=(
            StabilityPhase.CONTINUOUS
            if status & 0x10 != 0
            else StabilityPhase.AUCTION_RESULT
        ),
    )


def _partition_identity(partition: VerifiedSourcePartition) -> tuple[str, str, str]:
    try:
        market, trading_date, symbol = partition.root.parts[-3:]
    except ValueError as error:
        raise TeralionMappingError(
            "partition path must end in <market>/<date>/<symbol>"
        ) from error
    return market, trading_date, symbol


def verify_partition(partition_root: Path, expected_market: str) -> dict[str, Any]:
    partition = load_published_partition(partition_root)
    market, trading_date, symbol = _partition_identity(partition)
    if market != expected_market:
        raise TeralionMappingError(
            f"partition market mismatch: expected {expected_market}, got {market}"
        )
    observations: list[StabilityObservation] = []
    for record in partition.records:
        if record.get("market") != market or record.get("symbol") != symbol:
            raise TeralionMappingError(
                "Teralion tick identity differs from its source partition"
            )
        if record.get("type") == "quote":
            match_time = _parse_time(record.get("match_time"))
            if match_time.date().isoformat() != trading_date:
                raise TeralionMappingError(
                    "Teralion quote date differs from its source partition"
                )
        observation = classify_record(record)
        if observation is not None:
            observations.append(observation)

    sequences = {}
    for format_name in sorted(REGULAR_FORMATS):
        format_observations = [
            observation
            for observation in observations
            if observation.stream == format_name
        ]
        if format_observations:
            sequences[format_name] = find_stability_lifecycle(format_observations)
    if set(sequences) != REGULAR_FORMATS:
        missing = ", ".join(sorted(REGULAR_FORMATS - set(sequences)))
        raise TeralionMappingError(
            f"Teralion stability mapping evidence missing for format(s): {missing}"
        )
    return {
        "market": market,
        "trading_date": trading_date,
        "symbol": symbol,
        "revision_identity": partition.revision_identity,
        "page_count": partition.page_count,
        "tick_record_count": partition.tick_record_count,
        "stability_sequences": sequences,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Certify the current Teralion TWSE/TPEx stability mapping. "
            "Only sanitized identity, timestamps, counts, and hashes are printed."
        )
    )
    parser.add_argument("--twse-partition", type=Path, required=True)
    parser.add_argument("--tpex-partition", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = {
            "report_version": 2,
            "provider": "teralion",
            "twse": verify_partition(args.twse_partition, "twse"),
            "tpex": verify_partition(args.tpex_partition, "tpex"),
        }
    except (
        OSError,
        PartitionVerificationError,
        LifecycleVerificationError,
        TeralionMappingError,
        json.JSONDecodeError,
    ) as error:
        print(f"Teralion stability mapping verification failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
