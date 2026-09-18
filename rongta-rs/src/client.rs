//! Persistent framing and bounded exchanges over an RFCOMM byte stream.
use crate::{
    handshake::{Handshake, PrinterInfo},
    protocol::{Frame, FrameCodec},
};
use anyhow::{Context, Result, bail};
use bytes::BytesMut;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio_util::codec::Decoder;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub timeout: Duration,
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

struct Reader<R> {
    stream: R,
    codec: FrameCodec,
    buffer: BytesMut,
}
impl<R: AsyncRead + Unpin> Reader<R> {
    // All bytes live in self.buffer across cancellation and protocol phases.
    async fn next(&mut self) -> Result<Frame> {
        loop {
            if let Some(frame) = self
                .codec
                .decode(&mut self.buffer)
                .context("Decoding printer reply")?
            {
                return Ok(frame);
            }
            let mut chunk = [0; 4096];
            let count = self
                .stream
                .read(&mut chunk)
                .await
                .context("Reading printer replies")?;
            if count == 0 {
                if let Some(frame) = self.codec.decode_eof(&mut self.buffer)? {
                    return Ok(frame);
                }
                bail!("Printer disconnected");
            }
            tracing::debug!(direction="rx",bytes=%hex::encode(&chunk[..count]),"RFCOMM bytes received");
            self.buffer.extend_from_slice(&chunk[..count]);
        }
    }
}

/// Owns buffered replies across handshake, setup, image writes and status queries.
pub struct Session<S> {
    reader: Reader<ReadHalf<S>>,
    writer: WriteHalf<S>,
}
impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    pub fn new(stream: S) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            reader: Reader {
                stream: reader,
                codec: FrameCodec::default(),
                buffer: BytesMut::new(),
            },
            writer,
        }
    }
    pub async fn handshake(&mut self, nonce: [u8; 8], options: Options) -> Result<PrinterInfo> {
        let mut handshake = Handshake::new(nonce);
        let result = tokio::time::timeout(options.timeout, async {
            tokio::time::sleep(options.settle_delay).await;
            write_frames(
                &mut self.writer,
                &handshake.discovery(),
                Duration::ZERO,
                Duration::ZERO,
            )
            .await?;
            loop {
                let frame = self.reader.next().await?;
                let requests = handshake.accept(frame)?;
                write_frames(&mut self.writer, &requests, Duration::ZERO, Duration::ZERO).await?;
                if let Some(info) = handshake.info() {
                    return Ok(info);
                }
            }
        })
        .await;
        match result {
            Ok(result) => result,
            Err(_) => bail!(
                "Timed out after {:?}; waiting for {}",
                options.timeout,
                handshake.pending().join(", ")
            ),
        }
    }
    pub(crate) async fn next(&mut self) -> Result<Frame> {
        self.reader.next().await
    }
    /// Keep a single write future alive while draining replies. Never cancel a
    /// partial write and retry it, which could duplicate image bytes.
    pub(crate) async fn send_observing(
        &mut self,
        frames: &[Frame],
        delay: Duration,
        packet_delay: Duration,
        mut observe: impl FnMut(Frame) -> Result<()>,
    ) -> Result<usize> {
        let writing = write_frames(&mut self.writer, frames, delay, packet_delay);
        tokio::pin!(writing);
        loop {
            tokio::select! {
                biased;
                frame=self.reader.next()=>observe(frame?)?,
                result=&mut writing=>return result,
            }
        }
    }
}

async fn write_frames(
    writer: &mut (impl AsyncWrite + Unpin),
    frames: &[Frame],
    delay: Duration,
    packet_delay: Duration,
) -> Result<usize> {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let mut count = 0;
    for (i, frame) in frames.iter().enumerate() {
        if i > 0 && !packet_delay.is_zero() {
            tokio::time::sleep(packet_delay).await;
        }
        let wire = frame.encode()?;
        tracing::debug!(direction="tx",bytes=%hex::encode(&wire),"RFCOMM bytes submitted");
        writer
            .write_all(&wire)
            .await
            .context("Writing printer commands")?;
        count += wire.len();
    }
    writer.flush().await.context("Flushing printer commands")?;
    Ok(count)
}

pub async fn query_info<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    nonce: [u8; 8],
    options: Options,
) -> Result<PrinterInfo> {
    Session::new(stream).handshake(nonce, options).await
}
