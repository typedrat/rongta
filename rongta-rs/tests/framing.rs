use bytes::BytesMut;
use proptest::prelude::*;
use rongta::protocol::{Frame, FrameCodec, MAX_PAYLOAD_LEN};
use serde::Deserialize;
use tokio_util::codec::{Decoder, Encoder};

#[derive(Deserialize)]
struct Corpus {
    vectors: Vec<Vector>,
}

#[derive(Deserialize)]
struct Vector {
    name: String,
    method: String,
    superclass: Option<u8>,
    subclass: Option<u8>,
    payload_hex: Option<String>,
    wire_hex: String,
}

fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn references() -> Vec<(String, Vec<Frame>, Vec<u8>)> {
    let corpus: Corpus = serde_json::from_str(include_str!("fixtures/sdk/framing.json")).unwrap();
    corpus
        .vectors
        .into_iter()
        .map(|vector| {
            let frames = if vector.method == "getProtocolData" {
                vec![command(0, 1, 1, vec![]), command(0, 1, 6, vec![])]
            } else {
                vec![command(
                    1,
                    vector.superclass.unwrap(),
                    vector.subclass.unwrap(),
                    hex(vector.payload_hex.as_ref().unwrap()),
                )]
            };
            (vector.name, frames, hex(&vector.wire_hex))
        })
        .collect()
}

fn command(mode: u8, superclass: u8, subclass: u8, payload: Vec<u8>) -> Frame {
    Frame {
        mode,
        message_type: 1,
        sub_message_type: 1,
        superclass,
        subclass,
        payload,
    }
}

