//! Bounded asynchronous exchange over an RFCOMM-like byte stream.
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::Decoder;

use crate::{
    handshake::{Handshake, PrinterInfo},
    protocol::{Frame, FrameCodec},
};

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Deadline for the entire exchange, including settling and blocked writes.
    pub timeout: Duration,
    /// The Android R22 connection path waits 500 ms before discovery.
    pub settle_delay: Duration,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            settle_delay: Duration::from_millis(500),
        }
    }
}

pub async fn query_info<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    nonce: [u8; 8],
    options: Options,
) -> Result<PrinterInfo> {
    let mut handshake = Handshake::new(nonce);
    let result =
        tokio::time::timeout(options.timeout, exchange(stream, &mut handshake, options)).await;
    match result {
        Ok(result) => result,
        Err(_) => bail!(
            "Timed out after {:?}; waiting for {}",
            options.timeout,
            handshake.pending().join(", ")
        ),
    }
}

async fn send(stream: &mut (impl AsyncWrite + Unpin), frames: Vec<Frame>) -> Result<()> {
    if frames.is_empty() {
        return Ok(());
    }
    let mut wire = Vec::new();
    for frame in frames {
        wire.extend(frame.encode()?);
    }
    tracing::debug!(direction = "tx", bytes = %hex::encode(&wire), "RFCOMM bytes submitted");
    stream
        .write_all(&wire)
        .await
        .context("Writing printer commands")?;
    stream.flush().await.context("Flushing printer commands")?;
    Ok(())
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    handshake: &mut Handshake,
    options: Options,
) -> Result<PrinterInfo> {
    tokio::time::sleep(options.settle_delay).await;
    send(stream, handshake.discovery()).await?;
    let mut codec = FrameCodec::default();
    let mut buffer = BytesMut::new();
    let mut chunk = [0; 4096];
    loop {
        let count = stream
            .read(&mut chunk)
            .await
            .context("Reading printer replies")?;
        // Log bytes BEFORE decoding so malformed replies/noise remain visible.
        if count > 0 {
            tracing::debug!(direction = "rx", bytes = %hex::encode(&chunk[..count]), "RFCOMM bytes received");
        }
        buffer.extend_from_slice(&chunk[..count]);
        loop {
            let frame = if count == 0 {
                codec.decode_eof(&mut buffer)
            } else {
                codec.decode(&mut buffer)
            }
            .context("Decoding printer reply")?;
            let Some(frame) = frame else {
                break;
            };
            send(stream, handshake.accept(frame)?).await?;
            if let Some(info) = handshake.info() {
                return Ok(info);
            }
        }
        if count == 0 {
            bail!(
                "Printer disconnected; waiting for {}",
                handshake.pending().join(", ")
            );
        }
    }
}
