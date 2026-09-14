"""Provider-neutral validation of a classified intraday stability lifecycle."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from datetime import datetime, timedelta
from enum import Enum


class LifecycleVerificationError(Exception):
    pass


class StabilityPhase(Enum):
    TRIGGER = "trigger"
    INDICATIVE = "indicative"
    AUCTION_RESULT = "auction_result"
    CONTINUOUS = "continuous"


@dataclass(frozen=True)
class StabilityObservation:
    stream: str
    match_time: datetime
    phase: StabilityPhase
    cumulative_volume: int
    direction: str | None = None
    has_indicative_value: bool = False
    evidence_digest: str = ""


@dataclass(frozen=True)
class StabilityLifecyclePolicy:
    minimum_pause: timedelta = timedelta(seconds=110)
    maximum_pause: timedelta = timedelta(seconds=130)
    maximum_resume_wait: timedelta = timedelta(minutes=5)


def find_stability_lifecycle(
    observations: list[StabilityObservation],
    policy: StabilityLifecyclePolicy = StabilityLifecyclePolicy(),
) -> dict[str, object]:
    ordered = sorted(
        observations,
        key=lambda observation: (observation.match_time, observation.stream),
    )
    for trigger in ordered:
        if (
            trigger.phase is not StabilityPhase.TRIGGER
            or trigger.direction not in {"down", "up"}
            or trigger.cumulative_volume < 0
        ):
            continue
        following = [
            observation
            for observation in ordered
            if observation.stream == trigger.stream
            and observation.match_time > trigger.match_time
        ]
        formal = next(
            (
                observation
                for observation in following
                if observation.phase is StabilityPhase.AUCTION_RESULT
                and policy.minimum_pause
                <= observation.match_time - trigger.match_time
                <= policy.maximum_pause
                and observation.cumulative_volume > trigger.cumulative_volume
            ),
            None,
        )
        if formal is None:
            continue
        trials = [
            observation
            for observation in following
            if observation.match_time < formal.match_time
            and observation.phase is StabilityPhase.INDICATIVE
        ]
        if (
            not trials
            or any(
                observation.cumulative_volume != trigger.cumulative_volume
                for observation in trials
            )
            or not any(observation.has_indicative_value for observation in trials)
        ):
            continue
        resume = next(
            (
                observation
                for observation in following
                if formal.match_time < observation.match_time
                <= formal.match_time + policy.maximum_resume_wait
                and observation.phase is StabilityPhase.CONTINUOUS
                and observation.cumulative_volume >= formal.cumulative_volume
            ),
            None,
        )
        if resume is None:
            continue
        selected = [
            observation
            for observation in ordered
            if observation.stream == trigger.stream
            and trigger.match_time <= observation.match_time <= resume.match_time
        ]
        canonical = json.dumps(
            [
                {
                    "stream": observation.stream,
                    "match_time": observation.match_time.isoformat(),
                    "phase": observation.phase.value,
                    "cumulative_volume": observation.cumulative_volume,
                    "direction": observation.direction,
                    "has_indicative_value": observation.has_indicative_value,
                    "evidence_digest": observation.evidence_digest,
                }
                for observation in selected
            ],
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        return {
            "stream": trigger.stream,
            "direction": trigger.direction,
            "trigger_at": trigger.match_time.isoformat(),
            "trial_first_at": trials[0].match_time.isoformat(),
            "trial_last_at": trials[-1].match_time.isoformat(),
            "trial_observation_count": len(trials),
            "formal_result_at": formal.match_time.isoformat(),
            "resumed_continuous_at": resume.match_time.isoformat(),
            "selected_observation_count": len(selected),
            "selected_observations_sha256": hashlib.sha256(canonical).hexdigest(),
        }
    raise LifecycleVerificationError(
        "no complete trigger/indicative/auction-result/continuous lifecycle found"
    )
