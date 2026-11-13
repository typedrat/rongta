//! One print job over Bluetooth: connect, handshake, then print.
use crate::bluetooth::{Address, Connection};
use anyhow::{Context, Result};
use rongta::{
    client,
    handshake::PrinterInfo,
    printing::{PrintOptions, PrintReport},
    raster::Raster,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Connecting,
    Handshaking,
    Printing,
}

pub struct Target<'a> {
    pub address: Address,
    /// BlueZ adapter name; must be `None` on macOS and Windows.
    pub adapter: Option<&'a str>,
    pub pin: Option<String>,
    /// Applies separately to connecting, the handshake, and printing.
    pub timeout: Duration,
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
