use anyhow::{Context, Result, ensure};
use bytes::BytesMut;
use rongta::protocol::{Frame, FrameCodec};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use tokio_util::codec::Decoder;
use usage::{Args, Cli, Subcommands};

/// Experimental Rongta R22 printer protocol tools
///
/// Generate and inspect protocol bytes offline. Live printer connections are
/// not implemented yet.
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut output = io::stdout().lock();
    match cli.command {
        Commands::Discovery(_) => {
            for subclass in [1, 6] {
                let frame = Frame {
                    mode: 0,
                    message_type: 1,
                    sub_message_type: 1,
                    superclass: 1,
                    subclass,
                    payload: Vec::new(),
                };
                output.write_all(&frame.encode()?)?;
            }
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
