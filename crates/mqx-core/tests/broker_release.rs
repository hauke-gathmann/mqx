//! Real broker acceptance, run by scripts/smoke-broker.py with an isolated TLS broker.
use std::{path::PathBuf, time::Duration};

use mqx_core::{
    ConnectionProfile, LiveHandle, Protocol, RecordingHeader, ReplayJob, SessionEvent, StatusKind,
    Subscription, TlsConfig, UiConfig, load_replay_events, run_replay, save_recording,
    unix_ms_to_rfc3339,
};
use tokio::{sync::mpsc, time::timeout};

async fn wait_status(events: &mut mpsc::UnboundedReceiver<SessionEvent>, connected: bool) {
    timeout(Duration::from_secs(15), async {
        while let Some(event) = events.recv().await {
            if let SessionEvent::Status(status) = event {
                if status.status == StatusKind::Connected {
                    assert!(
                        connected,
                        "An invalid TLS/authentication connection was accepted"
                    );
                    return;
                }
                if status.error.is_some() {
                    assert!(!connected, "Valid connection failed: {:?}", status.error);
                    return;
                }
            }
        }
        panic!("Session ended before reporting connection status");
    })
    .await
    .expect("Broker connection timed out");
}

async fn wait_messages(live: &LiveHandle, count: u64) {
    timeout(Duration::from_secs(10), async {
        loop {
            if live.session().unwrap().stats_snapshot().messages_total >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Broker messages did not arrive");
}

#[tokio::test]
#[ignore = "requires the disposable TLS broker from scripts/smoke-broker.py"]
async fn tls_auth_record_save_replay_and_reconnect() {
    let profile = ConnectionProfile {
        id: "release-smoke".into(),
        name: "Release smoke test".into(),
        protocol: Protocol::Mqtts,
        host: "127.0.0.1".into(),
        port: std::env::var("MQX_TEST_BROKER_PORT")
            .unwrap()
            .parse()
            .unwrap(),
        client_id: "mqx-release-smoke".into(),
        username: "mqx-test".into(),
        tls: TlsConfig {
            ca_cert_path: Some(PathBuf::from(std::env::var("MQX_TEST_CA").unwrap())),
            ..Default::default()
        },
        session: Default::default(),
        subscriptions: vec![Subscription {
            topic: "release/#".into(),
            qos: 1,
        }],
        last_will: None,
        websocket_path: None,
    };
    let password = Some("disposable-test-password".to_string());
    let ui = UiConfig::default();
    let mut untrusted = profile.clone();
    untrusted.tls.ca_cert_path = None;
    let (invalid, mut events) = LiveHandle::spawn(untrusted, password.clone(), &ui).unwrap();
    wait_status(&mut events, false).await;
    invalid.stop().await;
    let (invalid, mut events) =
        LiveHandle::spawn(profile.clone(), Some("wrong".into()), &ui).unwrap();
    wait_status(&mut events, false).await;
    invalid.stop().await;

    let (live, mut events) = LiveHandle::spawn(profile.clone(), password.clone(), &ui).unwrap();
    wait_status(&mut events, true).await;
    let directory = tempfile::tempdir().unwrap();
    live.start_recording(directory.path().to_path_buf())
        .unwrap();
    assert!(live.has_unsaved_recording().unwrap());
    let payloads: [(&str, &[u8]); 2] = [
        ("release/json", br#"{"temperature":21.5}"#),
        ("release/binary", &[0, 255, 1, 128]),
    ];
    for (topic, payload) in payloads {
        live.client()
            .publish(topic, rumqttc::QoS::AtLeastOnce, true, payload)
            .await
            .unwrap();
    }
    wait_messages(&live, 2).await;
    assert_eq!(
        live.session()
            .unwrap()
            .get_message("release/json", None)
            .unwrap()
            .payload_json
            .unwrap()["temperature"],
        21.5
    );
    let (recorder, context) = live.take_recorder().unwrap().unwrap();
    let stopped = recorder.stop().unwrap();
    assert_eq!(stopped.messages, 2);
    assert_eq!(stopped.dropped, 0);
    let header = RecordingHeader {
        kind: mqx_core::RECORDING_KIND.into(),
        v: 1,
        started_at: unix_ms_to_rfc3339(stopped.started_ms),
        ended_at: unix_ms_to_rfc3339(stopped.ended_ms),
        profile_id: context.profile_id,
        profile_name: context.profile_name,
        broker: context.broker,
        messages: stopped.messages,
        topics: stopped.topics,
        app_version: "release-smoke".into(),
    };
    let saved = save_recording(&stopped.temp_path, directory.path(), "smoke", &header).unwrap();
    let recording = load_replay_events(&saved).unwrap();
    for (topic, payload) in payloads {
        assert_eq!(
            recording
                .iter()
                .find(|event| event.topic == topic)
                .unwrap()
                .payload_bytes()
                .unwrap(),
            payload
        );
    }
    run_replay(ReplayJob {
        client: live.client(),
        events: recording,
        file: saved.display().to_string(),
        epoch: live.epoch(),
        generation: 1,
        events_tx: live.events(),
    })
    .await;
    wait_messages(&live, 4).await;
    live.stop().await;

    let (reconnected, mut events) = LiveHandle::spawn(profile, password, &ui).unwrap();
    wait_status(&mut events, true).await;
    wait_messages(&reconnected, 2).await;
    reconnected.stop().await;
}
