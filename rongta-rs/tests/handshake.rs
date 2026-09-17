use bytes::BytesMut;
use rongta::{
    handshake::Handshake,
    protocol::{Frame, FrameCodec},
};
use serde_json::Value;
use tokio_util::codec::Decoder;

fn scenarios() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("fixtures/sdk/handshake.json")).unwrap()["scenarios"]
        .as_array()
        .unwrap()
        .clone()
}
fn frames(hex: &str) -> Vec<Frame> {
    let mut bytes = BytesMut::from(hex::decode(hex).unwrap().as_slice());
    let mut codec = FrameCodec::default();
    let mut frames = Vec::new();
    while let Some(frame) = codec.decode_eof(&mut bytes).unwrap() {
        frames.push(frame);
    }
    frames
}
fn replies(s: &Value) -> Vec<Frame> {
    s["responses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| frames(r["wire_hex"].as_str().unwrap()).remove(0))
        .collect()
}
fn nonce(s: &Value) -> [u8; 8] {
    let commands = frames(s["handshake_hex"].as_str().unwrap());
    hex::decode(&commands[1].payload[3..])
        .unwrap()
        .try_into()
        .unwrap()
}

#[test]
fn sdk_requests_and_completion_match_for_two_serials() {
    for s in &scenarios()[..2] {
        let mut handshake = Handshake::new(nonce(s));
        assert_eq!(
            handshake.discovery(),
            frames(s["discovery_hex"].as_str().unwrap())
        );
        let responses = replies(s);
        for (index, reply) in responses.into_iter().enumerate() {
            let requests = handshake.accept(reply).unwrap();
            if index == 1 {
                assert_eq!(requests, frames(s["handshake_hex"].as_str().unwrap()));
                assert_eq!(
                    requests[0].payload,
                    s["expected_proof_ascii"].as_str().unwrap().as_bytes()
                );
            } else {
                assert!(requests.is_empty());
            }
            assert_eq!(handshake.info().is_some(), index == 6);
        }
        let info = handshake.info().unwrap();
        let sdk: Value =
            serde_json::from_str(s["responses"][6]["sdk_events"][0]["json"].as_str().unwrap())
                .unwrap();
        assert_eq!(info.serial, s["serial"]);
        assert_eq!(info.device.name, sdk["devName"]);
        assert_eq!(info.device.manufacturer, sdk["manufacturer"]);
        assert_eq!(info.device.firmware, sdk["firmware"]);
        assert_eq!(info.model.width_dots.to_string(), sdk["widthMaxDot"]);
        assert_eq!(info.model.dpi.to_string(), sdk["dpi"]);
        assert_eq!(info.battery.voltage().to_string(), sdk["batteryVoltage"]);
        assert_eq!(info.battery.percent().to_string(), sdk["batteryPercent"]);
        assert_eq!(info.challenge.code, sdk["shakeHandCode"]);
        assert_eq!(info.challenge.auth_code, sdk["shakeHandAuthCode"]);
    }
}

#[test]
fn wrong_digest_fails_like_sdk_and_failure_is_terminal() {
    let s = &scenarios()[2];
    assert_eq!(s["responses"][6]["sdk_events"][0]["success"], false);
    let mut handshake = Handshake::new(nonce(s));
    let responses = replies(s);
    for reply in &responses[..6] {
        handshake.accept(reply.clone()).unwrap();
    }
    assert!(
        format!("{:#}", handshake.accept(responses[6].clone()).unwrap_err()).contains("digest")
    );
    assert!(handshake.info().is_none());
    assert!(
        handshake
            .accept(replies(&scenarios()[0])[6].clone())
            .is_err()
    );
}

#[test]
fn reordering_duplicates_and_unknown_commands_do_not_retrigger_requests() {
    let s = &scenarios()[0];
    let responses = replies(s);
    let mut handshake = Handshake::new(nonce(s));
    let mut unknown = responses[0].clone();
    unknown.subclass = 0xfe;
    assert!(handshake.accept(unknown).unwrap().is_empty());
    assert!(handshake.accept(responses[1].clone()).unwrap().is_empty());
    assert_eq!(handshake.accept(responses[0].clone()).unwrap().len(), 5);
    assert!(handshake.accept(responses[0].clone()).unwrap().is_empty());
    for reply in responses[2..].iter().rev() {
        handshake.accept(reply.clone()).unwrap();
    }
    assert!(handshake.info().is_some());
}

#[test]
fn negative_ack_and_malformed_required_fields_fail() {
    let s = &scenarios()[0];
    let responses = replies(s);
    for (index, payload) in [
        (0, vec![1, 4, 0, 1]),             // truncated TLV
        (0, vec![1, 1, 0, 1, 1, 1, 0, 1]), // duplicate tag
        (0, vec![1, 1, 0, 1]),             // missing fields
        (1, vec![]),                       // empty serial
        (1, b"bad\0serial".to_vec()),
        (3, b"{}".to_vec()),               // missing device info
        (4, vec![]),                       // missing model
        (5, vec![1, 5, 0, 0, 0, 0, 0, 0]), // oversized integer
        (6, vec![]),                       // missing challenge
    ] {
        let mut handshake = Handshake::new(nonce(s));
        for reply in &responses[..index] {
            handshake.accept(reply.clone()).unwrap();
        }
        let mut bad = responses[index].clone();
        bad.payload = payload;
        assert!(handshake.accept(bad).is_err(), "reply {index}");
        assert!(handshake.info().is_none());
    }
    for index in 0..responses.len() {
        let mut handshake = Handshake::new(nonce(s));
        for reply in &responses[..index] {
            handshake.accept(reply.clone()).unwrap();
        }
        let mut bad = responses[index].clone();
        bad.sub_message_type = 2;
        assert!(
            handshake
                .accept(bad)
                .unwrap_err()
                .to_string()
                .contains("rejected")
        );
    }
}

#[test]
fn reported_algorithm_does_not_change_native_cleartext_handshake() {
    let mut handshake = Handshake::new([0; 8]);
    let mut protocol = replies(&scenarios()[0])[0].clone();
    protocol.payload[8] = 1; // Native getShakeData does not branch on pAlgo.
    handshake.accept(protocol).unwrap();
    let requests = handshake
        .accept(replies(&scenarios()[0])[1].clone())
        .unwrap();
    assert!(requests.iter().all(|frame| frame.mode == 0));
}
