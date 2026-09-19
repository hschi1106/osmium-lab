mod support;

use market_types::InstrumentClass;
use run_planner::{
    CacheAction, CacheIdentity, CacheState, CompletionPolicy, CorruptReason, DegradedScope,
    EffectiveRunConfig, ExecutionPlan, IncompleteReason, NetworkRequirement, PlanError,
    PlannedPartition, PlanningVersionSet, SessionPlan, SessionPlanIdentity, SourceAction, SourceId,
    SourcePartitionKey, SourceRevisionIdentity, SourceState, SourceStateKind, VerificationAction,
};
use strategy_api::SessionKind;

use support::{date, degraded, instrument, run_config};

fn partition(
    symbol: &str,
    date_value: &str,
    sessions: Vec<SessionKind>,
    session_identity: u8,
) -> SourcePartitionKey {
    SourcePartitionKey::new(
        SourceId::new("teralion").unwrap(),
        instrument(symbol),
        date(date_value),
        sessions,
        SessionPlanIdentity::from_bytes([session_identity; 32]),
    )
    .unwrap()
}

fn planned_partition(
    symbol: &str,
    date_value: &str,
    sessions: Vec<SessionKind>,
    source_state: SourceState,
    cache_state: CacheState,
) -> PlannedPartition {
    let instrument = instrument(symbol);
    let trading_date = date(date_value);
    let session_plan = SessionPlan::for_instrument_class(
        &instrument,
        InstrumentClass::Equity,
        trading_date,
        sessions.clone(),
    )
    .unwrap();
    let key = SourcePartitionKey::new(
        SourceId::new("teralion").unwrap(),
        instrument,
        trading_date,
        sessions,
        session_plan.identity(),
    )
    .unwrap();
    PlannedPartition::classify(key, session_plan, source_state, cache_state).unwrap()
}

#[test]
fn partition_identity_uses_canonical_session_order() {
    let left = partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::AfterHours, SessionKind::Regular],
        1,
    );
    let right = partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular, SessionKind::AfterHours],
        1,
    );

    assert_eq!(left.session_kinds(), right.session_kinds());
    assert_eq!(left.canonical_bytes(), right.canonical_bytes());
    assert_eq!(left.identity(), right.identity());
    assert!(left.canonical_bytes().starts_with(b"OSPK"));
}

#[test]
fn provider_neutral_source_ids_are_storage_safe_and_part_of_identity() {
    let teralion = SourceId::new("teralion").unwrap();
    let synthetic = SourceId::new("synthetic-source").unwrap();

    assert_eq!(synthetic.as_str(), "synthetic-source");
    assert_eq!(synthetic.storage_namespace(), "synthetic-source");
    assert_ne!(
        SourcePartitionKey::new(
            teralion,
            instrument("2330"),
            date("2026-07-27"),
            vec![SessionKind::Regular],
            SessionPlanIdentity::from_bytes([1; 32]),
        )
        .unwrap()
        .identity(),
        SourcePartitionKey::new(
            synthetic,
            instrument("2330"),
            date("2026-07-27"),
            vec![SessionKind::Regular],
            SessionPlanIdentity::from_bytes([1; 32]),
        )
        .unwrap()
        .identity()
    );
    assert!(SourceId::new("").is_err());
    assert!(SourceId::new("unsafe/source").is_err());
}

#[test]
fn five_source_states_have_distinct_stable_kinds() {
    let revision = SourceRevisionIdentity::from_bytes([2; 32]);
    assert_eq!(SourceState::Missing.kind(), SourceStateKind::Missing);
    assert_eq!(SourceState::Building.kind(), SourceStateKind::Building);
    assert_eq!(
        SourceState::Complete { revision }.kind(),
        SourceStateKind::Complete
    );
    assert_eq!(
        SourceState::Incomplete {
            reason: IncompleteReason::CursorNotTerminal
        }
        .kind(),
        SourceStateKind::Incomplete
    );
    assert_eq!(
        SourceState::Corrupt {
            reason: CorruptReason::PayloadChecksumMismatch
        }
        .kind(),
        SourceStateKind::Corrupt
    );
}

#[test]
fn source_and_cache_states_classify_into_explicit_actions() {
    let key = partition("2330", "2026-07-27", vec![SessionKind::Regular], 1);
    let revision = SourceRevisionIdentity::from_bytes([3; 32]);
    let cache = CacheIdentity::from_bytes([4; 32]);

    let complete = PlannedPartition::classify(
        key.clone(),
        SessionPlan::for_instrument_class(
            key.instrument(),
            InstrumentClass::Equity,
            key.trading_date(),
            key.session_kinds().iter().copied(),
        )
        .unwrap(),
        SourceState::Complete { revision },
        CacheState::Valid { identity: cache },
    );
    assert!(complete.is_err());

    let complete = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Complete { revision },
        CacheState::Valid { identity: cache },
    );
    assert_eq!(
        complete.source_action(),
        SourceAction::ReuseCompleteSource { revision }
    );
    assert_eq!(
        complete.verification_action(),
        VerificationAction::VerifyCompleteSource
    );
    assert_eq!(
        complete.cache_action(),
        CacheAction::ReuseValidCache { identity: cache }
    );

    let missing = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Missing,
        CacheState::Missing,
    );
    assert_eq!(missing.source_action(), SourceAction::DownloadMissingSource);
    assert_eq!(missing.cache_action(), CacheAction::AwaitCompleteSource);

    let building = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Building,
        CacheState::Building,
    );
    assert_eq!(
        building.source_action(),
        SourceAction::ResumeOrRestartBuilding
    );

    let incomplete = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Incomplete {
            reason: IncompleteReason::DailyInstrumentMissing,
        },
        CacheState::Missing,
    );
    assert_eq!(
        incomplete.source_action(),
        SourceAction::RejectIncomplete {
            reason: IncompleteReason::DailyInstrumentMissing
        }
    );

    let corrupt = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Corrupt {
            reason: CorruptReason::CompressionFrameInvalid,
        },
        CacheState::Corrupt,
    );
    assert_eq!(
        corrupt.source_action(),
        SourceAction::RejectCorrupt {
            reason: CorruptReason::CompressionFrameInvalid
        }
    );
}

