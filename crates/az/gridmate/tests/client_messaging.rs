use std::time::Duration;

use gridmate::hub::{
    AckQosConfig, ApplicationLayerSeqAck, BandwidthMode, ClientContextId, ClientMessaging,
    FragmentBaselineUpdate, FragmentKey, FragmentSyncKey, InterestId, ReplicationControlData,
    ReusableBundleSendKind, SequenceNumber, StateBundleLane,
};
use gridmate::serialize::Marshaler;
use gridmate::serialize::buffer::{CARRIER_ENDIAN, WriteBuffer};

fn ack_qos(milliseconds: u64) -> AckQosConfig {
    AckQosConfig::try_from(Duration::from_millis(milliseconds)).expect("nonzero ACK QoS interval")
}

fn marshal_bundle(bundle: &gridmate::hub::ReplicatedStateBundle) -> Vec<u8> {
    let mut buffer = WriteBuffer::new(CARRIER_ENDIAN);
    bundle.marshal(&mut buffer);
    buffer.into_vec()
}

#[test]
fn reusable_bundle_retires_after_resend_and_valid_ack_qos_activity() {
    let mut messaging =
        ClientMessaging::new(ClientContextId::new(1), BandwidthMode::new(1), ack_qos(350));
    let control =
        ReplicationControlData::from_ids([InterestId::new(7)], std::iter::empty::<InterestId>())
            .unwrap();

    messaging
        .begin_reusable_unreliable_bundle(vec![0xaa, 0xbb], control)
        .unwrap();

    let first = messaging
        .reusable_unreliable_bundle_due(Duration::ZERO)
        .expect("first send is immediately due");
    assert_eq!(first.kind(), ReusableBundleSendKind::First);
    assert_eq!(first.sequence(), SequenceNumber::Seq(1));
    let first_bytes = marshal_bundle(first.bundle());

    messaging
        .mark_reusable_unreliable_bundle_queued(
            SequenceNumber::Seq(1),
            Duration::ZERO,
            std::iter::empty(),
        )
        .unwrap();
    assert_eq!(
        messaging.next_available_sequence(StateBundleLane::Unreliable),
        None,
        "the lane stays unavailable while its reusable bundle awaits ACK"
    );
    assert!(
        messaging
            .reusable_unreliable_bundle_due(Duration::from_millis(349))
            .is_none()
    );

    let resend = messaging
        .reusable_unreliable_bundle_due(Duration::from_millis(350))
        .expect("native ACK QoS interval makes the retained bundle due");
    assert_eq!(resend.kind(), ReusableBundleSendKind::Resend);
    assert_eq!(resend.sequence(), SequenceNumber::Seq(1));
    assert_eq!(marshal_bundle(resend.bundle()), first_bytes);
    messaging
        .mark_reusable_unreliable_bundle_queued(
            SequenceNumber::Seq(1),
            Duration::from_millis(350),
            std::iter::empty(),
        )
        .unwrap();

    let acknowledged = messaging.acknowledge_application_sequence(
        &ApplicationLayerSeqAck::valid_non_sequence(),
        Duration::from_millis(351),
    );
    assert!(acknowledged.released_stop_interest_ids().is_empty());
    assert_eq!(
        messaging
            .update(Duration::from_millis(351))
            .retired_reusable_bundle(),
        Some(SequenceNumber::Seq(1))
    );
    assert!(
        acknowledged.released_stop_interest_ids().is_empty(),
        "a valid non-sequence ACK refreshes QoS but cannot release sequence-owned control"
    );
    assert_eq!(
        messaging.next_available_sequence(StateBundleLane::Unreliable),
        Some(SequenceNumber::Seq(2))
    );
    assert!(
        messaging
            .reusable_unreliable_bundle_due(Duration::from_millis(700))
            .is_none()
    );
}

