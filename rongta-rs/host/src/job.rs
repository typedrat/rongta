//! One print job over Bluetooth: connect, handshake, then print.
use crate::bluetooth::{Address, Connection};
use anyhow::{Context, Result};
use rongta::{
    client,
    handshake::PrinterInfo,
    printing::{PrintOptions, PrintReport},
    raster::Raster,
};
use std::{fmt, time::Duration};

/// Progress through `print`, reported as each step begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Connecting,
    Handshaking,
    Printing,
}

/// The printer to connect to and how.
pub struct Target<'a> {
    pub address: Address,
    /// BlueZ adapter name; must be `None` on macOS and Windows.
    pub adapter: Option<&'a str>,
    /// Legacy pairing PIN, 1–16 ASCII characters. Without one, pairing uses
    /// the system's own agent or prompt.
    pub pin: Option<String>,
    /// Applies separately to connecting, the handshake, and printing.
    pub timeout: Duration,
}

// Hand-written so the PIN never reaches logs.
impl fmt::Debug for Target<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("address", &self.address)
            .field("adapter", &self.adapter)
            .field("pin", &self.pin.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// `raster` runs after the handshake because the canvas must match the
/// connected printer's head width. Bytes already sent cannot be recalled, so
/// a failure while printing is never retried.
pub async fn print(
    target: Target<'_>,
    raster: impl FnOnce(&PrinterInfo) -> Result<Raster>,
    options: PrintOptions,
    mut progress: impl FnMut(Stage),
) -> Result<(PrinterInfo, PrintReport)> {
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).context("Generating handshake nonce")?;
    progress(Stage::Connecting);
    let mut connection =
        Connection::connect(target.address, target.adapter, target.pin, target.timeout).await?;
    progress(Stage::Handshaking);
    let mut session = client::Session::new(&mut connection.stream);
    let info = session
        .handshake(
            nonce,
            client::Options {
                timeout: target.timeout,
                ..Default::default()
            },
        )
        .await?;
    tracing::info!(
        width = info.model.width_dots,
        buffer = info.protocol.buffer_length,
        "Printer handshake complete"
    );
    let raster = raster(&info)?;
    progress(Stage::Printing);
    let report = session.print(&info, &raster, options).await.context(
        "Print job failed; any bytes already sent cannot be recalled; check the printer before retrying",
    )?;
    Ok((info, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_pin() {
        let target = Target {
            address: "AA:BB:CC:DD:EE:FF".parse().unwrap(),
            adapter: None,
            pin: Some("8642".into()),
            timeout: Duration::from_secs(5),
        };
        let debug = format!("{target:?}");
        assert!(
            !debug.contains("8642") && debug.contains("redacted"),
            "{debug}"
        );
    }
}
