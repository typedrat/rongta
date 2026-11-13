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
compile_error!("rongta-cli supports Bluetooth on Linux, macOS, and Windows only");

/// Rejects `--adapter` on hosts whose Bluetooth API has no adapter selection.
#[cfg(any(target_os = "macos", windows))]
fn no_adapter_selection(adapter: Option<&str>) -> anyhow::Result<()> {
    anyhow::ensure!(
        adapter.is_none(),
        "--adapter selects a BlueZ adapter and is only supported on Linux"
    );
    Ok(())
}
