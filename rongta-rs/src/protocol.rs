//! Rongta protocol 28 cleartext framing, checked against original SDK output.
//!
//! Mode 0 and mode 1 are emitted as cleartext by the SDK. Mode 2 (AES) is not
//! supported. The CRC covers the inner header and payload, excluding the first
//! six bytes. Printer behavior and response semantics remain unverified.

use bytes::{Buf, BytesMut};
use deku::prelude::*;
use tokio_util::codec::{Decoder, Encoder};

const MAGIC: &[u8; 3] = b"\xa3\x1e\x1c";
const ENVELOPE_LEN: usize = 6;
const INNER_HEADER_LEN: usize = 5;
const HEADER_LEN: usize = ENVELOPE_LEN + INNER_HEADER_LEN;
const CRC_LEN: usize = 4;

/// Largest payload representable without overflowing the outer u16 length.
pub const MAX_PAYLOAD_LEN: usize = u16::MAX as usize - 5;

// The SDK starts its reflected shift register at ~0x76953521 = 0x896acade.
// `crc` reflects Algorithm::init for refin=true, so the configured value must
// be bit-reversed. Equivalently: zlib.crc32(inner_message, 0x76953521).
const CRC_ALGORITHM: crc::Algorithm<u32> = crc::Algorithm {
    init: 0x896a_cade_u32.reverse_bits(),
    check: 0xf3d8_4598,
    ..crc::CRC_32_ISO_HDLC
};
const CRC: crc::Crc<u32> = crc::Crc::<u32>::new(&CRC_ALGORITHM);

#[derive(Debug, DekuRead, DekuWrite)]
#[deku(magic = b"\xa3\x1e\x1c", endian = "little")]
struct Envelope {
    #[deku(bits = 4)]
    mode: u8,
    #[deku(bits = 4)]
    reserved: u8,
    body_len: u16,
}

#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "little")]
struct InnerHeader {
    #[deku(bits = 4)]
    message_type: u8,
    #[deku(bits = 4)]
    sub_message_type: u8,
    superclass: u8,
    subclass: u8,
    payload_len: u16,
}

/// A cleartext message. Unknown command and message-type values are preserved.
/// Lengths and CRC are derived at encoding time, never stored as mutable fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub mode: u8,
    pub message_type: u8,
    pub sub_message_type: u8,
    pub superclass: u8,
    pub subclass: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Layout(#[from] DekuError),
    #[error("unsupported packet mode {0}; only cleartext modes 0 and 1 are implemented")]
    UnsupportedMode(u8),
    #[error("nonzero reserved envelope bits: {0:#x}")]
    ReservedBits(u8),
    #[error("{field} value {value} exceeds four bits")]
    InvalidNibble { field: &'static str, value: u8 },
    #[error("payload length {0} exceeds the wire-format limit")]
    PayloadTooLarge(usize),
    #[error("inner-message length {0} is shorter than its five-byte header")]
    BodyTooShort(usize),
    #[error("inner-message length {length} exceeds configured limit {limit}")]
    BodyTooLarge { length: usize, limit: u16 },
    #[error("outer body length {body_len} disagrees with inner payload length {payload_len}")]
    LengthMismatch { body_len: usize, payload_len: usize },
    #[error("CRC mismatch: received {actual:08x}, calculated {expected:08x}")]
    CrcMismatch { expected: u32, actual: u32 },
    #[error("stream ended with {0} bytes of an incomplete frame")]
    Truncated(usize),
}

impl Frame {
    /// Encode without a device-specific buffer limit. Use the codec's Encoder
    /// implementation when enforcing a negotiated limit on outgoing frames.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        validate_mode(self.mode)?;
        for (field, value) in [
            ("message_type", self.message_type),
            ("sub_message_type", self.sub_message_type),
        ] {
            if value > 15 {
                return Err(Error::InvalidNibble { field, value });
            }
        }
        if self.payload.len() > MAX_PAYLOAD_LEN {
            return Err(Error::PayloadTooLarge(self.payload.len()));
        }
        let envelope = Envelope {
            mode: self.mode,
            reserved: 0,
            body_len: (INNER_HEADER_LEN + self.payload.len()) as u16,
        };
        let inner = InnerHeader {
            message_type: self.message_type,
            sub_message_type: self.sub_message_type,
            superclass: self.superclass,
            subclass: self.subclass,
            payload_len: self.payload.len() as u16,
        };
        let mut wire = Vec::with_capacity(HEADER_LEN + self.payload.len() + CRC_LEN);
        wire.extend(envelope.to_bytes()?);
        wire.extend(inner.to_bytes()?);
        wire.extend_from_slice(&self.payload);
        let checksum = CRC.checksum(&wire[ENVELOPE_LEN..]);
        wire.extend_from_slice(&checksum.to_le_bytes());
        Ok(wire)
    }
}

