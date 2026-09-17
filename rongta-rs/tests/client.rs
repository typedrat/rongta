use std::time::Duration;

use bytes::BytesMut;
use rongta::{
    client::{Options, query_info},
    protocol::{Frame, FrameCodec},
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio_util::codec::Decoder;

fn scenario(index: usize) -> Value {
    serde_json::from_str::<Value>(include_str!("fixtures/sdk/handshake.json")).unwrap()["scenarios"]
        [index]
        .clone()
}
fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}
fn nonce(s: &Value) -> [u8; 8] {
    let mut wire = BytesMut::from(bytes(&s["handshake_hex"]).as_slice());
    let mut codec = FrameCodec::default();
    codec.decode(&mut wire).unwrap();
    let challenge = codec.decode(&mut wire).unwrap().unwrap();
    hex::decode(&challenge.payload[3..])
        .unwrap()
        .try_into()
        .unwrap()
}
fn options() -> Options {
    Options {
        timeout: Duration::from_secs(2),
        settle_delay: Duration::ZERO,
    }
}
async fn expect(stream: &mut DuplexStream, wire: Vec<u8>) {
    let mut got = vec![0; wire.len()];
    stream.read_exact(&mut got).await.unwrap();
    assert_eq!(got, wire);
}
async fn simulate(mut printer: DuplexStream, s: Value) {
    expect(&mut printer, bytes(&s["discovery_hex"])).await;
    let responses = s["responses"].as_array().unwrap();
    for (i, response) in responses.iter().enumerate() {
        for chunk in bytes(&response["wire_hex"]).chunks(3) {
            printer.write_all(chunk).await.unwrap();
        }
        if i == 1 {
            expect(&mut printer, bytes(&s["handshake_hex"])).await;
        }
    }
}

#[tokio::test]
async fn complete_fragmented_exchange_matches_sdk_info() {
    let s = scenario(0);
    let (mut host, printer) = tokio::io::duplex(16);
    let server = tokio::spawn(simulate(printer, s.clone()));
    let info = query_info(&mut host, nonce(&s), options()).await.unwrap();
    assert_eq!(info.serial, s["serial"]);
    assert_eq!(info.model.width_dots, 384);
    assert_eq!(info.battery.percent(), 87.5);
    server.await.unwrap();
}

#[tokio::test]
async fn failed_handshake_does_not_return_partial_info() {
    let s = scenario(2);
    let (mut host, printer) = tokio::io::duplex(16);
    let server = tokio::spawn(simulate(printer, s.clone()));
    let error = query_info(&mut host, nonce(&s), options())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("digest"));
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn deadline_covers_blocked_writes() {
    let (mut host, _printer) = tokio::io::duplex(1);
    let error = query_info(&mut host, [0; 8], options()).await.unwrap_err();
    assert!(format!("{error:#}").contains("Timed out"));
    assert!(format!("{error:#}").contains("protocol"));
}

#[tokio::test(start_paused = true)]
async fn unsolicited_frames_do_not_extend_deadline() {
    let (mut host, mut printer) = tokio::io::duplex(128);
    let server = tokio::spawn(async move {
        expect(&mut printer, bytes(&scenario(0)["discovery_hex"])).await;
        let unknown = Frame {
            mode: 0,
            message_type: 2,
            sub_message_type: 1,
            superclass: 1,
            subclass: 0xfe,
            payload: vec![],
        }
        .encode()
        .unwrap();
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if printer.write_all(&unknown).await.is_err() {
                break;
            }
        }
    });
    let start = tokio::time::Instant::now();
    let error = query_info(&mut host, [0; 8], options()).await.unwrap_err();
    assert!(format!("{error:#}").contains("Timed out"));
    assert_eq!(start.elapsed(), options().timeout);
    server.abort();
}

#[tokio::test]
async fn truncation_and_bad_crc_fail_without_retrying() {
    for corrupt in [false, true] {
        let (mut host, mut printer) = tokio::io::duplex(128);
        let server = tokio::spawn(async move {
            expect(&mut printer, bytes(&scenario(0)["discovery_hex"])).await;
            let mut reply = bytes(&scenario(0)["responses"][0]["wire_hex"]);
            if corrupt {
                *reply.last_mut().unwrap() ^= 1;
            } else {
                reply.pop();
            }
            printer.write_all(&reply).await.unwrap();
        });
        let error = query_info(&mut host, [0; 8], options()).await.unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains(if corrupt { "CRC" } else { "incomplete frame" }),
            "{message}"
        );
        server.await.unwrap();
    }
}
