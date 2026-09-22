//! Platform-independent Bluetooth discovery and printer detection.
//!
//! Implement [`Discovery`] using the platform's Bluetooth API, then call [`scan`]
//! to apply the shared printer policy. Identifiers belong to the backend: a MAC
//! address, UUID, or native device handle can all be used without conversion.
use anyhow::{Result, ensure};
use std::{future::Future, time::Duration};

/// Bluetooth Classic Serial Port Profile service UUID as a portable 128-bit value.
pub const SPP_UUID: u128 = 0x00001101_0000_1000_8000_00805f9b34fb;

/// Platform discovery owns adapter selection, event handling, and resource cleanup.
/// Discovery must not pair or connect devices. Return the devices still present
/// at the end of `duration`, with resolved metadata when available; cached devices
/// may be included. Backend errors are propagated by [`scan`].
///
/// The returned future need not be `Send`, allowing backends tied to a UI thread.
pub trait Discovery {
    type DeviceId;

    fn discover(
        &mut self,
        duration: Duration,
    ) -> impl Future<Output = Result<Vec<ScanDevice<Self::DeviceId>>>>;
}

/// Which discovered devices to return. Printer candidates are name matches,
/// not a guarantee that the device supports this library's R22 protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceFilter {
    #[default]
    Printers,
    All,
}

/// Discover devices through a platform backend and apply the shared name policy.
/// A zero duration fails before calling the backend. Result order is preserved.
pub async fn scan<D: Discovery>(
    discovery: &mut D,
    duration: Duration,
    filter: DeviceFilter,
) -> Result<Vec<ScanDevice<D::DeviceId>>> {
    ensure!(!duration.is_zero(), "Scan duration must be nonzero");
    let mut devices = discovery.discover(duration).await?;
    if filter == DeviceFilter::Printers {
        devices.retain(ScanDevice::is_printer_candidate);
    }
    Ok(devices)
}

/// Metadata reported by a platform backend; cached devices may be out of range.
/// `Id` is the backend's identifier, not necessarily a Bluetooth MAC address.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ScanDevice<Id> {
    pub address: Id,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    pub class: Option<u32>,
    pub paired: bool,
    pub connected: bool,
    /// None means service UUIDs are unavailable; false is not proof of no SPP.
    pub spp: Option<bool>,
    pub known_before_scan: bool,
}

impl<Id> ScanDevice<Id> {
    /// RONGTA 3.1.6's normal-channel name filter, not a compatibility check.
    /// This applies the stock name rules only, without Android device-type or
    /// app-channel gates. It does not establish protocol compatibility.
    pub fn is_printer_candidate(&self) -> bool {
        self.name.as_deref().is_some_and(stock_printer_name)
    }
}

fn stock_printer_name(name: &str) -> bool {
    // SearchBluetoothDeviceActivity.filterDevice, PrintModel.values(), and qh.j0.d.
    // Keep this list independent of the models supported by our print protocol.
    const MODELS: &[&str] = &[
        "PN81", "F81", "F82", "F81A", "F81S", "F82A", "F11", "F12", "R5", "R22", "TM95", "TD210",
        "TH110", "RP425", "RP420", "RP421A", "RP415", "R26", "X21", "RP310D", "T11", "T12", "X11",
    ];
    const VARIANT_PREFIXES: &[&str] = &[
        "R5", "R22", "R26", "TM95", "TD210", "TH110", "RP425", "RP420", "RP415", "RP310D",
    ];
    let upper = name.to_uppercase();
    if name.trim().is_empty() || upper.ends_with("BLE") {
        return false;
    }
    // Java's regex dot excludes all of these, even after a name separator.
    if name.contains(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}']) {
        return false;
    }
    // Stock checks the original case before using an uppercase regex match.
    let allow_variant = VARIANT_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix));
    MODELS.iter().any(|model| {
        let Some(suffix) = upper.strip_prefix(model) else {
            return false;
        };
        let variant = suffix.split(['-', '_', ' ']).next().unwrap_or_default();
        // Equivalent to ^MODEL[A-Z0-9]{0,3}([-_ ].*)?$ (or no variant).
        variant.is_empty()
            || (allow_variant
                && variant.len() <= 3
                && variant.bytes().all(|c| c.is_ascii_alphanumeric()))
    })
}
