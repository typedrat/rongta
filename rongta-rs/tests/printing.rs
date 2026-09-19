use bytes::BytesMut;
use rongta::{
    client::{Options, Session},
    handshake::{Handshake, PrinterInfo},
    printing::{PrintOptions, PrinterStatus, command},
    protocol::{Frame, FrameCodec},
    raster::Raster,
};
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio_util::codec::Decoder;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/sdk/print.json")).unwrap()
}
fn wire(name: &str) -> Vec<u8> {
    let f = fixture();
    let v = f["commands"]
        .as_array()
        .unwrap()
        .iter()
        .chain(f["responses"].as_array().unwrap())
        .find(|v| v["name"] == name)
        .unwrap();
    hex::decode(v["wire_hex"].as_str().unwrap()).unwrap()
}
fn frame(bytes: &[u8]) -> Frame {
    FrameCodec::default()
        .decode(&mut BytesMut::from(bytes))
        .unwrap()
        .unwrap()
}
fn handshake_case() -> Value {
    serde_json::from_str::<Value>(include_str!("fixtures/sdk/handshake.json")).unwrap()["scenarios"]
        [0]
    .clone()
}
fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}
fn nonce() -> [u8; 8] {
    let s = handshake_case();
    let mut b = BytesMut::from(bytes(&s["handshake_hex"]).as_slice());
    let mut c = FrameCodec::default();
    c.decode(&mut b).unwrap();
    hex::decode(&c.decode(&mut b).unwrap().unwrap().payload[3..])
        .unwrap()
        .try_into()
        .unwrap()
}
fn info() -> PrinterInfo {
    let mut h = Handshake::new(nonce());
    for v in handshake_case()["responses"].as_array().unwrap() {
        h.accept(frame(&bytes(&v["wire_hex"]))).unwrap();
    }
    h.info().unwrap()
}
fn options() -> PrintOptions {
    PrintOptions {
        command_delay: Duration::ZERO,
        poll_interval: Duration::from_millis(10),
        ..Default::default()
    }
}
fn raster() -> Raster {
    Raster::new(8, 80, vec![0; 80]).unwrap()
}
fn image_job() -> Vec<u8> {
    let f: Value = serde_json::from_str(include_str!("fixtures/sdk/image.json")).unwrap();
    bytes(
        &f["image_vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "solid-wide")
            .unwrap()["wire_hex"],
    )
}
async fn expect(s: &mut DuplexStream, want: &[u8]) {
    let mut b = vec![0; want.len()];
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(b, want);
}
async fn serve_handshake(s: &mut DuplexStream, tail: &[u8]) {
    let case = handshake_case();
    expect(s, &bytes(&case["discovery_hex"])).await;
    let rs = case["responses"].as_array().unwrap();
    for (i, r) in rs.iter().enumerate() {
        let mut b = bytes(&r["wire_hex"]);
        if i == rs.len() - 1 {
            b.extend(tail);
        }
        s.write_all(&b).await.unwrap();
        if i == 1 {
            expect(s, &bytes(&case["handshake_hex"])).await;
        }
    }
}
async fn setup(s: &mut DuplexStream) {
    expect(s, &wire("status")).await;
    s.write_all(&wire("status-0")).await.unwrap();
    for name in ["connected", "mode-0", "density-8"] {
        expect(s, &wire(name)).await;
    }
}

#[test]
fn commands_and_status_match_sdk_reference() {
    for (name, superclass, subclass, payload) in [
        ("status", 1, 19, vec![]),
        ("connected", 1, 30, vec![1]),
        ("mode-0", 5, 41, vec![0]),
        ("density-0", 5, 4, vec![3, 8, 0]),
        ("density-8", 5, 4, vec![3, 8, 8]),
        ("density-15", 5, 4, vec![3, 8, 15]),
        ("speed-8", 5, 7, vec![8]),
        ("speed-16", 5, 7, vec![16]),
        ("feed-1", 5, 8, vec![1, 1, 0]),
        ("feed-256", 5, 8, vec![1, 0, 1]),
    ] {
        assert_eq!(
            command(superclass, subclass, payload).encode().unwrap(),
            wire(name)
        );
    }
    for r in fixture()["responses"].as_array().unwrap() {
        if r["sdk_events"][0]["success"] == false {
            assert!(
                PrinterStatus::from_frame(&frame(&bytes(&r["wire_hex"])))
                    .unwrap()
                    .is_none()
            );
            continue;
        }
        let status = PrinterStatus::from_frame(&frame(&bytes(&r["wire_hex"])))
            .unwrap()
            .unwrap();
        let sdk: Value =
            serde_json::from_str(r["sdk_events"][0]["json"].as_str().unwrap()).unwrap();
        assert_eq!(status.raw.to_string(), sdk["printStatus"]);
        assert_eq!(status.printing, status.raw & 128 != 0);
    }
}

#[tokio::test]
async fn full_handshake_then_two_copies_feed_and_busy_to_idle() {
    let (host, mut printer) = tokio::io::duplex(32);
    let server = tokio::spawn(async move {
        serve_handshake(&mut printer, &[]).await;
        setup(&mut printer).await;
        for _ in 0..2 {
            expect(&mut printer, &wire("speed-8")).await;
            expect(&mut printer, &image_job()).await;
            expect(&mut printer, &wire("feed-256")).await;
            expect(&mut printer, &wire("status")).await;
            printer.write_all(&wire("status-0")).await.unwrap();
            expect(&mut printer, &wire("status")).await;
            printer.write_all(&wire("status-128")).await.unwrap();
            expect(&mut printer, &wire("status")).await;
            printer.write_all(&wire("status-0")).await.unwrap();
        }
        let mut tail = vec![];
        printer.read_to_end(&mut tail).await.unwrap();
        assert!(tail.is_empty());
    });
    let mut session = Session::new(host);
    let info = session
        .handshake(
            nonce(),
            Options {
                settle_delay: Duration::ZERO,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let report = session
        .print(
            &info,
            &raster(),
            PrintOptions {
                copies: 2,
                feed_length: 256,
                speed: Some(8),
                ..options()
            },
        )
        .await
        .unwrap();
    assert_eq!(report.copies_sent, 2);
    assert!(report.observed_printing);
    assert_eq!(report.final_status.raw, 0);
    drop(session);
    server.await.unwrap();
}

#[tokio::test]
async fn status_coalesced_with_final_handshake_is_not_lost() {
    let (host, mut printer) = tokio::io::duplex(4096);
    let server = tokio::spawn(async move {
        serve_handshake(&mut printer, &wire("status-2")).await;
        let mut tail = vec![];
        printer.read_to_end(&mut tail).await.unwrap();
        assert!(tail.len() <= wire("status").len());
    });
    let mut session = Session::new(host);
    let info = session
        .handshake(
            nonce(),
            Options {
                settle_delay: Duration::ZERO,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let error = session
        .print(&info, &raster(), options())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("out of paper"));
    drop(session);
    server.await.unwrap();
}

#[tokio::test]
async fn fault_interrupts_a_blocked_image_write() {
    let (host, mut printer) = tokio::io::duplex(32);
    let server = tokio::spawn(async move {
        setup(&mut printer).await;
        expect(&mut printer, &image_job()[..15]).await;
        let mut fault = frame(&wire("hardware-notify-0"));
        fault.payload[3] = 2; // out of paper, with the hardware notification header
        printer.write_all(&fault.encode().unwrap()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let error = Session::new(host)
        .print(&info(), &raster(), options())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("out of paper"));
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn print_deadline_covers_silent_peer() {
    let (host, _printer) = tokio::io::duplex(1);
    let error = Session::new(host)
        .print(
            &info(),
            &raster(),
            PrintOptions {
                timeout: Duration::from_secs(1),
                ..options()
            },
        )
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("Timed out"));
}

#[tokio::test]
async fn invalid_width_or_buffer_sends_no_commands() {
    for bad_width in [true, false] {
        let (host, mut printer) = tokio::io::duplex(64);
        let mut info = info();
        if bad_width {
            info.model.width_dots = 0;
        } else {
            info.protocol.buffer_length = 65536;
        }
        assert!(
            Session::new(host)
                .print(&info, &raster(), options())
                .await
                .is_err()
        );
        let mut sent = vec![];
        printer.read_to_end(&mut sent).await.unwrap();
        assert!(sent.is_empty());
    }
}

#[tokio::test]
async fn busy_or_faulted_printer_rejects_job_before_setup() {
    for status in ["status-128", "status-1", "status-32896"] {
        let (host, mut printer) = tokio::io::duplex(64);
        let server = tokio::spawn(async move {
            expect(&mut printer, &wire("status")).await;
            printer.write_all(&wire(status)).await.unwrap();
            let mut tail = vec![];
            printer.read_to_end(&mut tail).await.unwrap();
            assert!(tail.is_empty());
        });
        assert!(
            Session::new(host)
                .print(&info(), &raster(), options())
                .await
                .is_err()
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn rejected_image_command_stops_the_job() {
    let (host, mut printer) = tokio::io::duplex(32);
    let server = tokio::spawn(async move {
        setup(&mut printer).await;
        expect(&mut printer, &image_job()[..15]).await;
        let mut rejection = command(5, 0x0b, vec![]);
        rejection.message_type = 2;
        rejection.sub_message_type = 2;
        printer
            .write_all(&rejection.encode().unwrap())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let error = Session::new(host)
        .print(&info(), &raster(), options())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("rejected command 05/0b"));
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn missing_final_status_is_not_reported_as_success() {
    let (host, mut printer) = tokio::io::duplex(32);
    let server = tokio::spawn(async move {
        setup(&mut printer).await;
        expect(&mut printer, &image_job()).await;
        expect(&mut printer, &wire("status")).await;
        let mut tail = vec![];
        printer.read_to_end(&mut tail).await.unwrap();
        assert!(tail.is_empty()); // no retransmission after a lost reply
    });
    let error = Session::new(host)
        .print(
            &info(),
            &raster(),
            PrintOptions {
                timeout: Duration::from_secs(1),
                ..options()
            },
        )
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("Timed out"));
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn initial_idle_is_not_completion_before_printing_starts() {
    let (host, mut printer) = tokio::io::duplex(4096);
    let server = tokio::spawn(async move {
        setup(&mut printer).await;
        expect(&mut printer, &image_job()).await;
        expect(&mut printer, &wire("status")).await;
        printer.write_all(&wire("status-0")).await.unwrap(); // motor hasn't started yet
        expect(&mut printer, &wire("status")).await;
        // Hardware notification can observe a short job missed by polling.
        let notification =
            hex::decode("a31e1c0010001001120b000104008000000002010000915ae3de").unwrap();
        printer.write_all(&notification).await.unwrap();
        printer.write_all(&wire("status-0")).await.unwrap();
        let mut tail = vec![];
        printer.read_to_end(&mut tail).await.unwrap();
        assert!(tail.is_empty());
    });
    let mut session = Session::new(host);
    let report = session.print(&info(), &raster(), options()).await.unwrap();
    assert!(report.observed_printing);
    drop(session);
    server.await.unwrap();
}