#[test]
fn coverage_unavailable_is_not_confused_with_missing_download() {
    let session_plan = SessionPlan::for_instrument_class(
        &instrument("2330"),
        InstrumentClass::Equity,
        date("2026-07-27"),
        [SessionKind::Regular],
    )
    .unwrap();
    let key = SourcePartitionKey::new(
        SourceId::new("teralion").unwrap(),
        instrument("2330"),
        date("2026-07-27"),
        [SessionKind::Regular],
        session_plan.identity(),
    )
    .unwrap();
    let planned = PlannedPartition::coverage_unavailable(key, session_plan).unwrap();

    assert_eq!(planned.source_state(), SourceState::Missing);
    assert_eq!(planned.source_action(), SourceAction::CoverageUnavailable);
    assert_eq!(
        planned.verification_action(),
        VerificationAction::CoverageUnavailable
    );
}

#[test]
fn execution_plan_is_independent_of_partition_discovery_order() {
    let selected = instrument("2330");
    let config = EffectiveRunConfig::resolve(run_config(
        vec![date("2026-07-28"), date("2026-07-27")],
        vec![selected],
        "target/test-data",
    ))
    .unwrap();
    let left = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Complete {
            revision: SourceRevisionIdentity::from_bytes([5; 32]),
        },
        CacheState::Missing,
    );
    let right = planned_partition(
        "2330",
        "2026-07-28",
        vec![SessionKind::Regular],
        SourceState::Complete {
            revision: SourceRevisionIdentity::from_bytes([6; 32]),
        },
        CacheState::Stale,
    );

    let forward =
        ExecutionPlan::new(config.clone(), vec![left.clone(), right.clone()], vec![]).unwrap();
    let reverse = ExecutionPlan::new(config, vec![right, left], vec![]).unwrap();

    assert_eq!(forward.canonical_bytes(), reverse.canonical_bytes());
    assert_eq!(forward.identity(), reverse.identity());
    assert_eq!(forward.version_set(), PlanningVersionSet::CURRENT);
    assert_eq!(
        forward.network_requirement(),
        NetworkRequirement::NotRequired
    );
    assert_eq!(forward.completion_policy(), CompletionPolicy::Strict);
}

#[test]
fn execution_plan_requires_every_universe_date_partition() {
    let selected = instrument("2330");
    let config = EffectiveRunConfig::resolve(run_config(
        vec![date("2026-07-27"), date("2026-07-28")],
        vec![selected.clone()],
        "target/test-data",
    ))
    .unwrap();
    let planned = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Missing,
        CacheState::Missing,
    );

    assert_eq!(
        ExecutionPlan::new(config, vec![planned], vec![]),
        Err(PlanError::MissingRequestedPartition {
            instrument: selected,
            trading_date: date("2026-07-28"),
        })
    );
}

#[test]
fn missing_partition_makes_network_requirement_explicit() {
    let selected = instrument("2330");
    let config = EffectiveRunConfig::resolve(run_config(
        vec![date("2026-07-27")],
        vec![selected],
        "target/test-data",
    ))
    .unwrap();
    let planned = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Missing,
        CacheState::Missing,
    );

    let plan = ExecutionPlan::new(config, vec![planned], vec![]).unwrap();
    assert_eq!(plan.network_requirement(), NetworkRequirement::Required);
}

#[test]
fn strict_plan_rejects_degraded_scope() {
    let selected = instrument("2330");
    let config = EffectiveRunConfig::resolve(run_config(
        vec![date("2026-07-27")],
        vec![selected],
        "target/test-data",
    ))
    .unwrap();
    let planned = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Incomplete {
            reason: IncompleteReason::CursorNotTerminal,
        },
        CacheState::Missing,
    );
    let scope = DegradedScope::new(planned.key().identity());

    assert_eq!(
        ExecutionPlan::new(config, vec![planned], vec![scope]),
        Err(PlanError::DegradedScopeWithStrictPolicy)
    );
}

#[test]
fn explicit_degraded_plan_only_accepts_incomplete_partition_scope() {
    let selected = instrument("2330");
    let config = EffectiveRunConfig::resolve(degraded(run_config(
        vec![date("2026-07-27")],
        vec![selected],
        "target/test-data",
    )))
    .unwrap();
    let planned = planned_partition(
        "2330",
        "2026-07-27",
        vec![SessionKind::Regular],
        SourceState::Incomplete {
            reason: IncompleteReason::CoverageUnconfirmed,
        },
        CacheState::Missing,
    );
    let scope = DegradedScope::new(planned.key().identity());

    let plan = ExecutionPlan::new(config, vec![planned], vec![scope]).unwrap();
    assert_eq!(plan.completion_policy(), CompletionPolicy::ExplicitDegraded);
    assert_eq!(plan.degraded_scopes(), [scope]);
}
