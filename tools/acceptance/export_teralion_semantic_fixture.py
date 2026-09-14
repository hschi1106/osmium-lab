#!/usr/bin/env python3
"""Export a bounded, provider-neutral stability sequence from Teralion data.

The output is deterministic JSON, which is also valid YAML. Raw provider
records are verified and interpreted here; consumers only receive semantic
observations and audit metadata.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timedelta
from decimal import Decimal, InvalidOperation, localcontext
from pathlib import Path
from typing import Any

from source_partition import (
    PartitionVerificationError,
    iter_published_records,
    open_published_partition,
)
from stability_lifecycle import LifecycleVerificationError, StabilityPhase, find_stability_lifecycle
from verify_teralion_stability import (
    REGULAR_FORMATS,
    TeralionMappingError,
    _evidence_digest,
    _parse_time,
    _partition_identity,
    _valid_book_shape,
    classify_record,
)


SCHEMA_VERSION = 1


def _offset_us(match_time: datetime, base_time: datetime) -> int:
    delta = match_time - base_time
    return (
        delta.days * 86_400_000_000
        + delta.seconds * 1_000_000
        + delta.microseconds
    )


def _decimal(value: Any, field: str) -> Decimal:
    if isinstance(value, bool) or not isinstance(value, (Decimal, int, str)):
        raise TeralionMappingError(f"{field} is not numeric")
    try:
        result = Decimal(str(value))
    except InvalidOperation as error:
        raise TeralionMappingError(f"{field} is not numeric") from error
    if not result.is_finite():
        raise TeralionMappingError(f"{field} is not finite")
    return result


def _add_decimal(left: Decimal, right: Decimal) -> Decimal:
    fractional_digits = max(-left.as_tuple().exponent, -right.as_tuple().exponent, 0)
    integer_digits = max(left.adjusted(), right.adjusted(), 0) + 1
    with localcontext() as context:
        context.prec = integer_digits + fractional_digits + 2
        return left + right


def _json_decimal(value: Decimal) -> str:
    text = format(value, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return "0" if text in {"-0", ""} else text


def _quantity(value: Any, field: str, scale: int) -> int:
    if type(value) is not int or value < 0:
        raise TeralionMappingError(f"{field} is not a non-negative integer")
    return value * scale


def _book_side(
    record: dict[str, Any], side: str, price_delta: Decimal, quantity_scale: int
) -> tuple[list[dict[str, int | str]], int | None]:
    raw = record.get(side)
    if not isinstance(raw, list) or len(raw) > 5:
        raise TeralionMappingError(f"quote {side} must contain at most five levels")
    levels: list[dict[str, int | str]] = []
    market_quantity = None
    for index, item in enumerate(raw):
        if not isinstance(item, dict):
            raise TeralionMappingError(f"quote {side} contains a non-object level")
        price = _decimal(item.get("price"), f"{side} price")
        quantity = _quantity(item.get("quantity"), f"{side} quantity", quantity_scale)
        if price == 0:
            if index != 0 or market_quantity is not None:
                raise TeralionMappingError(
                    "zero-price market-order level must be the first book entry"
                )
            market_quantity = quantity
        else:
            levels.append({"price": _json_decimal(_add_decimal(price, price_delta)), "quantity": quantity})
    return levels, market_quantity


def _has_market_order_quantity(record: dict[str, Any]) -> bool:
    for side in ("bids", "asks"):
        levels = record.get(side)
        if (
            isinstance(levels, list)
            and levels
            and isinstance(levels[0], dict)
            and _decimal(levels[0].get("price"), f"{side} price") == 0
            and type(levels[0].get("quantity")) is int
        ):
            return True
    return False


def _semantic_observation(
    phase: str,
    record: dict[str, Any],
    base_time: datetime,
    price_delta: Decimal,
    quantity_scale: int,
    direction: str | None = None,
) -> dict[str, Any]:
    match_time = _parse_time(record.get("match_time"))
    deal = record.get("deal")
    if deal is not None and not isinstance(deal, dict):
        raise TeralionMappingError("quote deal must be an object or null")
    raw_quantity = None if deal is None else deal.get("quantity")
    has_trade = type(raw_quantity) is int and raw_quantity > 0
    bids, market_bid = _book_side(record, "bids", price_delta, quantity_scale)
    asks, market_ask = _book_side(record, "asks", price_delta, quantity_scale)
    result = {
        "phase": phase,
        "offset_us": _offset_us(match_time, base_time),
        "price": (
            _json_decimal(
                _add_decimal(_decimal(deal.get("price"), "deal price"), price_delta)
            )
            if has_trade
            else None
        ),
        "quantity": (
            _quantity(raw_quantity, "deal quantity", quantity_scale) if has_trade else None
        ),
        "cumulative_volume": _quantity(
            record.get("cum_volume"), "cum_volume", quantity_scale
        ),
        "bids": bids,
        "asks": asks,
        "market_bid_quantity": market_bid,
        "market_ask_quantity": market_ask,
        "direction": direction,
        "provenance": {
            "kind": "source_observation",
            "source_match_time": match_time.isoformat(),
            "source_record_exact_canonical_sha256": _evidence_digest(record),
        },
    }
    return result


def export_fixture(
    partition_root: Path,
    *,
    case_id: str,
    stream: str,
    target_market: str | None = None,
    target_trading_date: str | None = None,
    target_symbol: str | None = None,
    target_base_time: datetime | None = None,
    price_delta: Decimal = Decimal(0),
    quantity_scale: int = 1,
    max_classified_records: int = 20_000,
    expected_match_delay_us: int = 120_000_000,
    decision_lead_us: int = 1_000_000,
    order_intent_count: int = 2,
) -> dict[str, Any]:
    if stream not in REGULAR_FORMATS:
        raise TeralionMappingError(f"unsupported quote stream: {stream}")
    if not case_id:
        raise TeralionMappingError("case id must not be empty")
    if quantity_scale <= 0 or max_classified_records <= 0:
        raise TeralionMappingError("quantity scale and record bound must be positive")
    if expected_match_delay_us < 0 or not 0 <= decision_lead_us <= expected_match_delay_us:
        raise TeralionMappingError("expected timing parameters are invalid")
    if order_intent_count < 0:
        raise TeralionMappingError("expected order intent count is invalid")
    partition = open_published_partition(partition_root)
    market, trading_date, symbol = _partition_identity(partition)
    classified = []
    for record in iter_published_records(partition, parse_float=str):
        if record.get("market") != market or record.get("symbol") != symbol:
            raise TeralionMappingError("Teralion tick identity differs from its source partition")
        if record.get("type") == "quote" and _parse_time(record.get("match_time")).date().isoformat() != trading_date:
            raise TeralionMappingError("Teralion quote date differs from its source partition")
        observation = classify_record(record)
        if observation is not None and observation.stream == stream:
            classified.append(observation)
            if len(classified) > max_classified_records:
                raise TeralionMappingError("classified observation bound exceeded")
    lifecycle = find_stability_lifecycle(classified)
    trigger_at = datetime.fromisoformat(str(lifecycle["trigger_at"]))
    formal_at = datetime.fromisoformat(str(lifecycle["formal_result_at"]))
    resume_at = datetime.fromisoformat(str(lifecycle["resumed_continuous_at"]))

    firm: dict[str, Any] | None = None
    continuous_market: dict[str, Any] | None = None
    selected: list[tuple[str, dict[str, Any], str | None]] = []
    trigger_seen = formal_seen = resume_seen = False
    for record in iter_published_records(partition, parse_float=str):
        if record.get("type") != "quote" or record.get("format") != stream:
            continue
        match_time = _parse_time(record.get("match_time"))
        observation = classify_record(record)
        if (
            match_time < trigger_at
            and _valid_book_shape(record)
            and bool(record.get("bids") or record.get("asks"))
            and not (observation and observation.phase is StabilityPhase.INDICATIVE)
        ):
            if firm is None or match_time >= _parse_time(firm.get("match_time")):
                firm = record
        if observation is None:
            continue
        if not trigger_seen and match_time == trigger_at and observation.phase is StabilityPhase.TRIGGER:
            selected.append(("trigger", record, observation.direction))
            trigger_seen = True
        elif trigger_at < match_time < formal_at and observation.phase is StabilityPhase.INDICATIVE:
            selected.append(("trial", record, None))
        elif not formal_seen and match_time == formal_at and observation.phase is StabilityPhase.AUCTION_RESULT:
            selected.append(("auction_result", record, None))
            formal_seen = True
        elif not resume_seen and match_time == resume_at and observation.phase is StabilityPhase.CONTINUOUS:
            selected.append(("continuous", record, None))
            resume_seen = True
        elif (
            match_time > resume_at
            and observation.phase is StabilityPhase.CONTINUOUS
            and _has_market_order_quantity(record)
            and (
                continuous_market is None
                or match_time < _parse_time(continuous_market.get("match_time"))
            )
        ):
            continuous_market = record
    if firm is None or not (trigger_seen and formal_seen and resume_seen):
        raise TeralionMappingError("selected lifecycle source records are incomplete")
    if continuous_market is not None:
        selected.append(("continuous_market", continuous_market, None))

    source_base = _parse_time(firm.get("match_time"))
    target_base = target_base_time or source_base
    target_date = target_trading_date or target_base.date().isoformat()
    if target_base.date().isoformat() != target_date:
        raise TeralionMappingError(
            "target base time date differs from target trading date"
        )
    observations = [
        _semantic_observation("firm", firm, source_base, price_delta, quantity_scale)
    ] + [
        _semantic_observation(phase, record, source_base, price_delta, quantity_scale, direction)
        for phase, record, direction in selected
    ]
    trigger_offset = next(item["offset_us"] for item in observations if item["phase"] == "trigger")
    return {
        "schema_version": SCHEMA_VERSION,
        "case_id": case_id,
        "provenance": {
            "source_revision": partition.revision_identity,
            "market": market,
            "trading_date": trading_date,
            "symbol": symbol,
            "stream": stream,
            "digest_encoding": {
                "name": "canonical_json_numeric_lexemes_v1",
                "canonicalization": "sorted keys, compact separators, UTF-8",
                "numeric_decode": "non-integer JSON number lexemes become strings",
                "identity": "semantic canonical digest, not a raw-record byte checksum",
                "adapter_report_comparable": False,
            },
            "selector": {
                "lifecycle": "first sequence accepted by stability_lifecycle policy",
                "firm": "latest non-trial non-empty quote book before trigger",
                "trials": "all indicative observations strictly between trigger and formal result",
                "continuous": "first accepted continuous observation after formal result",
                "continuous_market": (
                    "first subsequent continuous observation carrying a non-null "
                    "market-order quantity, when present"
                ),
                "max_classified_records": max_classified_records,
            },
            "transforms": {
                "identity": {
                    "source": {"market": market, "trading_date": trading_date, "symbol": symbol},
                    "target": {
                        "market": target_market or market,
                        "trading_date": target_date,
                        "symbol": target_symbol or symbol,
                    },
                },
                "time": {
                    "operation": "translate_preserving_offset_us",
                    "source_base_match_time": source_base.isoformat(),
                    "target_base_match_time": target_base.isoformat(),
                },
                "price": {
                    "operation": "add_decimal",
                    "value": _json_decimal(price_delta),
                    "targets": ["price", "bids[*].price", "asks[*].price"],
                },
                "quantity": {
                    "operation": "multiply_integer",
                    "value": quantity_scale,
                    "targets": [
                        "quantity", "bids[*].quantity", "asks[*].quantity",
                        "market_bid_quantity", "market_ask_quantity", "cumulative_volume",
                    ],
                },
                "source_record_digest": {
                    "algorithm": "sha256",
                    "encoding": "canonical_json_numeric_lexemes_v1",
                },
            },
            "expected_policy": {
                "expected_match_offset_us": "trigger_offset_us + expected_match_delay_us",
                "decision_timer_offset_us": "expected_match_offset_us - decision_lead_us",
                "expected_match_delay_us": expected_match_delay_us,
                "decision_lead_us": decision_lead_us,
                "remaining_fields": "fixed semantic-differential policy constants",
            },
        },
        "observations": observations,
        "expected": {
            "trigger_offset_us": trigger_offset,
            "direction": lifecycle["direction"],
            "expected_match_offset_us": trigger_offset + expected_match_delay_us,
            "decision_timer_offset_us": trigger_offset + expected_match_delay_us - decision_lead_us,
            "timer_count_min": 1,
            "order_intent_count": order_intent_count,
            "legacy_trial_replaces_firm_book": True,
            "vnext_trial_replaces_firm_book": False,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Export a provider-neutral semantic fixture from one verified Teralion partition")
    parser.add_argument("--partition", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--case-id", required=True)
    parser.add_argument("--stream", choices=sorted(REGULAR_FORMATS), default="STOCK_REALTIME")
    parser.add_argument("--target-market")
    parser.add_argument("--target-trading-date")
    parser.add_argument("--target-symbol")
    parser.add_argument("--target-base-time", type=_parse_time)
    parser.add_argument("--price-delta", type=Decimal, default=Decimal(0))
    parser.add_argument("--quantity-scale", type=int, default=1)
    parser.add_argument("--max-classified-records", type=int, default=20_000)
    parser.add_argument("--expected-match-delay-us", type=int, default=120_000_000)
    parser.add_argument("--decision-lead-us", type=int, default=1_000_000)
    parser.add_argument("--order-intent-count", type=int, default=2)
    args = parser.parse_args()
    try:
        result = export_fixture(
            args.partition,
            case_id=args.case_id,
            stream=args.stream,
            target_market=args.target_market,
            target_trading_date=args.target_trading_date,
            target_symbol=args.target_symbol,
            target_base_time=args.target_base_time,
            price_delta=args.price_delta,
            quantity_scale=args.quantity_scale,
            max_classified_records=args.max_classified_records,
            expected_match_delay_us=args.expected_match_delay_us,
            decision_lead_us=args.decision_lead_us,
            order_intent_count=args.order_intent_count,
        )
        payload = json.dumps(result, sort_keys=True, indent=2) + "\n"
        if args.output:
            args.output.write_text(payload, encoding="utf-8")
        else:
            sys.stdout.write(payload)
    except (OSError, PartitionVerificationError, LifecycleVerificationError, TeralionMappingError) as error:
        print(f"Teralion semantic fixture export failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
