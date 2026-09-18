//! R22 print control, status parsing, and bounded full-duplex job submission.
use crate::{
    client::Session,
    handshake::{PrinterInfo, number, tlvs},
    image_data::encode_image,
    protocol::Frame,
    raster::Raster,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

pub fn command(superclass: u8, subclass: u8, payload: Vec<u8>) -> Frame {
    Frame {
        mode: 0,
        message_type: 1,
        sub_message_type: 1,
        superclass,
        subclass,
        payload,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PrinterStatus {
    pub raw: u32,
    pub printing: bool,
    pub faults: Vec<String>,
}
impl PrinterStatus {
    pub fn from_frame(frame: &Frame) -> Result<Option<Self>> {
        if frame.superclass != 1
            || frame.subclass != 0x13
            || frame.message_type != 2
            || frame.sub_message_type != 1
        {
            return Ok(None);
        }
        let raw = number(&tlvs(&frame.payload)?, 1).context("Invalid printer status")?;
        let names = [
            "cover open",
            "out of paper",
            "paper jam",
            "overheating",
            "missing print head",
            "high voltage",
            "low voltage",
            "printing",
            "cutter error",
            "font memory error",
            "PSRAM error",
            "paper learning required",
            "about to shut down",
            "shutting down",
            "black-mark paper error",
            "NFC error",
        ];
        let mut faults: Vec<_> = names
            .iter()
            .enumerate()
            .filter(|(bit, _)| *bit != 7 && raw & (1 << bit) != 0)
            .map(|(_, name)| (*name).into())
            .collect();
        if raw >> 16 != 0 {
            faults.push(format!("unknown status bits {:#x}", raw & 0xffff0000));
        }
        Ok(Some(Self {
            raw,
            printing: raw & 128 != 0,
            faults,
        }))
    }
}

#[derive(Clone, Debug)]
pub struct PrintOptions {
    pub copies: u16,
    /// App density pair 8,N; normal is N=8, light=0, dark=15.
    pub density: u8,
    pub speed: Option<u8>,
    /// Raw SDK forward-feed length. Physical units still require hardware validation.
    pub feed_length: u16,
    pub timeout: Duration,
    pub command_delay: Duration,
    pub packet_delay: Duration,
    pub poll_interval: Duration,
}
impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            copies: 1,
            density: 8,
            speed: None,
            feed_length: 0,
            timeout: Duration::from_secs(120),
            command_delay: Duration::from_millis(100),
            packet_delay: Duration::ZERO,
            poll_interval: Duration::from_millis(500),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct PrintReport {
    pub outcome: &'static str,
    pub copies_sent: u16,
    pub bytes_sent: usize,
    pub width_dots: u32,
    pub height_dots: u32,
    pub observed_printing: bool,
    pub final_status: PrinterStatus,
}
#[derive(Default)]
struct Monitor {
    query: Option<PrinterStatus>,
    observed_printing: bool,
}
impl Monitor {
    fn accept(&mut self, frame: Frame) -> Result<()> {
        ensure!(
            !(frame.message_type == 2
                && frame.sub_message_type == 2
                && (frame.superclass == 5
                    || (frame.superclass == 1 && [0x13, 0x1e].contains(&frame.subclass)))),
            "Printer rejected command {:02x}/{:02x}",
            frame.superclass,
            frame.subclass
        );
        if let Some(status) = PrinterStatus::from_frame(&frame)? {
            tracing::info!(
                raw = status.raw,
                printing = status.printing,
                "Printer status"
            );
            ensure!(
                status.faults.is_empty(),
                "Printer fault: {} (status {:#x})",
                status.faults.join(", "),
                status.raw
            );
            self.observed_printing |= status.printing;
            if frame.subclass == 0x13 {
                self.query = Some(status);
            }
        }
        Ok(())
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    /// Uses the same connection as handshake. No automatic retry is safe after
    /// transmission starts; success means bytes sent and a subsequent idle reply.
    pub async fn print(
        &mut self,
        info: &PrinterInfo,
        source: &Raster,
        options: PrintOptions,
    ) -> Result<PrintReport> {
        ensure!(
            (1..=100).contains(&options.copies),
            "Copies must be between 1 and 100"
        );
        ensure!(options.density <= 15, "Density must be between 0 and 15");
        ensure!(
            !options.timeout.is_zero() && !options.poll_interval.is_zero(),
            "Print timeout and poll interval must be nonzero"
        );
        let buffer = u16::try_from(info.protocol.buffer_length)
            .context("Printer buffer length exceeds supported wire limit")?;
        let raster = source.centered(info.model.width_dots)?;
        let image = encode_image(&raster, buffer)?; // validate everything before any print command
        tokio::time::timeout(options.timeout,self.print_inner(&raster,&image,&options)).await
            .context("Timed out submitting/waiting for print job; it may be partially sent; no automatic retry")?
    }
    async fn status(&mut self, m: &mut Monitor, delay: Duration) -> Result<PrinterStatus> {
        m.query = None;
        self.send_observing(&[command(1, 0x13, vec![])], delay, Duration::ZERO, |f| {
            m.accept(f)
        })
        .await?;
        loop {
            if let Some(status) = m.query.take() {
                return Ok(status);
            }
            m.accept(self.next().await?)?;
        }
    }
    async fn print_inner(
        &mut self,
        raster: &Raster,
        image: &[Frame],
        o: &PrintOptions,
    ) -> Result<PrintReport> {
        let mut monitor = Monitor::default();
        let preflight = self.status(&mut monitor, o.command_delay).await?;
        ensure!(
            !preflight.printing,
            "Printer is already printing; job was not sent"
        );
        let mut bytes_sent = 15; // preflight status query
        for setup in [
            command(1, 0x1e, vec![1]),
            command(5, 0x29, vec![0]),
            command(5, 4, vec![3, 8, o.density]),
        ] {
            bytes_sent += self
                .send_observing(&[setup], o.command_delay, Duration::ZERO, |f| {
                    monitor.accept(f)
                })
                .await?;
        }
        let mut final_status = preflight;
        for copy in 1..=o.copies {
            tracing::info!(copy, copies = o.copies, "Sending image job");
            if let Some(speed) = o.speed {
                bytes_sent += self
                    .send_observing(
                        &[command(5, 7, vec![speed])],
                        o.command_delay,
                        Duration::ZERO,
                        |f| monitor.accept(f),
                    )
                    .await?;
            }
            bytes_sent += self
                .send_observing(image, o.command_delay, o.packet_delay, |f| {
                    monitor.accept(f)
                })
                .await?;
            if o.feed_length > 0 {
                let mut payload = vec![1];
                payload.extend(o.feed_length.to_le_bytes());
                bytes_sent += self
                    .send_observing(
                        &[command(5, 8, payload)],
                        o.command_delay,
                        Duration::ZERO,
                        |f| monitor.accept(f),
                    )
                    .await?;
            }
            loop {
                final_status = self.status(&mut monitor, o.poll_interval).await?;
                bytes_sent += 15;
                if !final_status.printing {
                    break;
                }
            }
        }
        Ok(PrintReport {
            outcome: "sent_printer_idle",
            copies_sent: o.copies,
            bytes_sent,
            width_dots: raster.width(),
            height_dots: raster.height(),
            observed_printing: monitor.observed_printing,
            final_status,
        })
    }
}
