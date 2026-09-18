use anyhow::{Context, Result, ensure};
use bytes::BytesMut;
use rongta::{bluetooth::Connection, client, handshake::Handshake, protocol::FrameCodec};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::codec::Decoder;
use usage::{Args, Cli, Subcommands};

/// Experimental Rongta R22 printer protocol tools
///
/// Inspect an R22 over Bluetooth Classic SPP or work with protocol bytes offline.
#[derive(Cli)]
#[usage(bin = "rongta", version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[usage(subcommand)]
    command: Commands,
}

#[derive(Subcommands)]
enum Commands {
    Discovery(Discovery),
    Decode(Decode),
    Info(Info),
    Image(Image),
}

/// Write the two R22 discovery packets as binary to stdout
#[derive(Args)]
struct Discovery {}

/// Decode a raw Rongta byte stream into JSON lines
///
/// Reads a file or stdin. Input must be application bytes extracted from the
/// RFCOMM stream, not an entire btsnoop or pcap file.
#[derive(Args)]
struct Decode {
    /// Raw packet file; omit to read stdin
    input: Option<PathBuf>,
    /// Maximum inner-message length including its five-byte header
    #[usage(long, default = "2050")]
    max_body_len: u16,
}

/// Connect to an R22, complete its handshake, and print information as JSON
///
/// Uses host BlueZ: stop bt-bridge and restore host Bluetooth first.
/// Pairing uses the current BlueZ agent unless --pin is supplied.
#[derive(Args)]
struct Info {
    /// Printer Bluetooth MAC address (AA:BB:CC:DD:EE:FF)
    address: String,
    /// Host BlueZ adapter; defaults to the system's default adapter
    #[usage(long)]
    adapter: Option<String>,
    /// Deadline in seconds for connection, then separately for the handshake
    #[usage(long, default = "30")]
    timeout: u64,
    /// Legacy pairing PIN, if required by the printer
    #[usage(long)]
    pin: Option<String>,
    /// Log raw transmitted/received bytes to stderr
    #[usage(long, short = 'v')]
    verbose: bool,
}

/// Encode a PNG/PNM as an R22 compressed image job, writing binary to stdout
///
/// Offline only. Input pixels are printer dots, with no resizing; alpha is
/// composited onto white and rows are padded to eight dots. This emits image
/// start/tree/data/end packets, without copies, heat, feed, or transport pacing.
#[derive(Args)]
struct Image {
    /// Source PNG or PNM image
    input: PathBuf,
    /// Printer pBuffLength from info output; not the decoder's body-length limit
    #[usage(long)]
    buffer_length: u16,
    /// SDK black/white threshold (inclusive)
    #[usage(long, default = "208")]
    threshold: u8,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut output = io::stdout().lock();
    match cli.command {
        Commands::Discovery(_) => {
            for frame in Handshake::new([0; 8]).discovery() {
                output.write_all(&frame.encode()?)?;
            }
        }
        Commands::Image(args) => {
            ensure!(
                args.buffer_length >= 64,
                "--buffer-length must be at least 64 bytes"
            );
            let mut reader = image::ImageReader::open(&args.input)
                .with_context(|| format!("Cannot open {}", args.input.display()))?
                .with_guessed_format()?;
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(64 * 1024 * 1024);
            reader.limits(limits);
            let source = reader
                .decode()
                .context("Decoding PNG/PNM image")?
                .into_rgba8();
            let mut canvas = image::RgbaImage::from_pixel(
                source.width(),
                source.height(),
                image::Rgba([255; 4]),
            );
            image::imageops::overlay(&mut canvas, &source, 0, 0);
            let raster = rongta::raster::Raster::from_rgba(&canvas, args.threshold)?;
            let job = rongta::image_data::encode_image(&raster, args.buffer_length)?;
            for frame in job {
                output.write_all(&frame.encode()?)?;
            }
        }
        Commands::Info(args) => {
            let address = args
                .address
                .parse()
                .context("Invalid printer Bluetooth address")?;
            ensure!(
                (1..=3600).contains(&args.timeout),
                "--timeout must be between 1 and 3600 seconds"
            );
            if let Some(pin) = &args.pin {
                ensure!(
                    (1..=16).contains(&pin.len()) && pin.is_ascii() && !pin.contains('\0'),
                    "--pin must contain 1–16 ASCII characters without NUL"
                );
            }
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                        if args.verbose {
                            "rongta=debug".into()
                        } else {
                            "rongta=info".into()
                        }
                    }),
                )
                .with_writer(io::stderr)
                .with_ansi(false)
                .init();
            let timeout = Duration::from_secs(args.timeout);
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).context("Generating handshake nonce")?;
            let mut connection =
                Connection::connect(address, args.adapter.as_deref(), args.pin, timeout).await?;
            let info = client::query_info(
                &mut connection.stream,
                nonce,
                client::Options {
                    timeout,
                    ..Default::default()
                },
            )
            .await?;
            serde_json::to_writer_pretty(&mut output, &info)?;
            writeln!(output)?;
        }
        Commands::Decode(args) => {
            ensure!(args.max_body_len >= 5, "--max-body-len must be at least 5");
            let input: Box<dyn Read> = match args.input {
                Some(path) => Box::new(
                    File::open(&path).with_context(|| format!("Cannot open {}", path.display()))?,
                ),
                None => Box::new(io::stdin().lock()),
            };
            decode_stream(input, &mut output, args.max_body_len)?;
        }
    }
    output.flush()?;
    Ok(())
}

fn decode_stream(mut input: impl Read, output: &mut impl Write, max_body_len: u16) -> Result<()> {
    let mut codec = FrameCodec::new(max_body_len);
    let mut buffer = BytesMut::new();
    let mut chunk = [0; 4096];
    loop {
        let count = match input.read(&mut chunk) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result.context("Reading packet stream")?,
        };
        buffer.extend_from_slice(&chunk[..count]);
        loop {
            let frame = if count == 0 {
                codec.decode_eof(&mut buffer)
            } else {
                codec.decode(&mut buffer)
            }
            .context("Decoding Rongta frame")?;
            let Some(frame) = frame else { break };
            serde_json::to_writer(
                &mut *output,
                &serde_json::json!({
                    "mode": frame.mode,
                    "message_type": frame.message_type,
                    "sub_message_type": frame.sub_message_type,
                    "superclass": frame.superclass,
                    "subclass": frame.subclass,
                    "payload_hex": hex::encode(&frame.payload),
                }),
            )?;
            writeln!(output)?;
        }
        if count == 0 {
            return Ok(());
        }
    }
}
