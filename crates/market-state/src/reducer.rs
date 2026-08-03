use std::{cmp::Ordering, collections::BTreeSet, error::Error, fmt};

use market_types::{
    CanonicalEncodingError, DomainEvent, EventPayload, Observation, QuantityUnit, TradePrintKind,
    Volume,
};

use crate::{
    AppliedEventRef, CumulativeVolumePolicy, MarketState, MarketStateProfile, ProfileError,
    SessionSegmentId, StateField, TradeObservation, UnavailableReason,
};

pub const MARKET_STATE_VERSION: u16 = 3;
pub const STATE_REDUCER_VERSION: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentBoundaryPolicy {
    Carry,
    ResetObservableFields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReducerContext {
    trading_date: market_types::TradingDate,
    session_segment_id: SessionSegmentId,
    segment_boundary_policy: SegmentBoundaryPolicy,
    segment_boundary_policy_version: u16,
}

impl ReducerContext {
    #[must_use]
    pub const fn new(
        trading_date: market_types::TradingDate,
        session_segment_id: SessionSegmentId,
        segment_boundary_policy: SegmentBoundaryPolicy,
        segment_boundary_policy_version: u16,
    ) -> Self {
        Self {
            trading_date,
            session_segment_id,
            segment_boundary_policy,
            segment_boundary_policy_version,
        }
    }

    #[must_use]
    pub const fn trading_date(&self) -> market_types::TradingDate {
        self.trading_date
    }

    #[must_use]
    pub const fn session_segment_id(&self) -> &SessionSegmentId {
        &self.session_segment_id
    }
}

#[derive(Debug, Clone)]
pub struct MarketStateReducer {
    profile: MarketStateProfile,
}

impl MarketStateReducer {
    #[must_use]
    pub fn new(profile: MarketStateProfile) -> Self {
        Self { profile }
    }

    #[must_use]
    pub fn twse_regular() -> Self {
        Self::new(MarketStateProfile::twse_regular())
    }

    #[must_use]
    pub fn taifex_futures() -> Self {
        Self::new(MarketStateProfile::taifex_futures())
    }

    #[must_use]
    pub fn taifex_options() -> Self {
        Self::new(MarketStateProfile::taifex_options())
    }

    #[must_use]
    pub fn twse_warrant() -> Self {
        Self::new(MarketStateProfile::twse_warrant())
    }

    #[must_use]
    pub fn tpex_regular() -> Self {
        Self::new(MarketStateProfile::tpex_regular())
    }

    #[must_use]
    pub fn tpex_warrant() -> Self {
        Self::new(MarketStateProfile::tpex_warrant())
    }

    #[must_use]
    pub const fn profile(&self) -> &MarketStateProfile {
        &self.profile
    }

    pub fn propose(
        &self,
        state: &MarketState,
        event: &DomainEvent,
        context: &ReducerContext,
    ) -> Result<ProposedTransition, StateTransitionError> {
        self.validate_envelope(state, event, context)?;
        self.profile
            .validate_event(event)
            .map_err(StateTransitionError::Profile)?;
        validate_realtime_shape(event)?;

        let event_ref =
            AppliedEventRef::from_event(event).map_err(StateTransitionError::CanonicalEncoding)?;
        if let Some(previous) = state.last_event()
            && compare_within_instrument(&event_ref, previous) == Ordering::Less
        {
            return Err(StateTransitionError::OrderingRegression);
        }
        let new_version = state
            .state_version()
            .checked_add(1)
            .ok_or(StateTransitionError::StateVersionOverflow)?;

        let mut next = state.clone();
        let mut changed = BTreeSet::new();
        let mut boundary_action = None;
        let segment_changed = state.current_segment_id() != Some(context.session_segment_id());
        if segment_changed {
            if state.current_segment_id().is_some()
                && context.segment_boundary_policy == SegmentBoundaryPolicy::ResetObservableFields
            {
                next.reset_observable_fields();
                changed.extend([
                    ChangedField::Book,
                    ChangedField::RecentTrade,
                    ChangedField::CumulativeVolume,
                    ChangedField::LastAnnotations,
                ]);
                boundary_action = Some(BoundaryAction::ResetObservableFields);
            }
            next.set_segment(context.session_segment_id.clone());
            changed.insert(ChangedField::CurrentSegment);
        }

        let mut warning_codes = BTreeSet::new();
        match event.payload() {
            EventPayload::QuoteSnapshot(snapshot) => {
                next.set_book(StateField::Known {
                    value: snapshot.book().clone(),
                    observed_at: event_ref.clone(),
                });
                changed.insert(ChangedField::Book);
                apply_trade_observation(
                    &mut next,
                    snapshot.trade(),
                    &event_ref,
                    &mut changed,
                    &mut warning_codes,
                );
                self.apply_volume_observation(
                    &mut next,
                    snapshot.cumulative_volume(),
                    &event_ref,
                    !segment_changed,
                    &mut changed,
                    &mut warning_codes,
                )?;
                next.set_last_annotations(StateField::Known {
                    value: snapshot.annotations().clone(),
                    observed_at: event_ref.clone(),
                });
                changed.insert(ChangedField::LastAnnotations);
            }
            EventPayload::BookSnapshot(snapshot) => {
                next.set_book(StateField::Known {
                    value: snapshot.book().clone(),
                    observed_at: event_ref.clone(),
                });
                changed.insert(ChangedField::Book);
                next.set_last_annotations(StateField::Known {
                    value: snapshot.annotations().clone(),
                    observed_at: event_ref.clone(),
                });
                changed.insert(ChangedField::LastAnnotations);
            }
            EventPayload::TradeBatch(batch) => {
                next.set_recent_trade(StateField::Known {
                    value: TradeObservation::Batch {
                        trades: batch.trades().to_vec().into_boxed_slice(),
                        trade_order: batch.trade_order(),
                    },
                    observed_at: event_ref.clone(),
                });
                changed.insert(ChangedField::RecentTrade);
                self.apply_volume_observation(
                    &mut next,
                    batch.cumulative_volume(),
                    &event_ref,
                    !segment_changed,
                    &mut changed,
                    &mut warning_codes,
                )?;
                next.set_last_annotations(StateField::Known {
                    value: batch.annotations().clone(),
                    observed_at: event_ref.clone(),
                });
                changed.insert(ChangedField::LastAnnotations);
            }
            EventPayload::IndicativeOpeningAuction(auction)
            | EventPayload::IndicativeClosingAuction(auction) => {
                // Indicative values are timeline observations, but they are not
                // actual trades or cumulative volume and must not overwrite
                // executable market state.
                next.set_last_annotations(StateField::Known {
                    value: auction.annotations().clone(),
                    observed_at: event_ref.clone(),
                });
                changed.insert(ChangedField::LastAnnotations);
            }
        }

        next.finish_event(event_ref.clone(), new_version);
        changed.insert(ChangedField::LastEvent);
        Ok(ProposedTransition {
            previous_version: state.state_version(),
            next_state: next,
            receipt: TransitionReceipt {
                instrument: state.instrument().clone(),
                previous_version: state.state_version(),
                new_version,
                event: event_ref,
                boundary_action,
                changed_fields: changed.into_iter().collect(),
                warning_codes: warning_codes.into_iter().collect(),
            },
        })
    }

    pub fn commit(
        &self,
        state: &mut MarketState,
        proposal: ProposedTransition,
    ) -> Result<TransitionReceipt, StateTransitionError> {
        if state.state_version() != proposal.previous_version
            || state.instrument() != proposal.next_state.instrument()
            || state.trading_date() != proposal.next_state.trading_date()
        {
            return Err(StateTransitionError::StaleProposal);
        }
        *state = proposal.next_state;
        Ok(proposal.receipt)
    }

    pub fn apply(
        &self,
        state: &mut MarketState,
        event: &DomainEvent,
        context: &ReducerContext,
    ) -> Result<TransitionReceipt, StateTransitionError> {
        let proposal = self.propose(state, event, context)?;
        self.commit(state, proposal)
    }

    fn validate_envelope(
        &self,
        state: &MarketState,
        event: &DomainEvent,
        context: &ReducerContext,
    ) -> Result<(), StateTransitionError> {
        if event.instrument() != state.instrument() {
            return Err(StateTransitionError::InstrumentMismatch);
        }
        if event.trading_date() != state.trading_date()
            || event.trading_date() != context.trading_date
        {
            return Err(StateTransitionError::TradingDateMismatch);
        }
        if context.segment_boundary_policy_version != self.profile.segment_boundary_policy_version()
        {
            return Err(StateTransitionError::BoundaryPolicyVersionMismatch);
        }
        if let Some(last_time) = state.last_match_time()
            && event.match_time() < last_time
        {
            return Err(StateTransitionError::MatchTimeRegression);
        }
        Ok(())
    }

    fn apply_volume_observation(
        &self,
        state: &mut MarketState,
        observation: &Observation<Volume>,
        event_ref: &AppliedEventRef,
        compare_previous: bool,
        changed: &mut BTreeSet<ChangedField>,
        warnings: &mut BTreeSet<StateWarningCode>,
    ) -> Result<(), StateTransitionError> {
        match observation {
            Observation::NoObservation => return Ok(()),
            Observation::Set(volume) => {
                self.validate_volume(state.cumulative_volume(), *volume, compare_previous)?;
                state.set_cumulative_volume(StateField::Known {
                    value: *volume,
                    observed_at: event_ref.clone(),
                });
            }
            Observation::Clear => {
                state.set_cumulative_volume(StateField::Unavailable(UnavailableReason::Cleared {
                    cleared_at: event_ref.clone(),
                }));
            }
            Observation::Unknown(raw) => {
                state.set_cumulative_volume(StateField::Unknown {
                    raw: raw.clone(),
                    observed_at: event_ref.clone(),
                });
                warnings.insert(StateWarningCode::UnknownCumulativeVolume);
            }
        }
        changed.insert(ChangedField::CumulativeVolume);
        Ok(())
    }

    fn validate_volume(
        &self,
        current: &StateField<Volume>,
        next: Volume,
        compare_previous: bool,
    ) -> Result<(), StateTransitionError> {
        let CumulativeVolumePolicy::NonDecreasingWithinSegment { unit } =
            self.profile.cumulative_volume_policy()
        else {
            return Ok(());
        };
        if next.unit() != unit {
            return Err(StateTransitionError::VolumeUnitMismatch {
                expected: unit,
                actual: next.unit(),
            });
        }
        if compare_previous && let StateField::Known { value, .. } = current {
            if value.unit() != unit {
                return Err(StateTransitionError::VolumeUnitMismatch {
                    expected: unit,
                    actual: value.unit(),
                });
            }
            if next.value() < value.value() {
                return Err(StateTransitionError::CumulativeVolumeRegression {
                    previous: value.value(),
                    next: next.value(),
                });
            }
        }
        Ok(())
    }
}

fn apply_trade_observation(
    state: &mut MarketState,
    observation: &Observation<market_types::TradePrint>,
    event_ref: &AppliedEventRef,
    changed: &mut BTreeSet<ChangedField>,
    warnings: &mut BTreeSet<StateWarningCode>,
) {
    match observation {
        Observation::NoObservation => return,
        Observation::Set(trade) => state.set_recent_trade(StateField::Known {
            value: TradeObservation::Single(*trade),
            observed_at: event_ref.clone(),
        }),
        Observation::Clear => {
            state.set_recent_trade(StateField::Unavailable(UnavailableReason::Cleared {
                cleared_at: event_ref.clone(),
            }))
        }
        Observation::Unknown(raw) => {
            state.set_recent_trade(StateField::Unknown {
                raw: raw.clone(),
                observed_at: event_ref.clone(),
            });
            warnings.insert(StateWarningCode::UnknownRecentTrade);
        }
    }
    changed.insert(ChangedField::RecentTrade);
}

fn validate_realtime_shape(event: &DomainEvent) -> Result<(), StateTransitionError> {
    let error = match event.instrument().market() {
        market_types::MarketId::Twse => StateTransitionError::InvalidTwseRealtimeShape,
        market_types::MarketId::Tpex => StateTransitionError::InvalidTpexRealtimeShape,
        _ => return Ok(()),
    };
    if event.source_format().as_str() != "STOCK_REALTIME" {
        return Ok(());
    }
    match event.payload() {
        EventPayload::QuoteSnapshot(_) => Ok(()),
        EventPayload::TradeBatch(batch)
            if batch
                .trades()
                .iter()
                .all(|trade| trade.print_kind() == TradePrintKind::Intermediate) =>
        {
            Ok(())
        }
        EventPayload::IndicativeOpeningAuction(_) | EventPayload::IndicativeClosingAuction(_) => {
            Ok(())
        }
        EventPayload::BookSnapshot(_) | EventPayload::TradeBatch(_) => Err(error),
    }
}

fn compare_within_instrument(left: &AppliedEventRef, right: &AppliedEventRef) -> Ordering {
    left.match_time()
        .cmp(&right.match_time())
        .then_with(|| left.source_format().cmp(right.source_format()))
        .then_with(|| source_phase_for_ref(left).cmp(&source_phase_for_ref(right)))
        .then_with(|| left.event_kind().cmp(&right.event_kind()))
        .then_with(|| left.source_sequence().cmp(&right.source_sequence()))
        .then_with(|| left.event_fingerprint().cmp(&right.event_fingerprint()))
}

fn source_phase_for_ref(event: &AppliedEventRef) -> u8 {
    event.source_phase()
}

#[derive(Debug, Clone)]
pub struct ProposedTransition {
    previous_version: u64,
    next_state: MarketState,
    receipt: TransitionReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionReceipt {
    instrument: market_types::InstrumentId,
    previous_version: u64,
    new_version: u64,
    event: AppliedEventRef,
    boundary_action: Option<BoundaryAction>,
    changed_fields: Vec<ChangedField>,
    warning_codes: Vec<StateWarningCode>,
}

impl TransitionReceipt {
    #[must_use]
    pub const fn instrument(&self) -> &market_types::InstrumentId {
        &self.instrument
    }

    #[must_use]
    pub const fn previous_version(&self) -> u64 {
        self.previous_version
    }

    #[must_use]
    pub const fn new_version(&self) -> u64 {
        self.new_version
    }

    #[must_use]
    pub const fn event(&self) -> &AppliedEventRef {
        &self.event
    }

    #[must_use]
    pub const fn boundary_action(&self) -> Option<BoundaryAction> {
        self.boundary_action
    }

    #[must_use]
    pub fn changed_fields(&self) -> &[ChangedField] {
        &self.changed_fields
    }

    #[must_use]
    pub fn warning_codes(&self) -> &[StateWarningCode] {
        &self.warning_codes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryAction {
    ResetObservableFields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangedField {
    CurrentSegment,
    Book,
    RecentTrade,
    CumulativeVolume,
    LastAnnotations,
    LastEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StateWarningCode {
    UnknownRecentTrade,
    UnknownCumulativeVolume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateTransitionError {
    InstrumentMismatch,
    TradingDateMismatch,
    BoundaryPolicyVersionMismatch,
    Profile(ProfileError),
    CanonicalEncoding(CanonicalEncodingError),
    MatchTimeRegression,
    OrderingRegression,
    InvalidTwseRealtimeShape,
    InvalidTpexRealtimeShape,
    CumulativeVolumeRegression {
        previous: u64,
        next: u64,
    },
    VolumeUnitMismatch {
        expected: QuantityUnit,
        actual: QuantityUnit,
    },
    StateVersionOverflow,
    StaleProposal,
}

impl fmt::Display for StateTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstrumentMismatch => {
                formatter.write_str("event instrument does not match state")
            }
            Self::TradingDateMismatch => {
                formatter.write_str("event trading date does not match state and reducer context")
            }
            Self::BoundaryPolicyVersionMismatch => {
                formatter.write_str("segment boundary policy version is incompatible")
            }
            Self::Profile(error) => {
                write!(formatter, "market-state profile rejected event: {error}")
            }
            Self::CanonicalEncoding(error) => {
                write!(formatter, "event fingerprint encoding failed: {error}")
            }
            Self::MatchTimeRegression => formatter.write_str("event match_time regressed"),
            Self::OrderingRegression => formatter.write_str("event ordering key regressed"),
            Self::InvalidTwseRealtimeShape => {
                formatter.write_str("invalid TWSE STOCK_REALTIME domain event shape")
            }
            Self::InvalidTpexRealtimeShape => {
                formatter.write_str("invalid TPEx STOCK_REALTIME domain event shape")
            }
            Self::CumulativeVolumeRegression { previous, next } => write!(
                formatter,
                "cumulative volume regressed from {previous} to {next}"
            ),
            Self::VolumeUnitMismatch { expected, actual } => {
                write!(
                    formatter,
                    "volume unit mismatch: {expected:?} != {actual:?}"
                )
            }
            Self::StateVersionOverflow => formatter.write_str("state version overflowed"),
            Self::StaleProposal => formatter.write_str("proposed transition is stale"),
        }
    }
}

impl Error for StateTransitionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Profile(error) => Some(error),
            Self::CanonicalEncoding(error) => Some(error),
            _ => None,
        }
    }
}
