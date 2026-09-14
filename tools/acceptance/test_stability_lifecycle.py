#!/usr/bin/env python3

from __future__ import annotations

import unittest
from datetime import datetime, timedelta

from stability_lifecycle import (
    LifecycleVerificationError,
    StabilityObservation,
    StabilityPhase,
    find_stability_lifecycle,
)


def lifecycle() -> list[StabilityObservation]:
    trigger = datetime.fromisoformat("2026-07-20T09:00:00+08:00")
    return [
        StabilityObservation("provider-a", trigger, StabilityPhase.TRIGGER, 10, "up"),
        StabilityObservation(
            "provider-a",
            trigger + timedelta(seconds=5),
            StabilityPhase.INDICATIVE,
            10,
            has_indicative_value=True,
        ),
        StabilityObservation(
            "provider-a",
            trigger + timedelta(seconds=120),
            StabilityPhase.AUCTION_RESULT,
            13,
        ),
        StabilityObservation(
            "provider-a",
            trigger + timedelta(seconds=121),
            StabilityPhase.CONTINUOUS,
            14,
        ),
    ]


class StabilityLifecycleTests(unittest.TestCase):
    def test_validates_preclassified_provider_neutral_observations(self):
        report = find_stability_lifecycle(lifecycle())
        self.assertEqual(report["stream"], "provider-a")
        self.assertEqual(report["direction"], "up")
        self.assertEqual(report["trial_observation_count"], 1)
        self.assertEqual(len(report["selected_observations_sha256"]), 64)

    def test_requires_formal_volume_transition(self):
        observations = lifecycle()
        observations[2] = StabilityObservation(
            "provider-a",
            observations[2].match_time,
            StabilityPhase.AUCTION_RESULT,
            10,
        )
        with self.assertRaises(LifecycleVerificationError):
            find_stability_lifecycle(observations)

    def test_does_not_join_different_provider_streams(self):
        observations = lifecycle()
        observations[1] = StabilityObservation(
            "provider-b",
            observations[1].match_time,
            StabilityPhase.INDICATIVE,
            10,
            has_indicative_value=True,
        )
        with self.assertRaises(LifecycleVerificationError):
            find_stability_lifecycle(observations)


if __name__ == "__main__":
    unittest.main()
