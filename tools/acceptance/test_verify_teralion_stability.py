#!/usr/bin/env python3

from __future__ import annotations

import unittest
from datetime import datetime, timedelta

from stability_lifecycle import StabilityPhase
from verify_teralion_stability import classify_record


def tick(seconds: int, status: int, limits: int, quantity: int, volume: int):
    match_time = datetime.fromisoformat("2026-07-20T09:00:00+08:00") + timedelta(
        seconds=seconds
    )
    trial = status & 0x80 != 0
    return {
        "type": "quote",
        "market": "twse",
        "symbol": "2330",
        "format": "STOCK_REALTIME",
        "match_time": match_time.isoformat(),
        "received_at": match_time.isoformat(),
        "status_flags": status,
        "limit_flags": limits,
        "deal": {"price": 100, "quantity": quantity},
        "bids": [{"price": 99, "quantity": 3}] if trial or quantity else [],
        "asks": [{"price": 101, "quantity": 4}] if trial or quantity else [],
        "cum_volume": volume,
        "intermediate_print": False,
    }


class TeralionStabilityMappingTests(unittest.TestCase):
    def test_classifies_wire_fields_before_generic_lifecycle_validation(self):
        cases = [
            (tick(0, 16, 2, 0, 10), StabilityPhase.TRIGGER),
            (tick(5, 128, 0, 2, 10), StabilityPhase.INDICATIVE),
            (tick(120, 0, 0, 3, 13), StabilityPhase.AUCTION_RESULT),
            (tick(121, 16, 0, 1, 14), StabilityPhase.CONTINUOUS),
        ]
        self.assertEqual(
            [classify_record(record).phase for record, _ in cases],
            [phase for _, phase in cases],
        )

    def test_does_not_classify_non_teralion_regular_format(self):
        record = tick(0, 16, 2, 0, 10)
        record["format"] = "ANOTHER_PROVIDER_QUOTE"
        self.assertIsNone(classify_record(record))


if __name__ == "__main__":
    unittest.main()
