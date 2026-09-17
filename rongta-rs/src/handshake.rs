//! SDK protocol-28 discovery, serial proof, and printer information handshake.
//! See notes/handshake.md for wire layouts and limits of the reference evidence.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use deku::prelude::*;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use crate::protocol::Frame;

const STEPS: [(u8, &str); 7] = [
    (1, "protocol"),
    (6, "serial"),
    (2, "proof acknowledgement"),
    (0x17, "challenge"),
    (3, "device info"),
    (4, "model"),
    (0x0e, "battery"),
];

#[derive(Clone, Debug, Serialize)]
pub struct ProtocolInfo {
    pub version: u32,
    pub algorithm: u32,
    /// Reported pBuffLength; its relation to frame/body limits is not established.
    pub buffer_length: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceInfo {
    #[serde(rename(deserialize = "DevName"))]
    pub name: String,
    #[serde(rename(deserialize = "Firmware"))]
    pub firmware: String,
    #[serde(rename(deserialize = "HWVer"))]
    pub hardware_version: String,
    #[serde(rename(deserialize = "SN"))]
    pub serial: String,
    #[serde(rename(deserialize = "Manufacturer"), default = "manufacturer")]
    pub manufacturer: String,
}
fn manufacturer() -> String {
    "Rongta".into()
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelInfo {
    pub product_type: u32,
    pub supported_paper_size: u32,
    pub dpi: u32,
    pub width_dots: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatteryInfo {
    pub voltage_hundredths: u32,
    pub percent_tenths: u32,
    pub state: u32,
}
impl BatteryInfo {
    pub fn voltage(&self) -> f64 {
        self.voltage_hundredths as f64 / 100.0
    }
    pub fn percent(&self) -> f64 {
        self.percent_tenths as f64 / 10.0
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ChallengeInfo {
    pub code: String,
    pub md5: String,
    pub serial: String,
    pub auth_code: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrinterInfo {
    pub serial: String,
    pub protocol: ProtocolInfo,
    pub device: DeviceInfo,
    pub model: ModelInfo,
    pub battery: BatteryInfo,
    pub challenge: ChallengeInfo,
}

#[derive(DekuRead)]
#[deku(endian = "little")]
struct Tlv {
    tag: u8,
    _length: u16,
    #[deku(count = "_length")]
    value: Vec<u8>,
}

fn tlvs(mut bytes: &[u8]) -> Result<BTreeMap<u8, Vec<u8>>> {
    let mut fields = BTreeMap::new();
    while !bytes.is_empty() {
        let ((rest, _), tlv) = Tlv::from_bytes((bytes, 0)).context("Malformed TLV")?;
        ensure!(
            fields.insert(tlv.tag, tlv.value).is_none(),
            "Duplicate TLV tag {}",
            tlv.tag
        );
        bytes = rest;
    }
    Ok(fields)
}
fn field(fields: &BTreeMap<u8, Vec<u8>>, tag: u8) -> Result<&[u8]> {
    fields
        .get(&tag)
        .map(Vec::as_slice)
        .with_context(|| format!("Missing TLV tag {tag}"))
}
fn number(fields: &BTreeMap<u8, Vec<u8>>, tag: u8) -> Result<u32> {
    let bytes = field(fields, tag)?;
    ensure!(
        (1..=4).contains(&bytes.len()),
        "TLV tag {tag} must contain a 1–4 byte integer"
    );
    let mut padded = [0; 4];
    padded[..bytes.len()].copy_from_slice(bytes);
    Ok(u32::from_le_bytes(padded))
}
fn text(bytes: &[u8]) -> Result<String> {
    ensure!(
        !bytes.is_empty() && !bytes.contains(&0),
        "Text must be nonempty and contain no NUL"
    );
    Ok(std::str::from_utf8(bytes)
        .context("Non-UTF-8 text")?
        .to_owned())
}
fn string(fields: &BTreeMap<u8, Vec<u8>>, tag: u8) -> Result<String> {
    text(field(fields, tag)?)
}

fn proof(serial: &str) -> String {
    let hash = Md5::new()
        .chain_update(serial.as_bytes())
        .chain_update(b"D932A43C3084B1D3")
        .finalize();
    hex::encode(&hash[4..12])
}
fn request(subclass: u8, payload: Vec<u8>) -> Frame {
    Frame {
        mode: 0,
        message_type: 1,
        sub_message_type: 1,
        superclass: 1,
        subclass,
        payload,
    }
}

/// One exchange. Send discovery() once, then send any requests returned by accept().
/// Completion requires all seven successful replies. Errors are terminal.
/// Unrelated commands are ignored; identical duplicates are harmless.
pub struct Handshake {
    nonce: [u8; 8],
    sent_handshake: bool,
    failed: bool,
    seen: BTreeMap<u8, Vec<u8>>,
    protocol: Option<ProtocolInfo>,
    serial: Option<String>,
    device: Option<DeviceInfo>,
    model: Option<ModelInfo>,
    battery: Option<BatteryInfo>,
    challenge: Option<ChallengeInfo>,
}
impl Handshake {
    pub fn new(nonce: [u8; 8]) -> Self {
        Self {
            nonce,
            sent_handshake: false,
            failed: false,
            seen: BTreeMap::new(),
            protocol: None,
            serial: None,
            device: None,
            model: None,
            battery: None,
            challenge: None,
        }
    }
    pub fn discovery(&self) -> Vec<Frame> {
        vec![request(1, vec![]), request(6, vec![])]
    }
    pub fn pending(&self) -> Vec<&'static str> {
        STEPS
            .iter()
            .filter(|(code, _)| !self.seen.contains_key(code))
            .map(|(_, name)| *name)
            .collect()
    }
    pub fn info(&self) -> Option<PrinterInfo> {
        if self.failed || !self.pending().is_empty() {
            return None;
        }
        Some(PrinterInfo {
            serial: self.serial.clone()?,
            protocol: self.protocol.clone()?,
            device: self.device.clone()?,
            model: self.model.clone()?,
            battery: self.battery.clone()?,
            challenge: self.challenge.clone()?,
        })
    }
    pub fn accept(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        ensure!(
            !self.failed,
            "Handshake already failed; reconnect before retrying"
        );
        let result = self.accept_inner(frame);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn accept_inner(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        let Some((_, name)) = STEPS.iter().find(|(code, _)| *code == frame.subclass) else {
            return Ok(vec![]);
        };
        if frame.superclass != 1 || frame.message_type != 2 {
            return Ok(vec![]);
        }
        // The SDK requests these only after receiving protocol + serial.
        if !self.sent_handshake && ![1, 6].contains(&frame.subclass) {
            return Ok(vec![]);
        }
        ensure!(
            frame.sub_message_type == 1,
            "Printer rejected {name} (reply subtype {})",
            frame.sub_message_type
        );
        if let Some(previous) = self.seen.get(&frame.subclass) {
            ensure!(
                *previous == frame.payload,
                "Conflicting duplicate {name} reply"
            );
            return Ok(vec![]);
        }
        self.parse_reply(frame.subclass, &frame.payload)
            .with_context(|| format!("Invalid {name} reply"))?;
        self.seen.insert(frame.subclass, frame.payload);
        if !self.sent_handshake
            && self.protocol.is_some()
            && let Some(serial) = &self.serial
        {
            self.sent_handshake = true;
            let mut challenge = vec![1, 16, 0];
            challenge.extend(hex::encode(self.nonce).as_bytes());
            return Ok(vec![
                request(2, proof(serial).into_bytes()),
                request(0x17, challenge),
                request(3, vec![]),
                request(4, vec![]),
                request(0x0e, vec![]),
            ]);
        }
        Ok(vec![])
    }
    fn parse_reply(&mut self, code: u8, payload: &[u8]) -> Result<()> {
        match code {
            1 => {
                let fields = tlvs(payload)?;
                self.protocol = Some(ProtocolInfo {
                    version: number(&fields, 1)?,
                    algorithm: number(&fields, 2)?,
                    buffer_length: number(&fields, 3)?,
                });
            }
            6 => {
                // The native serial-key buffer is 200 bytes including its terminator.
                ensure!(payload.len() < 200, "Serial exceeds native key buffer");
                self.serial = Some(text(payload)?);
            }
            2 => {} // Native code only checks the successful reply header.
            3 => {
                let mut info: DeviceInfo =
                    serde_json::from_slice(payload).context("Device info JSON")?;
                for value in [
                    &info.name,
                    &info.firmware,
                    &info.hardware_version,
                    &info.serial,
                ] {
                    text(value.as_bytes())?;
                }
                if info.manufacturer.is_empty() {
                    info.manufacturer = manufacturer();
                }
                self.device = Some(info);
            }
            4 => {
                let fields = tlvs(payload)?;
                self.model = Some(ModelInfo {
                    product_type: number(&fields, 1)?,
                    supported_paper_size: number(&fields, 2)?,
                    dpi: number(&fields, 3)?,
                    width_dots: number(&fields, 4)?,
                });
            }
            0x0e => {
                let fields = tlvs(payload)?;
                self.battery = Some(BatteryInfo {
                    voltage_hundredths: number(&fields, 1)?,
                    percent_tenths: number(&fields, 2)?,
                    state: number(&fields, 3)?,
                });
            }
            0x17 => {
                let fields = tlvs(payload)?;
                let challenge = ChallengeInfo {
                    code: string(&fields, 1)?,
                    md5: string(&fields, 2)?,
                    serial: string(&fields, 3)?,
                    auth_code: string(&fields, 4)?,
                };
                let expected = hex::encode(
                    Md5::new()
                        .chain_update(challenge.code.as_bytes())
                        .chain_update(proof(
                            self.serial.as_ref().context("Challenge before serial")?,
                        ))
                        .finalize(),
                );
                // Native isCheckMD5Success hashes the RETURNED code. It does not
                // require an echo of our nonce or equality of the two serials.
                if challenge.md5 != expected {
                    bail!("Challenge digest mismatch");
                }
                self.challenge = Some(challenge);
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