#[test]
fn newer_application_ack_retires_every_covered_control_record() {
    let mut messaging =
        ClientMessaging::new(ClientContextId::new(1), BandwidthMode::new(1), ack_qos(350));

    for (sequence, interest_id) in [(1, 11), (2, 12)] {
        let control = ReplicationControlData::from_ids(
            [InterestId::new(interest_id)],
            std::iter::empty::<InterestId>(),
        )
        .unwrap();
        let bundle = messaging
            .build_one_time_bundle(
                StateBundleLane::Reliable,
                vec![u8::try_from(sequence).expect("test sequences are 1 and 2")],
                control,
            )
            .unwrap();
        assert_eq!(bundle.seq(), SequenceNumber::Seq(sequence));
        messaging
            .mark_one_time_bundle_queued(
                StateBundleLane::Reliable,
                bundle.seq(),
                bundle.stop_replication_ids().iter().copied(),
                std::iter::empty(),
            )
            .unwrap();
    }

    let acknowledged = messaging.acknowledge_application_sequence(
        &ApplicationLayerSeqAck::new(SequenceNumber::Seq(2), 0),
        Duration::from_millis(1),
    );
    assert_eq!(
        acknowledged.released_stop_interest_ids(),
        &[InterestId::new(11), InterestId::new(12)]
    );
    assert_eq!(
        messaging.next_available_sequence(StateBundleLane::Reliable),
        Some(SequenceNumber::Seq(3))
    );
}

#[test]
fn ack_before_first_resend_refreshes_qos_without_retiring_bundle() {
    let mut messaging =
        ClientMessaging::new(ClientContextId::new(1), BandwidthMode::new(1), ack_qos(350));
    messaging
        .begin_reusable_unreliable_bundle(vec![0xaa], ReplicationControlData::default())
        .unwrap();
    messaging
        .mark_reusable_unreliable_bundle_queued(
            SequenceNumber::Seq(1),
            Duration::ZERO,
            std::iter::empty(),
        )
        .unwrap();

    let _ = messaging.acknowledge_application_sequence(
        &ApplicationLayerSeqAck::valid_non_sequence(),
        Duration::from_millis(200),
    );
    assert!(
        messaging
            .update(Duration::from_millis(200))
            .retired_reusable_bundle()
            .is_none()
    );
    assert!(
        messaging
            .reusable_unreliable_bundle_due(Duration::from_millis(549))
            .is_none()
    );
    assert_eq!(
        messaging
            .reusable_unreliable_bundle_due(Duration::from_millis(550))
            .expect("ACK QoS threshold is measured from the latest valid input ACK")
            .kind(),
        ReusableBundleSendKind::Resend
    );
}

#[test]
fn accepted_bundles_advance_fragment_baselines_and_stopped_interests_release_them() {
    let mut messaging =
        ClientMessaging::new(ClientContextId::new(1), BandwidthMode::new(1), ack_qos(350));
    let key = FragmentSyncKey::new(InterestId::new(7), FragmentKey::new(16));
    let bundle = messaging
        .build_one_time_bundle(
            StateBundleLane::Unreliable,
            vec![0xaa],
            ReplicationControlData::default(),
        )
        .unwrap();

    messaging
        .mark_one_time_bundle_queued(
            StateBundleLane::Unreliable,
            bundle.seq(),
            std::iter::empty(),
            [FragmentBaselineUpdate::new(key, SequenceNumber::Seq(9))],
        )
        .unwrap();
    assert_eq!(messaging.fragment_baseline(key), SequenceNumber::Seq(9));

    let stop = messaging
        .build_one_time_bundle(
            StateBundleLane::Reliable,
            Vec::new(),
            ReplicationControlData::from_ids([key.interest_id()], std::iter::empty::<InterestId>())
                .unwrap(),
        )
        .unwrap();
    messaging
        .mark_one_time_bundle_queued(
            StateBundleLane::Reliable,
            stop.seq(),
            stop.stop_replication_ids().iter().copied(),
            std::iter::empty(),
        )
        .unwrap();
    let _ = messaging.acknowledge_application_sequence(
        &ApplicationLayerSeqAck::new(stop.seq(), 0),
        Duration::from_millis(1),
    );

    assert_eq!(messaging.fragment_baseline(key), SequenceNumber::Invalid);
}
