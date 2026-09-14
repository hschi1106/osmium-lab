#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import tempfile
import unittest
from datetime import datetime, timedelta
from decimal import Decimal
from pathlib import Path

from export_teralion_semantic_fixture import export_fixture
from verify_teralion_stability import TeralionMappingError


BASE = datetime.fromisoformat("2026-08-10T09:00:00+08:00")


def quote(
    seconds: float,
    status: int,
    limits: int,
    quantity: int | None,
    volume: int,
    *,
    market_order: bool = False,
):
    match_time = BASE + timedelta(seconds=seconds)
    has_book = quantity is None or quantity > 0
    return {
        "type": "quote",
        "market": "twse",
        "symbol": "3026",
        "format": "STOCK_REALTIME",
        "match_time": match_time.isoformat(),
        "received_at": match_time.isoformat(),
        "status_flags": status,
        "limit_flags": limits,
        "deal": None if quantity is None else {"price": 578, "quantity": quantity},
        "bids": (
            ([{"price": 0, "quantity": 7}] if market_order else [])
            + [
                {"price": 577, "quantity": 3},
                {"price": 576, "quantity": 6},
                {"price": 575, "quantity": 8},
                *([{"price": 574, "quantity": 9}] if not market_order else []),
            ]
            if has_book
            else []
        ),
        "asks": (
            [
                {"price": 579, "quantity": 4},
                {"price": 580, "quantity": 5},
                {"price": 581, "quantity": 10},
                {"price": 582, "quantity": 11},
                {"price": 583, "quantity": 12},
            ]
            if has_book
            else []
        ),
        "cum_volume": volume,
        "intermediate_print": False,
    }


@unittest.skipUnless(shutil.which("zstd"), "zstd command is not installed")
class ExportTeralionSemanticFixtureTests(unittest.TestCase):
    def make_partition(self, root: Path) -> Path:
        partition = root / "twse" / "2026-08-10" / "3026"
        revision = "a" * 64
        revision_root = partition / "revisions" / revision
        objects = revision_root / "objects"
        objects.mkdir(parents=True)
        (partition / "current.yaml").write_text(revision, encoding="utf-8")
        items = [
            quote(0, 16, 0, None, 10),
            quote(0.4, 16, 2, 0, 10),
            quote(5, 128, 0, 2, 10),
            quote(120, 0, 0, 3, 13),
            quote(121, 16, 0, 1, 14),
            quote(122, 16, 0, 2, 16, market_order=True),
            quote(123, 16, 0, 1, 17, market_order=True),
        ]
        items[3]["deal"]["price"] = "__EXACT_PRICE__"
        uncompressed = json.dumps(
            {"items": items, "next_cursor": None}, separators=(",", ":")
        ).encode().replace(
            b'"__EXACT_PRICE__"',
            b"1.23456789012345678901234567890123456789123456789012345678e38",
        )
        compressed = subprocess.run(
            ["zstd", "-q", "-c"], input=uncompressed, check=True, stdout=subprocess.PIPE
        ).stdout
        relative_path = "objects/00000000.json.zst"
        (revision_root / relative_path).write_bytes(compressed)
        manifest = {
            "manifest_version": 1,
            "revision_identity": revision,
            "terminal_cursor_reached": True,
            "tick_record_count": len(items),
            "pages": [{
                "kind": "TickPage",
                "ordinal": 0,
                "relative_path": relative_path,
                "record_count": len(items),
                "uncompressed_bytes": len(uncompressed),
                "compressed_bytes": len(compressed),
                "uncompressed_sha256": hashlib.sha256(uncompressed).hexdigest(),
                "compressed_sha256": hashlib.sha256(compressed).hexdigest(),
            }],
        }
        (revision_root / "manifest.yaml").write_text(json.dumps(manifest), encoding="utf-8")
        return partition

    def test_exports_full_semantics_and_explicit_transforms_deterministically(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            partition = self.make_partition(Path(temp_dir))
            arguments = dict(
                case_id="twse-stability-up",
                stream="STOCK_REALTIME",
                target_market="twse",
                target_trading_date="2026-07-01",
                target_symbol="2330",
                target_base_time=datetime.fromisoformat("2026-07-01T10:00:00+08:00"),
                price_delta=Decimal(-470),
                quantity_scale=100,
            )
            first = export_fixture(partition, **arguments)
            second = export_fixture(partition, **arguments)
            self.assertEqual(first, second)
            self.assertEqual(first["observations"][0]["price"], None)
            self.assertEqual(first["observations"][0]["quantity"], None)
            self.assertEqual(len(first["observations"][0]["bids"]), 4)
            self.assertEqual(
                first["observations"][0]["bids"][0], {"price": "107", "quantity": 300}
            )
            self.assertIsNone(first["observations"][0]["market_bid_quantity"])
            self.assertEqual(
                first["observations"][0]["asks"],
                [
                    {"price": "109", "quantity": 400},
                    {"price": "110", "quantity": 500},
                    {"price": "111", "quantity": 1_000},
                    {"price": "112", "quantity": 1_100},
                    {"price": "113", "quantity": 1_200},
                ],
            )
            formal = next(item for item in first["observations"] if item["phase"] == "auction_result")
            self.assertEqual(formal["quantity"], 300)
            self.assertEqual(
                formal["price"],
                "123456789012345678901234567890123456319.123456789012345678",
            )
            self.assertEqual(formal["cumulative_volume"], 1_300)
            trigger = next(item for item in first["observations"] if item["phase"] == "trigger")
            self.assertEqual(
                formal["cumulative_volume"] - trigger["cumulative_volume"],
                formal["quantity"],
            )
            continuous_market = next(
                item for item in first["observations"] if item["phase"] == "continuous_market"
            )
            self.assertEqual(continuous_market["market_bid_quantity"], 700)
            self.assertEqual(continuous_market["offset_us"], 122_000_000)
            self.assertEqual(len(continuous_market["bids"]), 3)
            self.assertEqual(
                continuous_market["provenance"]["kind"], "source_observation"
            )
            self.assertEqual(first["expected"]["trigger_offset_us"], 400_000)
            self.assertEqual(first["expected"]["expected_match_offset_us"], 120_400_000)
            self.assertEqual(
                first["provenance"]["transforms"]["quantity"]["targets"],
                [
                    "quantity", "bids[*].quantity", "asks[*].quantity",
                    "market_bid_quantity", "market_ask_quantity", "cumulative_volume",
                ],
            )

    def test_rejects_input_beyond_configured_classified_record_bound(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            partition = self.make_partition(Path(temp_dir))
            with self.assertRaisesRegex(TeralionMappingError, "bound exceeded"):
                export_fixture(
                    partition,
                    case_id="bounded",
                    stream="STOCK_REALTIME",
                    max_classified_records=2,
                )


if __name__ == "__main__":
    unittest.main()
