//! Server-side application sequence and ACK-QoS state from
//! `Amazon::Hub::ClientMessagingInterface`.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use thiserror::Error;

use super::{
    ApplicationLayerSeqAck, BandwidthMode, ClientContextId, FragmentKey, InterestId,
    ReplicatedStateBundle, ReplicationControlData, SequenceNumber,
};

/// Actor-local identity of one fragment in a client's sync state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FragmentSyncKey {
    interest_id: InterestId,
    fragment_key: FragmentKey,
}

impl FragmentSyncKey {
    #[must_use]
    pub const fn new(interest_id: InterestId, fragment_key: FragmentKey) -> Self {
        Self {
            interest_id,
            fragment_key,
        }
    }

    #[must_use]
    pub const fn interest_id(self) -> InterestId {
        self.interest_id
    }

    #[must_use]
    pub const fn fragment_key(self) -> FragmentKey {
        self.fragment_key
    }
}

/// New baseline established for one fragment by a carrier-accepted bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentBaselineUpdate {
    key: FragmentSyncKey,
    sequence: SequenceNumber,
}

impl FragmentBaselineUpdate {
    #[must_use]
    pub const fn new(key: FragmentSyncKey, sequence: SequenceNumber) -> Self {
        Self { key, sequence }
    }

    #[must_use]
    pub const fn key(self) -> FragmentSyncKey {
        self.key
    }

    #[must_use]
    pub const fn sequence(self) -> SequenceNumber {
        self.sequence
    }
}

/// Application sequence lane selected by
/// `ReplicatedStateBundle::m_isUnreliable`.
///
/// This is independent of carrier delivery reliability. Native required-
/// connection state uses this unreliable application lane while the carrier
/// delivers the message reliably.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateBundleLane {
    Reliable,
    Unreliable,
}

/// Native send lifecycle selected after a state bundle's contents are known.
///
/// Required connection data is retained and retransmitted on the unreliable
/// application lane until ACK `QoS` becomes healthy after a resend. One-time
/// data advances as soon as the carrier accepts the send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateBundleDelivery {
    OneTime,
    RequiredConnectionData,
}

impl StateBundleLane {
    #[must_use]
    pub const fn from_unreliable(is_unreliable: bool) -> Self {
        if is_unreliable {
            Self::Unreliable
        } else {
            Self::Reliable
        }
    }

    #[must_use]
    pub const fn for_bundle(bundle: &ReplicatedStateBundle) -> Self {
        Self::from_unreliable(bundle.is_unreliable())
    }
}

/// Validated `AckQosData` configuration for reusable-bundle retransmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AckQosConfig {
    max_ack_dt_before_bundle_resend: Duration,
}

impl AckQosConfig {
    #[must_use]
    pub const fn max_ack_dt_before_bundle_resend(self) -> Duration {
        self.max_ack_dt_before_bundle_resend
    }
}

impl TryFrom<Duration> for AckQosConfig {
    type Error = AckQosConfigError;

