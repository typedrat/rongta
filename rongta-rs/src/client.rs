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
    let mut batch = Vec::new();
    for (i, frame) in frames.iter().enumerate() {
        let wire = frame.encode()?;
        if packet_delay.is_zero() {
            batch.extend(wire);
        } else {
            if i > 0 {
                tokio::time::sleep(packet_delay).await;
            }
            tracing::debug!(direction="tx",bytes=%hex::encode(&wire),"RFCOMM bytes submitted");
            writer
                .write_all(&wire)
                .await
                .context("Writing printer commands")?;
            count += wire.len();
        }
    }
    if !batch.is_empty() {
        // Android submits each command-list item (including a complete image) in
        // one write. Frame-by-frame writes produced incomplete output on the R22.
        tracing::debug!(direction="tx",bytes=%hex::encode(&batch),"RFCOMM batch submitted");
        writer
            .write_all(&batch)
            .await
            .context("Writing printer command batch")?;
        count += batch.len();
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

#[cfg(test)]
mod write_tests {
    use super::*;
    use std::{
        pin::Pin,
        task::{Context as TaskContext, Poll},
    };

    #[derive(Default)]
    struct Writes(Vec<Vec<u8>>);
    impl AsyncWrite for Writes {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut TaskContext<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.0.push(bytes.to_vec());
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: Pin<&mut Self>,
            _: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn unpaced_job_is_submitted_as_one_buffer_like_android() {
        let raster = crate::raster::Raster::new(384, 64, vec![0; 48 * 64]).unwrap();
        let frames = crate::image_data::encode_image(&raster, 2050).unwrap();
        let expected: Vec<u8> = frames.iter().flat_map(|f| f.encode().unwrap()).collect();
        let mut writer = Writes::default();
        let count = write_frames(&mut writer, &frames, Duration::ZERO, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(count, expected.len());
        assert_eq!(writer.0, vec![expected]);
    }
}
