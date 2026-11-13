//! Couples a transport that lives on its own OS thread to an async byte stream.
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::mpsc,
};

const BUFFER: usize = 64 * 1024;

/// The transport thread's side of the bridge.
///
/// Dropping `incoming` ends the async stream's input. When the async stream is
/// dropped, `outgoing` reports disconnection, which tells the transport to close.
pub struct Ends {
    pub incoming: mpsc::UnboundedSender<Vec<u8>>,
    /// Holds at most one pending write, so async writers wait for the transport.
    pub outgoing: mpsc::Receiver<Vec<u8>>,
}

/// Must be called within a tokio runtime; the copying tasks run there.
pub fn stream() -> (DuplexStream, Ends) {
    let (stream, far) = tokio::io::duplex(BUFFER);
    let (mut far_read, mut far_write) = tokio::io::split(far);
    let (incoming, mut incoming_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (outgoing_tx, outgoing) = mpsc::channel::<Vec<u8>>(1);
    tokio::spawn(async move {
        while let Some(data) = incoming_rx.recv().await {
            if far_write.write_all(&data).await.is_err() {
                return;
            }
        }
        let _ = far_write.shutdown().await;
    });
    tokio::spawn(async move {
        let mut buffer = vec![0; 4096];
        while let Ok(n @ 1..) = far_read.read(&mut buffer).await {
            if outgoing_tx.send(buffer[..n].to_vec()).await.is_err() {
                return;
            }
        }
    });
    (stream, Ends { incoming, outgoing })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn carries_bytes_both_ways_and_reports_closure() {
        let (mut stream, mut ends) = stream();
        stream.write_all(b"ping").await.unwrap();
        assert_eq!(ends.outgoing.recv().await.unwrap(), b"ping");
        ends.incoming.send(b"pong".to_vec()).unwrap();
        let mut reply = [0; 4];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"pong");

        drop(ends.incoming);
        assert_eq!(stream.read(&mut reply).await.unwrap(), 0);
        drop(stream);
        assert!(ends.outgoing.recv().await.is_none());
    }
}