    fn try_from(max_ack_dt_before_bundle_resend: Duration) -> Result<Self, Self::Error> {
        if max_ack_dt_before_bundle_resend.is_zero() {
            return Err(AckQosConfigError::ZeroResendInterval);
        }
        Ok(Self {
            max_ack_dt_before_bundle_resend,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AckQosData {
    config: AckQosConfig,
    last_ack_at: Option<Duration>,
}

impl AckQosData {
    const fn new(config: AckQosConfig) -> Self {
        Self {
            config,
            last_ack_at: None,
        }
    }

    const fn observe(&mut self, ack: &ApplicationLayerSeqAck, now: Duration) {
        if ack.newest_sequence().is_valid() {
            self.last_ack_at = Some(now);
        }
    }

    fn resend_due(&self, last_queued_at: Duration, now: Duration) -> bool {
        let activity_at = self
            .last_ack_at
            .map_or(last_queued_at, |last_ack| last_ack.max(last_queued_at));
        now.saturating_sub(activity_at) >= self.config.max_ack_dt_before_bundle_resend
    }

    fn healthy_after(&self, send_at: Duration, now: Duration) -> bool {
        self.last_ack_at.is_some_and(|last_ack| {
            last_ack >= send_at
                && last_ack <= now
                && now.saturating_sub(last_ack) <= self.config.max_ack_dt_before_bundle_resend
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AckQosConfigError {
    #[error("ACK QoS bundle-resend interval must be nonzero")]
    ZeroResendInterval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SequenceCursor {
    next: SequenceNumber,
}

impl Default for SequenceCursor {
    fn default() -> Self {
        Self {
            next: SequenceNumber::STARTING_SEQUENCE,
        }
    }
}

impl SequenceCursor {
    const fn next(self) -> SequenceNumber {
        self.next
    }

    fn commit(&mut self, expected: SequenceNumber) -> Result<(), ClientMessagingError> {
        if self.next != expected {
            return Err(ClientMessagingError::SequenceMismatch {
                expected: self.next,
                received: expected,
            });
        }
        self.next = self.next.next();
        Ok(())
    }

    fn accept(&mut self, received: SequenceNumber) -> bool {
        let Some(_) = received.as_seq() else {
            return true;
        };
        if received != self.next {
            return false;
        }
        self.next = self.next.next();
        true
    }
}

/// Receive-side sequence gates for the two application lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateBundleSequenceTracker {
    reliable: SequenceCursor,
    unreliable: SequenceCursor,
}

impl StateBundleSequenceTracker {
    #[must_use]
    pub const fn expected(&self, lane: StateBundleLane) -> SequenceNumber {
        match lane {
            StateBundleLane::Reliable => self.reliable.next(),
            StateBundleLane::Unreliable => self.unreliable.next(),
        }
    }

    pub fn accept(&mut self, lane: StateBundleLane, sequence: SequenceNumber) -> bool {
        match lane {
            StateBundleLane::Reliable => self.reliable.accept(sequence),
            StateBundleLane::Unreliable => self.unreliable.accept(sequence),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnackedReplicationControlRecord {
    sequence: SequenceNumber,
    stopped_interest_ids: Vec<InterestId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct UnackedReplicationControl {
    records: VecDeque<UnackedReplicationControlRecord>,
}

impl UnackedReplicationControl {
    fn record(
        &mut self,
        sequence: SequenceNumber,
        stopped_interest_ids: impl IntoIterator<Item = InterestId>,
    ) {
        let stopped_interest_ids = stopped_interest_ids.into_iter().collect::<Vec<_>>();
        if stopped_interest_ids.is_empty() {
            return;
        }
        debug_assert!(
            self.records
                .back()
                .is_none_or(|record| record.sequence < sequence),
            "replication-control records must follow application sequence order"
        );
        self.records.push_back(UnackedReplicationControlRecord {
            sequence,
            stopped_interest_ids,
        });
    }

    fn acknowledge_through(&mut self, newest_ack: SequenceNumber) -> Vec<InterestId> {
        let Some(newest_ack) = newest_ack.as_seq() else {
            return Vec::new();
        };
        let mut released = Vec::new();
        while self.records.front().is_some_and(|record| {
            record
                .sequence
                .as_seq()
                .is_some_and(|sequence| sequence <= newest_ack)
        }) {
            let record = self
                .records
                .pop_front()
                .expect("front record was checked above");
            released.extend(record.stopped_interest_ids);
        }
        released
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReusableUnreliableBundle {
    bundle: ReplicatedStateBundle,
    last_queued_at: Option<Duration>,
    resending_previous: bool,
}

/// Whether a retained reusable bundle is being sent for the first time or
/// retransmitted by ACK `QoS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReusableBundleSendKind {
    First,
    Resend,
}

/// Borrowed send decision for the exact retained bundle.
#[derive(Debug, Clone, Copy)]
pub struct ReusableBundleSend<'a> {
    bundle: &'a ReplicatedStateBundle,
    kind: ReusableBundleSendKind,
}

impl ReusableBundleSend<'_> {
    #[must_use]
    pub const fn bundle(&self) -> &ReplicatedStateBundle {
        self.bundle
    }

    #[must_use]
    pub const fn sequence(&self) -> SequenceNumber {
        self.bundle.seq()
    }

    #[must_use]
    pub const fn kind(&self) -> ReusableBundleSendKind {
        self.kind
    }
}

/// State released by one application-layer acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApplicationLayerSeqAckOutcome {
    released_stop_interest_ids: Vec<InterestId>,
}

impl ApplicationLayerSeqAckOutcome {
    #[must_use]
    pub fn released_stop_interest_ids(&self) -> &[InterestId] {
        &self.released_stop_interest_ids
    }

    #[must_use]
    pub fn into_released_stop_interest_ids(self) -> Vec<InterestId> {
        self.released_stop_interest_ids
    }
}

/// Work completed by the native client-messaging maintenance update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientMessagingUpdate {
    retired_reusable_bundle: Option<SequenceNumber>,
}

impl ClientMessagingUpdate {
    #[must_use]
    pub const fn retired_reusable_bundle(self) -> Option<SequenceNumber> {
        self.retired_reusable_bundle
    }
}

/// Server-side state for one native `ClientMessagingInterface` application
/// sequence stream.
#[derive(Debug, Clone)]
pub struct ClientMessaging {
    reliable: SequenceCursor,
    unreliable: SequenceCursor,
    client_context_id: ClientContextId,
    bandwidth_mode: BandwidthMode,
    ack_qos: AckQosData,
    reusable_unreliable_bundle: Option<ReusableUnreliableBundle>,
    unacked_replication_control: UnackedReplicationControl,
    fragment_baselines: HashMap<FragmentSyncKey, SequenceNumber>,
}

impl ClientMessaging {
    #[must_use]
    pub fn new(
        client_context_id: ClientContextId,
        bandwidth_mode: BandwidthMode,
        ack_qos: AckQosConfig,
    ) -> Self {
        Self {
            reliable: SequenceCursor {
                next: SequenceNumber::STARTING_SEQUENCE,
            },
            unreliable: SequenceCursor {
                next: SequenceNumber::STARTING_SEQUENCE,
            },
            client_context_id,
            bandwidth_mode,
            ack_qos: AckQosData::new(ack_qos),
            reusable_unreliable_bundle: None,
            unacked_replication_control: UnackedReplicationControl {
                records: VecDeque::new(),
            },
            fragment_baselines: HashMap::new(),
        }
    }

    #[must_use]
    pub const fn client_context_id(&self) -> ClientContextId {
        self.client_context_id
    }

    pub const fn set_client_context_id(&mut self, client_context_id: ClientContextId) {
        self.client_context_id = client_context_id;
    }

    #[must_use]
    pub const fn bandwidth_mode(&self) -> BandwidthMode {
        self.bandwidth_mode
    }

    pub const fn set_bandwidth_mode(&mut self, bandwidth_mode: BandwidthMode) {
        self.bandwidth_mode = bandwidth_mode;
    }

    /// Return the next sequence only when the lane can accept a new bundle.
    #[must_use]
    pub const fn next_available_sequence(&self, lane: StateBundleLane) -> Option<SequenceNumber> {
        if matches!(lane, StateBundleLane::Unreliable) && self.reusable_unreliable_bundle.is_some()
        {
            return None;
        }
        Some(self.cursor(lane).next())
    }

    /// Build a single-shot bundle for `lane` at that lane's next sequence.
    ///
    /// # Errors
    ///
    /// Returns [`ClientMessagingError::LaneBlockedByReusableBundle`] when
    /// `lane` is [`StateBundleLane::Unreliable`] and a reusable unreliable
    /// bundle is still active — that bundle owns the lane's next sequence.
    pub fn build_one_time_bundle(
        &self,
        lane: StateBundleLane,
        body: Vec<u8>,
        replication_control: ReplicationControlData,
    ) -> Result<ReplicatedStateBundle, ClientMessagingError> {
        let sequence = self
            .next_available_sequence(lane)
            .ok_or(ClientMessagingError::LaneBlockedByReusableBundle)?;
        Ok(self.build_bundle(sequence, lane, body, replication_control))
    }

    /// Advance `lane`'s send cursor past a bundle that has just been queued,
    /// recording its stop-replication ids and fragment baselines.
    ///
    /// # Errors
    ///
    /// Returns [`ClientMessagingError::SequenceMismatch`] if `sequence` is not
    /// the lane cursor's next value — the bundle being marked was built against
    /// a different cursor state. Nothing is recorded in that case.
    pub fn mark_one_time_bundle_queued(
        &mut self,
        lane: StateBundleLane,
        sequence: SequenceNumber,
        stopped_interest_ids: impl IntoIterator<Item = InterestId>,
        fragment_baselines: impl IntoIterator<Item = FragmentBaselineUpdate>,
    ) -> Result<(), ClientMessagingError> {
        self.cursor_mut(lane).commit(sequence)?;
        self.unacked_replication_control
            .record(sequence, stopped_interest_ids);
        self.update_fragment_baselines(fragment_baselines);
        Ok(())
    }

    /// Retain a bundle on the unreliable lane so it can be resent until ACK
    /// `QoS` recovers.
    ///
    /// # Errors
    ///
    /// Returns [`ClientMessagingError::ReusableBundleAlreadyActive`] if a
    /// reusable unreliable bundle is already retained; only one may be held at
    /// a time.
    pub fn begin_reusable_unreliable_bundle(
        &mut self,
        body: Vec<u8>,
        replication_control: ReplicationControlData,
    ) -> Result<(), ClientMessagingError> {
        if self.reusable_unreliable_bundle.is_some() {
            return Err(ClientMessagingError::ReusableBundleAlreadyActive);
        }
        let sequence = self.unreliable.next();
        let bundle = self.build_bundle(
            sequence,
            StateBundleLane::Unreliable,
            body,
            replication_control,
        );
        self.reusable_unreliable_bundle = Some(ReusableUnreliableBundle {
            bundle,
            last_queued_at: None,
            resending_previous: false,
        });
        Ok(())
    }

    #[must_use]
    pub const fn has_reusable_unreliable_bundle(&self) -> bool {
        self.reusable_unreliable_bundle.is_some()
    }

    #[must_use]
    pub fn reusable_unreliable_bundle_due(&self, now: Duration) -> Option<ReusableBundleSend<'_>> {
        let retained = self.reusable_unreliable_bundle.as_ref()?;
        let Some(last_queued_at) = retained.last_queued_at else {
            return Some(ReusableBundleSend {
                bundle: &retained.bundle,
                kind: ReusableBundleSendKind::First,
            });
        };
        if !self.ack_qos.resend_due(last_queued_at, now) {
            return None;
        }
        Some(ReusableBundleSend {
            bundle: &retained.bundle,
            kind: ReusableBundleSendKind::Resend,
        })
    }

    /// Record that the retained reusable bundle has been handed to the
    /// carrier, reporting whether this was its first send or a resend.
    ///
    /// # Errors
    ///
    /// Returns [`ClientMessagingError::NoReusableBundle`] if no reusable
    /// bundle is retained, [`ClientMessagingError::SequenceMismatch`] if
    /// `sequence` is not the retained bundle's sequence, or — on the first
    /// send only — the [`ClientMessagingError::SequenceMismatch`] raised by
    /// committing the unreliable cursor.
    ///
    /// # Panics
    ///
    /// Panics if the retained bundle disappears between the immutable check
    /// above and the mutable re-borrow. `&mut self` is held throughout, so
    /// that is unreachable.
    pub fn mark_reusable_unreliable_bundle_queued(
        &mut self,
        sequence: SequenceNumber,
        now: Duration,
        fragment_baselines: impl IntoIterator<Item = FragmentBaselineUpdate>,
    ) -> Result<ReusableBundleSendKind, ClientMessagingError> {
        let retained = self
            .reusable_unreliable_bundle
            .as_ref()
            .ok_or(ClientMessagingError::NoReusableBundle)?;
        if retained.bundle.seq() != sequence {
            return Err(ClientMessagingError::SequenceMismatch {
                expected: retained.bundle.seq(),
                received: sequence,
            });
        }

        let first_send = retained.last_queued_at.is_none();
        let stopped_interest_ids = if first_send {
            retained.bundle.stop_replication_ids().to_vec()
        } else {
            Vec::default()
        };
        if first_send {
            self.unreliable.commit(sequence)?;
            self.unacked_replication_control
                .record(sequence, stopped_interest_ids);
            self.update_fragment_baselines(fragment_baselines);
        }

        let retained = self
            .reusable_unreliable_bundle
            .as_mut()
            .expect("reusable bundle was validated above");
        let kind = if first_send {
            ReusableBundleSendKind::First
        } else {
            retained.resending_previous = true;
            ReusableBundleSendKind::Resend
        };
        retained.last_queued_at = Some(now);
        Ok(kind)
    }

    #[must_use]
    pub fn acknowledge_application_sequence(
        &mut self,
        ack: &ApplicationLayerSeqAck,
        now: Duration,
    ) -> ApplicationLayerSeqAckOutcome {
        self.ack_qos.observe(ack, now);
        let released_stop_interest_ids = self
            .unacked_replication_control
            .acknowledge_through(ack.newest_sequence());
        self.fragment_baselines
            .retain(|key, _| !released_stop_interest_ids.contains(&key.interest_id()));
        ApplicationLayerSeqAckOutcome {
            released_stop_interest_ids,
        }
    }

    /// Baseline currently established for one fragment in this client sync.
    #[must_use]
    pub fn fragment_baseline(&self, key: FragmentSyncKey) -> SequenceNumber {
        self.fragment_baselines
            .get(&key)
            .copied()
            .unwrap_or(SequenceNumber::Invalid)
    }

    fn update_fragment_baselines(
        &mut self,
        updates: impl IntoIterator<Item = FragmentBaselineUpdate>,
    ) {
        for update in updates {
            if !update.sequence().is_valid() {
                continue;
            }
            let baseline = self
                .fragment_baselines
                .entry(update.key())
                .or_insert(SequenceNumber::Invalid);
            *baseline = (*baseline).max(update.sequence());
        }
    }

    /// Run the native post-input client-messaging maintenance step.
    ///
    /// A reusable bundle is not released merely because its sequence appears
    /// in an ACK. Native requires that a resend has occurred and that ACK `QoS`
    /// subsequently returned to healthy.
    ///
    /// # Panics
    ///
    /// Panics if the retained reusable bundle disappears between the
    /// retirement test and the `take` that follows it. `&mut self` is held
    /// throughout, so that is unreachable.
    #[must_use]
    pub fn update(&mut self, now: Duration) -> ClientMessagingUpdate {
        let should_retire = self
            .reusable_unreliable_bundle
            .as_ref()
            .filter(|retained| retained.resending_previous)
            .and_then(|retained| retained.last_queued_at)
            .is_some_and(|last_send| self.ack_qos.healthy_after(last_send, now));
        let retired_reusable_bundle = should_retire.then(|| {
            self.reusable_unreliable_bundle
                .take()
                .expect("retained bundle was selected above")
                .bundle
                .seq()
        });

        ClientMessagingUpdate {
            retired_reusable_bundle,
        }
    }

    fn build_bundle(
        &self,
        sequence: SequenceNumber,
        lane: StateBundleLane,
        body: Vec<u8>,
        replication_control: ReplicationControlData,
    ) -> ReplicatedStateBundle {
        let builder = ReplicatedStateBundle::builder(sequence, self.client_context_id)
            .bandwidth(self.bandwidth_mode)
            .with_replication_control(replication_control)
            .payload(body);
        match lane {
            StateBundleLane::Reliable => builder.build(),
            StateBundleLane::Unreliable => builder.unreliable().build(),
        }
    }

    const fn cursor(&self, lane: StateBundleLane) -> &SequenceCursor {
        match lane {
            StateBundleLane::Reliable => &self.reliable,
            StateBundleLane::Unreliable => &self.unreliable,
        }
    }

    const fn cursor_mut(&mut self, lane: StateBundleLane) -> &mut SequenceCursor {
        match lane {
            StateBundleLane::Reliable => &mut self.reliable,
            StateBundleLane::Unreliable => &mut self.unreliable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClientMessagingError {
    #[error("a reusable unreliable state bundle is already active")]
    ReusableBundleAlreadyActive,
    #[error("no reusable unreliable state bundle is active")]
    NoReusableBundle,
    #[error("the unreliable application lane is blocked by a reusable state bundle")]
    LaneBlockedByReusableBundle,
    #[error("application sequence mismatch: expected {expected:?}, received {received:?}")]
    SequenceMismatch {
        expected: SequenceNumber,
        received: SequenceNumber,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receive_tracker_keeps_application_lanes_independent() {
        let mut tracker = StateBundleSequenceTracker::default();

        assert!(tracker.accept(StateBundleLane::Unreliable, SequenceNumber::Seq(1)));
        assert!(!tracker.accept(StateBundleLane::Unreliable, SequenceNumber::Seq(1)));
        assert!(tracker.accept(StateBundleLane::Reliable, SequenceNumber::Seq(1)));
        assert_eq!(
            tracker.expected(StateBundleLane::Unreliable),
            SequenceNumber::Seq(2)
        );
        assert_eq!(
            tracker.expected(StateBundleLane::Reliable),
            SequenceNumber::Seq(2)
        );
    }

    #[test]
    fn receive_tracker_accepts_non_sequence_sentinels_without_advancing() {
        let mut tracker = StateBundleSequenceTracker::default();

        assert!(tracker.accept(StateBundleLane::Reliable, SequenceNumber::Invalid));
        assert!(tracker.accept(StateBundleLane::Reliable, SequenceNumber::ValidNonSequence));
        assert_eq!(
            tracker.expected(StateBundleLane::Reliable),
            SequenceNumber::STARTING_SEQUENCE
        );
    }
}