fn validate_mode(mode: u8) -> Result<(), Error> {
    if mode > 1 {
        return Err(Error::UnsupportedMode(mode));
    }
    Ok(())
}

/// Incremental framing with bounded lengths. Noise before magic is discarded;
/// a partial magic prefix is retained. A recognized but invalid frame produces
/// an error without consuming it; treat this as a failed stream, not an implicit
/// retry. Raw transport logging should happen before bytes enter this codec.
pub struct FrameCodec {
    max_body_len: u16,
}

impl FrameCodec {
    /// Limit counts the five-byte inner header plus payload, excluding the
    /// six-byte envelope and CRC. This is not a BLE MTU or a printer width.
    pub fn new(max_body_len: u16) -> Self {
        Self { max_body_len }
    }

    fn check_body_len(&self, body_len: usize) -> Result<(), Error> {
        if body_len < INNER_HEADER_LEN {
            return Err(Error::BodyTooShort(body_len));
        }
        if body_len > usize::from(self.max_body_len) {
            return Err(Error::BodyTooLarge {
                length: body_len,
                limit: self.max_body_len,
            });
        }
        Ok(())
    }
}

impl Default for FrameCodec {
    fn default() -> Self {
        // Native HAL_RT_Packet_Decode's receive bound. Hardware negotiation may
        // establish a different outgoing limit; it must be applied explicitly.
        Self::new(0x802)
    }
}

impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Frame>, Error> {
        if let Some(offset) = src.windows(MAGIC.len()).position(|bytes| bytes == MAGIC) {
            src.advance(offset);
        } else {
            let keep = if src.ends_with(&MAGIC[..2]) {
                2
            } else if src.ends_with(&MAGIC[..1]) {
                1
            } else {
                0
            };
            src.advance(src.len() - keep);
            return Ok(None);
        }
        if src.len() < ENVELOPE_LEN {
            return Ok(None);
        }
        let envelope = Envelope::try_from(&src[..ENVELOPE_LEN])?;
        validate_mode(envelope.mode)?;
        if envelope.reserved != 0 {
            return Err(Error::ReservedBits(envelope.reserved));
        }
        let body_len = usize::from(envelope.body_len);
        self.check_body_len(body_len)?;
        if src.len() < HEADER_LEN {
            return Ok(None);
        }
        let inner = InnerHeader::try_from(&src[ENVELOPE_LEN..HEADER_LEN])?;
        let payload_len = usize::from(inner.payload_len);
        if payload_len + INNER_HEADER_LEN != body_len {
            return Err(Error::LengthMismatch {
                body_len,
                payload_len,
            });
        }
        let crc_offset = ENVELOPE_LEN + body_len;
        let total_len = crc_offset + CRC_LEN;
        if src.len() < total_len {
            src.reserve(total_len - src.len());
            return Ok(None);
        }
        let expected = CRC.checksum(&src[ENVELOPE_LEN..crc_offset]);
        let actual = u32::from_le_bytes(src[crc_offset..total_len].try_into().unwrap());
        if expected != actual {
            return Err(Error::CrcMismatch { expected, actual });
        }
        let wire = src.split_to(total_len);
        Ok(Some(Frame {
            mode: envelope.mode,
            message_type: inner.message_type,
            sub_message_type: inner.sub_message_type,
            superclass: inner.superclass,
            subclass: inner.subclass,
            payload: wire[HEADER_LEN..crc_offset].to_vec(),
        }))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Frame>, Error> {
        match self.decode(src)? {
            None if !src.is_empty() => Err(Error::Truncated(src.len())),
            result => Ok(result),
        }
    }
}

impl Encoder<Frame> for FrameCodec {
    type Error = Error;

    fn encode(&mut self, frame: Frame, dst: &mut BytesMut) -> Result<(), Error> {
        self.check_body_len(frame.payload.len().saturating_add(INNER_HEADER_LEN))?;
        dst.extend_from_slice(&frame.encode()?);
        Ok(())
    }
}