#[test]
fn encoder_matches_every_sdk_vector_byte_for_byte() {
    for (name, frames, expected) in references() {
        let mut actual = Vec::new();
        for frame in frames {
            actual.extend(frame.encode().unwrap());
        }
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn decoder_matches_sdk_frames_at_every_possible_two_part_split() {
    for (name, expected, wire) in references() {
        for split in 0..=wire.len() {
            let mut codec = FrameCodec::new(u16::MAX);
            let mut buffer = BytesMut::new();
            let mut actual = Vec::new();
            for part in [&wire[..split], &wire[split..]] {
                buffer.extend_from_slice(part);
                while let Some(frame) = codec.decode(&mut buffer).unwrap() {
                    actual.push(frame);
                }
            }
            assert_eq!(actual, expected, "{name} at {split}");
            assert!(buffer.is_empty());
        }
    }
}

#[test]
fn skips_noise_and_preserves_partial_magic_and_unknown_commands() {
    let (_, expected, wire) = references()
        .into_iter()
        .find(|v| v.0 == "embedded-magic")
        .unwrap();
    let mut codec = FrameCodec::default();
    let mut buffer = BytesMut::from(&b"noise\xa3\xa3\x1e"[..]);
    assert!(codec.decode(&mut buffer).unwrap().is_none());
    assert_eq!(&buffer[..], &[0xa3, 0x1e]);
    buffer.extend_from_slice(&wire[2..]);
    assert_eq!(
        codec.decode(&mut buffer).unwrap(),
        Some(expected[0].clone())
    );
    assert!(buffer.is_empty());
}

#[test]
fn rejects_corruption_of_each_crc_covered_byte_and_each_crc_byte() {
    let (_, _, wire) = references()
        .into_iter()
        .find(|v| v.0 == "embedded-magic")
        .unwrap();
    for index in 6..wire.len() {
        let mut damaged = wire.clone();
        damaged[index] ^= 1;
        assert!(
            FrameCodec::default()
                .decode(&mut BytesMut::from(&damaged[..]))
                .is_err(),
            "accepted corruption at {index}"
        );
    }
}

#[test]
fn validates_outer_length_and_mode_before_waiting_for_body() {
    for header in [
        [0xa3, 0x1e, 0x1c, 0, 4, 0],    // body shorter than its fixed header
        [0xa3, 0x1e, 0x1c, 0, 3, 8],    // exceeds SDK receive limit 0x802
        [0xa3, 0x1e, 0x1c, 0x20, 5, 0], // AES is unsupported
        [0xa3, 0x1e, 0x1c, 1, 5, 0],    // reserved low nibble
    ] {
        assert!(
            FrameCodec::default()
                .decode(&mut BytesMut::from(&header[..]))
                .is_err()
        );
    }
}

#[test]
fn validates_inner_length_before_waiting_for_payload() {
    let header = hex("a31e1c0006001101010000");
    assert!(
        FrameCodec::default()
            .decode(&mut BytesMut::from(&header[..]))
            .is_err()
    );
}

#[test]
fn eof_reports_partial_frame_but_accepts_complete_stream() {
    let (_, _, wire) = references().remove(0);
    // One complete discovery frame is 15 bytes.
    for length in 1..15 {
        assert!(
            FrameCodec::default()
                .decode_eof(&mut BytesMut::from(&wire[..length]))
                .is_err()
        );
    }
    let mut codec = FrameCodec::default();
    let mut buffer = BytesMut::from(&wire[..]);
    assert!(codec.decode_eof(&mut buffer).unwrap().is_some());
    assert!(codec.decode_eof(&mut buffer).unwrap().is_some());
    assert!(codec.decode_eof(&mut buffer).unwrap().is_none());
}

#[test]
fn bounds_encode_and_leaves_destination_unchanged_on_failure() {
    let mut codec = FrameCodec::default();
    let mut output = BytesMut::from(&b"existing"[..]);
    assert!(
        codec
            .encode(command(0, 1, 9, vec![0; 2046]), &mut output)
            .is_err()
    );
    assert_eq!(&output[..], b"existing");
    assert!(
        command(0, 1, 9, vec![0; MAX_PAYLOAD_LEN + 1])
            .encode()
            .is_err()
    );
    for (mode, message_type, sub_message_type) in [(2, 1, 1), (0, 16, 1), (0, 1, 16)] {
        let mut frame = command(mode, 1, 9, vec![]);
        frame.message_type = message_type;
        frame.sub_message_type = sub_message_type;
        assert!(frame.encode().is_err());
    }
}

#[test]
fn maximum_wire_length_does_not_overflow_and_lengths_are_recomputed() {
    let mut frame = command(0, 1, 9, vec![0; MAX_PAYLOAD_LEN]);
    let bytes = frame.encode().unwrap();
    assert_eq!(bytes.len(), u16::MAX as usize + 10);
    assert_eq!(&bytes[4..6], &[0xff, 0xff]);
    let mut codec = FrameCodec::new(u16::MAX);
    assert_eq!(
        codec.decode(&mut BytesMut::from(&bytes[..])).unwrap(),
        Some(frame.clone())
    );
    frame.payload = vec![0x42];
    let smaller = frame.encode().unwrap();
    assert_eq!(&smaller[4..6], &[6, 0]);
    assert_eq!(&smaller[9..11], &[1, 0]);
}

proptest! {
    #[test]
    fn arbitrary_frames_round_trip_in_arbitrary_chunks(
        mode in 0u8..=1, message_type in 0u8..=15, sub_message_type in 0u8..=15,
        superclass in any::<u8>(), subclass in any::<u8>(),
        payload in prop::collection::vec(any::<u8>(), 0..1024), chunk_size in 1usize..64,
    ) {
        let frame = Frame { mode, message_type, sub_message_type, superclass, subclass, payload };
        let wire = frame.encode().unwrap();
        let mut buffer = BytesMut::new();
        let mut codec = FrameCodec::default();
        let mut received = Vec::new();
        for chunk in wire.chunks(chunk_size) {
            buffer.extend_from_slice(chunk);
            while let Some(frame) = codec.decode(&mut buffer).unwrap() {
                received.push(frame);
            }
        }
        prop_assert_eq!(received, vec![frame]);
        prop_assert!(buffer.is_empty());
    }
}
