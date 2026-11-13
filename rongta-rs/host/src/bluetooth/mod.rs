//! Host Bluetooth Classic SPP backends. Each platform module provides a
//! `Connection` whose `stream` carries the printer's serial bytes, and a
//! `SystemDiscovery` for the portable scan interface.
mod address;
pub use address::Address;

#[cfg(any(target_os = "macos", windows, test))]
mod bridge;

#[cfg(target_os = "linux")]
mod bluez;
#[cfg(target_os = "linux")]
pub use bluez::{Connection, SystemDiscovery};

#[cfg(target_os = "macos")]
mod iobluetooth;
#[cfg(target_os = "macos")]
pub use iobluetooth::{Connection, SystemDiscovery};

#[cfg(windows)]
mod winsock;
#[cfg(windows)]
pub use winsock::{Connection, SystemDiscovery};

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
compile_error!("rongta-host supports Bluetooth on Linux, macOS, and Windows only");

/// A connection setting that no backend can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// Bluetooth PIN codes are 1–16 bytes; NUL would end the PIN early.
    Pin,
    Timeout,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pin => "The PIN must contain 1–16 ASCII characters without NUL",
            Self::Timeout => "The timeout must be between 1 and 3600 seconds",
        })
    }
}

impl std::error::Error for Invalid {}

/// Checks the PIN and timeout given to `Connection::connect`, which calls it
/// first; front ends can call it earlier to report problems before
/// connecting.
pub fn validate(pin: Option<&str>, timeout: std::time::Duration) -> Result<(), Invalid> {
    if let Some(pin) = pin
        && !((1..=16).contains(&pin.len()) && pin.is_ascii() && !pin.contains('\0'))
    {
        return Err(Invalid::Pin);
    }
    if !(1..=3600).contains(&timeout.as_secs()) {
        return Err(Invalid::Timeout);
    }
    Ok(())
}

/// Rejects an adapter name on hosts whose Bluetooth API has no adapter
/// selection.
#[cfg(any(target_os = "macos", windows))]
fn no_adapter_selection(adapter: Option<&str>) -> anyhow::Result<()> {
    anyhow::ensure!(
        adapter.is_none(),
        "Adapter selection is only supported on Linux"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn connection_settings_are_checked() {
        let second = Duration::from_secs(1);
        assert_eq!(validate(None, second), Ok(()));
        assert_eq!(validate(Some("0000"), Duration::from_secs(3600)), Ok(()));
        assert_eq!(validate(Some(&"9".repeat(16)), second), Ok(()));
        for pin in ["", "01234567890123456", "12\0ab", "pïn"] {
            assert_eq!(validate(Some(pin), second), Err(Invalid::Pin), "{pin:?}");
        }
        for timeout in [
            Duration::ZERO,
            Duration::from_millis(999),
            Duration::from_secs(3601),
        ] {
            assert_eq!(
                validate(None, timeout),
                Err(Invalid::Timeout),
                "{timeout:?}"
            );
        }
    }
}
