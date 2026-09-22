use crate::bluetooth::Connection;
use crate::configuration::{Model, Storage};
use anyhow::{Context, Result, ensure};
use bytes::BytesMut;
use rongta::bluetooth::{DeviceFilter, ScanDevice};
use rongta::media::Media;
use rongta::{client, handshake::Handshake, protocol::FrameCodec};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::codec::Decoder;
use usage::{Args, Cli, Subcommands};
mod bluetooth;
mod configuration;
mod settings_ui;

/// Experimental Rongta R22 printer protocol tools
///
/// Inspect an R22 over Bluetooth Classic SPP or work with protocol bytes offline.
#[derive(Cli)]
#[usage(bin = "rongta", version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// TOML configuration override file (or RONGTA_CONFIG)
    #[usage(long, global)]
    config: Option<PathBuf>,
    #[usage(subcommand)]
    command: Commands,
}

#[derive(Subcommands)]
enum Commands {
    Settings(SettingsArgs),
    Status(Status),
    Preview(Preview),
    Discovery(Discovery),
    Scan(Scan),
    Decode(Decode),
    Info(Info),
    Image(Image),
    Print(Print),
}

/// Configure saved printer/media; without flags, open a terminal menu
#[derive(Args)]
struct SettingsArgs {
    /// Save a printer MAC address; changing printers clears saved media
    #[usage(long)]
    address: Option<String>,
    /// Save printer model (r22)
    #[usage(long)]
    model: Option<String>,
    /// Save loaded media: gap:50x30, circle:40, or continuous:50 (mm)
    #[usage(long)]
    media: Option<String>,
    /// Clear the saved media selection
    #[usage(long)]
    clear_media: bool,
}

/// Show effective printer/media selections without contacting Bluetooth
#[derive(Args)]
struct Status {
    /// Output effective settings as JSON
    #[usage(long)]
    json: bool,
}

#[derive(Args)]
struct Preparation {
    /// Override loaded media for this job: gap:50x30, circle:40, continuous:50
    #[usage(long)]
    media: Option<String>,
    /// Image placement: label crops physical margins; fit preserves the whole image
    #[usage(long, default = "label")]
    layout: String,
    /// Length in mm for continuous stock; otherwise derived from image aspect ratio
    #[usage(long)]
    length_mm: Option<f64>,
    /// Ignore selected media and treat image pixels as printer dots
    #[usage(long)]
    raw: bool,
}

#[derive(Args)]
struct ImageConversion {
    /// 1-bit conversion: none, floyd-steinberg, atkinson, jarvis, stucki, sierra, sierra-lite, bayer
    #[usage(long, default = "none")]
    dither: String,
    /// SDK threshold (inclusive, default 208); only valid with --dither none
    #[usage(long)]
    threshold: Option<u8>,
}

impl ImageConversion {
    fn resolve(&self) -> Result<rongta::monochrome::Conversion> {
        use rongta::monochrome::Conversion;
        if self.dither == "none" {
            Ok(Conversion::Threshold(self.threshold.unwrap_or(208)))
        } else {
            let method = self.dither.parse()?;
            ensure!(
                self.threshold.is_none(),
                "--threshold only applies to --dither none"
            );
            Ok(Conversion::Dither(method))
        }
    }
}

/// Save an exact monochrome raster preview without contacting the printer
#[derive(Args)]
struct Preview {
    input: PathBuf,
    /// Destination PNG file
    #[usage(long)]
    output: PathBuf,
    /// Head width in dots; defaults to the measured R22 width
    #[usage(long, default = "384")]
    head_width: u32,
    #[usage(flatten)]
    conversion: ImageConversion,
    #[usage(flatten)]
    preparation: Preparation,
}

/// Discover likely printers and show a readable device table
///
/// Searches Classic and LE without pairing. Includes BlueZ's cached devices,
/// which may be out of range. Use --all to bypass the stock app's name filter.
#[derive(Args)]
struct Scan {
    /// Match stock RONGTA printer names (default); --all disables name filtering
    #[usage(long, negate = "--all", default = "true")]
    printers: bool,
    /// Output a JSON array instead of a readable table
    #[usage(long)]
    json: bool,
    /// Host BlueZ adapter; defaults to the system's default adapter
    #[usage(long)]
    adapter: Option<String>,
    /// Discovery duration in seconds (1–3600)
    #[usage(long, default = "10")]
    timeout: u64,
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
    address: Option<String>,
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
    #[usage(flatten)]
    conversion: ImageConversion,
}

/// Handshake and send a PNG/PNM label over host BlueZ SPP
///
/// Uses selected media for image geometry. Without media, pixels are printer dots.
/// Success reports submission and a subsequent idle status, not verified paper output.
#[derive(Args)]
struct Print {
    #[usage(flatten)]
    preparation: Preparation,
    /// Source image (legacy: MAC followed by image is also accepted)
    input: PathBuf,
    /// Image path when using the legacy MAC IMAGE syntax
    legacy_input: Option<PathBuf>,
    /// Override the selected printer for this job
    #[usage(long)]
    address: Option<String>,
    /// Host BlueZ adapter; defaults to the system's default adapter
    #[usage(long)]
    adapter: Option<String>,
    /// Legacy pairing PIN, if required
    #[usage(long)]
    pin: Option<String>,
    /// Deadline in seconds, separately for connection, handshake, and all copies
    #[usage(long, default = "120")]
    timeout: u64,
    /// Number of copies (1–100)
    #[usage(long, default = "1")]
    copies: u16,
    /// Heat density: 0 light, 8 normal, 15 dark
    #[usage(long, default = "8")]
    density: u8,
    /// Raw SDK speed value; omitted by default, as in the app's image path
    #[usage(long)]
    speed: Option<u8>,
    /// Forward-feed length after each copy; raw SDK units, not yet measured
    #[usage(long, default = "0")]
    feed_length: u16,
    #[usage(flatten)]
    conversion: ImageConversion,
    /// Delay before each setup command or image batch, matching the app by default
    #[usage(long, default = "100")]
    command_delay_ms: u64,
    /// Extra delay between encoded image packets for hardware experiments
    #[usage(long, default = "0")]
    packet_delay_ms: u64,
    /// Log raw transmitted/received bytes to stderr
    #[usage(long, short = 'v')]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut output = io::stdout().lock();
    match cli.command {
        Commands::Preview(args) => {
            let settings = Storage::discover(cli.config)?.effective()?;
            let media = resolve_media(&args.preparation, settings.printer.media)?;
            let source = load_image(&args.input)?;
            let raster = prepare_image(
                &source,
                media.as_ref(),
                &args.preparation,
                args.head_width,
                args.conversion.resolve()?,
            )?;
            rongta::image_processing::preview(&raster)
                .save_with_format(&args.output, image::ImageFormat::Png)
                .with_context(|| format!("Cannot save preview {}", args.output.display()))?;
            writeln!(
                output,
                "Saved {} × {} dot preview to {}",
                raster.width(),
                raster.height(),
                args.output.display()
            )?;
        }
        Commands::Settings(args) => {
            let storage = Storage::discover(cli.config)?;
            // Validate explicit config even though this command only writes saved state.
            storage.effective()?;
            if args.address.is_none()
                && args.model.is_none()
                && args.media.is_none()
                && !args.clear_media
            {
                settings_ui::menu(&storage).await?;
            } else {
                ensure!(
                    !(args.clear_media && args.media.is_some()),
                    "Use either --media or --clear-media"
                );
                let address = args
                    .address
                    .as_deref()
                    .map(str::parse::<bluer::Address>)
                    .transpose()
                    .context("Invalid printer Bluetooth address")?;
                let media = args.media.as_deref().map(str::parse::<Media>).transpose()?;
                if let Some(model) = &args.model {
                    ensure!(
                        model == "r22",
                        "Unsupported printer model {model}; only r22 is supported"
                    );
                }
                storage.update(|saved| {
                    if let Some(address) = address {
                        if saved.printer.address != Some(address) {
                            saved.printer.media = None;
                        }
                        saved.printer.address = Some(address);
                    }
                    if args.model.is_some() {
                        saved.printer.model = Model::R22;
                    }
                    if args.clear_media {
                        saved.printer.media = None;
                    }
                    if let Some(media) = media {
                        saved.printer.media = Some(media);
                    }
                    Ok(())
                })?;
                writeln!(output, "Saved settings to {}", storage.state_path.display())?;
                if storage.effective()? != storage.saved()? {
                    eprintln!(
                        "File/environment overrides remain active; use rongta status to see effective settings."
                    );
                }
            }
        }
        Commands::Status(args) => {
            let storage = Storage::discover(cli.config)?;
            let settings = storage.effective()?;
            if args.json {
                serde_json::to_writer_pretty(&mut output, &settings)?;
                writeln!(output)?;
            } else {
                writeln!(
                    output,
                    "Printer: {}\nModel:   {}\nMedia:   {}\nState:   {}\nConfig:  {}",
                    settings
                        .printer
                        .address
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| "not selected".into()),
                    settings.printer.model,
                    settings
                        .printer
                        .media
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "not selected".into()),
                    storage.state_path.display(),
                    storage.config_path.display()
                )?;
                if settings != storage.saved()? {
                    writeln!(output, "File/environment overrides are active.")?;
                }
            }
        }
        Commands::Scan(args) => {
            ensure!(
                (1..=3600).contains(&args.timeout),
                "--timeout must be between 1 and 3600 seconds"
            );
            init_logging(false);
            let mut discovery = bluetooth::BlueZDiscovery {
                adapter_name: args.adapter.as_deref(),
            };
            let filter = if args.printers {
                DeviceFilter::Printers
            } else {
                DeviceFilter::All
            };
            let devices =
                rongta::bluetooth::scan(&mut discovery, Duration::from_secs(args.timeout), filter)
                    .await?;
            write_scan(&mut output, &devices, args.printers, args.json)?;
        }
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
            let conversion = args.conversion.resolve()?;
            let source = load_image(&args.input)?;
            let raster = rongta::monochrome::convert(&source, source.dimensions(), conversion)?;
            let job = rongta::image_data::encode_image(&raster, args.buffer_length)?;
            for frame in job {
                output.write_all(&frame.encode()?)?;
            }
        }
        Commands::Info(args) => {
            let settings = Storage::discover(cli.config)?.effective()?;
            let address = resolve_address(args.address.as_deref(), settings.printer.address)?;
            let address = validate_connection(&address, args.timeout, args.pin.as_deref())?;
            init_logging(args.verbose);
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
        Commands::Print(args) => {
            let settings = Storage::discover(cli.config)?.effective()?;
            let (input, explicit_address) = if let Some(input) = &args.legacy_input {
                ensure!(
                    args.address.is_none(),
                    "Do not combine legacy MAC IMAGE syntax with --address"
                );
                (
                    input,
                    Some(
                        args.input
                            .to_str()
                            .context("Invalid printer Bluetooth address")?,
                    ),
                )
            } else {
                (&args.input, args.address.as_deref())
            };
            let address = resolve_address(explicit_address, settings.printer.address)?;
            let address = validate_connection(&address, args.timeout, args.pin.as_deref())?;
            // A one-off different printer has no known loaded media.
            let saved_media =
                if explicit_address.is_some() && settings.printer.address != Some(address) {
                    None
                } else {
                    settings.printer.media
                };
            let media = resolve_media(&args.preparation, saved_media)?;
            ensure!(
                (1..=100).contains(&args.copies),
                "--copies must be between 1 and 100"
            );
            ensure!(args.density <= 15, "--density must be between 0 and 15");
            let conversion = args.conversion.resolve()?;
            let source = load_image(input)?;
            init_logging(args.verbose);
            let timeout = Duration::from_secs(args.timeout);
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).context("Generating handshake nonce")?;
            let mut connection =
                Connection::connect(address, args.adapter.as_deref(), args.pin, timeout).await?;
            let mut session = client::Session::new(&mut connection.stream);
            let info = session
                .handshake(
                    nonce,
                    client::Options {
                        timeout,
                        ..Default::default()
                    },
                )
                .await?;
            tracing::info!(
                width = info.model.width_dots,
                buffer = info.protocol.buffer_length,
                "Printer handshake complete"
            );
            let raster = prepare_image(
                &source,
                media.as_ref(),
                &args.preparation,
                info.model.width_dots,
                conversion,
            )?;
            let options = rongta::printing::PrintOptions {
                copies: args.copies,
                density: args.density,
                speed: args.speed,
                feed_length: args.feed_length,
                timeout,
                command_delay: Duration::from_millis(args.command_delay_ms),
                packet_delay: Duration::from_millis(args.packet_delay_ms),
                ..Default::default()
            };
            let report = session.print(&info, &raster, options).await
                .context("Print job failed; any bytes already sent cannot be recalled; check the printer before retrying")?;
            serde_json::to_writer_pretty(&mut output, &report)?;
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

fn write_scan(
    output: &mut impl Write,
    devices: &[ScanDevice<bluer::Address>],
    printers: bool,
    json: bool,
) -> Result<()> {
    if json {
        serde_json::to_writer_pretty(&mut *output, &devices)?;
        writeln!(output)?;
        return Ok(());
    }
    if devices.is_empty() {
        writeln!(
            output,
            "{}",
            if printers {
                "No matching printer names found. Try --all to include devices with other names."
            } else {
                "No Bluetooth devices found."
            }
        )?;
        return Ok(());
    }
    writeln!(
        output,
        "{:<17}  {:>5}  {:<7}  {:<6}  {:<5}  NAME",
        "ADDRESS", "RSSI", "SPP", "PAIRED", "KNOWN"
    )?;
    for device in devices {
        writeln!(
            output,
            "{:<17}  {:>5}  {:<7}  {:<6}  {:<5}  {}",
            device.address.to_string(),
            device
                .rssi
                .map(|rssi| rssi.to_string())
                .unwrap_or_else(|| "-".into()),
            match device.spp {
                Some(true) => "yes",
                Some(false) => "unlisted",
                None => "?",
            },
            if device.paired { "yes" } else { "no" },
            if device.known_before_scan {
                "yes"
            } else {
                "no"
            },
            device.name.as_deref().unwrap_or("(unknown)").escape_debug(),
        )?;
    }
    writeln!(
        output,
        "\nRSSI in dBm; KNOWN means cached before scanning, not confirmed nearby. SPP may be unresolved."
    )?;
    Ok(())
}

fn validate_connection(address: &str, timeout: u64, pin: Option<&str>) -> Result<bluer::Address> {
    let address = address
        .parse()
        .context("Invalid printer Bluetooth address")?;
    ensure!(
        (1..=3600).contains(&timeout),
        "--timeout must be between 1 and 3600 seconds"
    );
    if let Some(pin) = pin {
        ensure!(
            (1..=16).contains(&pin.len()) && pin.is_ascii() && !pin.contains('\0'),
            "--pin must contain 1–16 ASCII characters without NUL"
        );
    }
    Ok(address)
}

fn init_logging(verbose: bool) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                if verbose {
                    "rongta=debug".into()
                } else {
                    "rongta=info".into()
                }
            }),
        )
        .with_writer(io::stderr)
        .with_ansi(false)
        .init();
}

fn resolve_address(explicit: Option<&str>, saved: Option<bluer::Address>) -> Result<String> {
    explicit
        .map(str::to_owned)
        .or_else(|| saved.map(|a| a.to_string()))
        .context("No printer selected; run rongta settings or provide a printer address")
}

fn resolve_media(args: &Preparation, saved: Option<Media>) -> Result<Option<Media>> {
    let _: rongta::image_processing::Layout = args.layout.parse()?;
    ensure!(
        !(args.raw && (args.media.is_some() || args.length_mm.is_some() || args.layout != "label")),
        "--raw cannot be combined with media, length, or layout options"
    );
    let media = if args.raw {
        None
    } else {
        args.media.as_deref().map(str::parse).transpose()?.or(saved)
    };
    ensure!(
        media.is_some() || (args.length_mm.is_none() && args.layout == "label"),
        "Length and layout options require selected media"
    );
    if let Some(length) = args.length_mm {
        ensure!(
            matches!(media, Some(Media::Continuous { .. })),
            "--length-mm only applies to continuous media"
        );
        ensure!(
            length.is_finite() && length > 2.0 && length <= 1000.0,
            "Label length must be greater than 2 mm and at most 1000 mm"
        );
    }
    Ok(media)
}

fn prepare_image(
    source: &image::RgbaImage,
    media: Option<&Media>,
    args: &Preparation,
    width: u32,
    conversion: rongta::monochrome::Conversion,
) -> Result<rongta::raster::Raster> {
    match media {
        Some(media) => rongta::image_processing::prepare(
            source,
            media,
            width,
            args.layout.parse()?,
            args.length_mm,
            conversion,
        ),
        None => {
            rongta::monochrome::convert(source, source.dimensions(), conversion)?.centered(width)
        }
    }
}

fn load_image(path: &std::path::Path) -> Result<image::RgbaImage> {
    let mut reader = image::ImageReader::open(path)
        .with_context(|| format!("Cannot open {}", path.display()))?
        .with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let source = reader
        .decode()
        .context("Decoding PNG/PNM image")?
        .into_rgba8();
    Ok(source)
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

#[cfg(test)]
mod scan_output_tests {
    use super::*;
    use rongta::bluetooth::ScanDevice;

    fn device(name: Option<&str>, class: Option<u32>) -> ScanDevice<bluer::Address> {
        ScanDevice {
            address: "AA:BB:CC:DD:EE:FF".parse().unwrap(),
            name: name.map(str::to_owned),
            class,
            rssi: None,
            paired: false,
            connected: false,
            spp: Some(true),
            known_before_scan: true,
        }
    }

    #[test]
    fn scan_defaults_and_overrides() {
        for (args, printers, json) in [
            (vec!["scan"], true, false),
            (vec!["scan", "--json"], true, true),
            (vec!["scan", "--all", "--json"], false, true),
            (vec!["scan", "--all", "--printers"], true, false),
        ] {
            let args: Vec<_> = args.iter().map(std::ffi::OsStr::new).collect();
            let Commands::Scan(scan) = Cli::parse_from(&args).unwrap().command else {
                panic!()
            };
            assert_eq!((scan.printers, scan.json), (printers, json));
        }
    }

    #[test]
    fn json_and_human_output_handle_unknowns_and_escape_names() {
        let devices = [
            device(Some("R22_\x1b[31m"), None),
            device(Some("Headphones"), None),
        ];
        let mut out = vec![];
        write_scan(&mut out, &devices[..1], true, true).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0]["address"], "AA:BB:CC:DD:EE:FF");
        assert!(json[0]["rssi"].is_null());
        out.clear();
        write_scan(&mut out, &devices, false, true).unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<serde_json::Value>>(&out)
                .unwrap()
                .len(),
            2
        );
        out.clear();
        write_scan(&mut out, &devices[..1], true, false).unwrap();
        let table = String::from_utf8(out).unwrap();
        assert!(table.contains("ADDRESS") && table.contains("AA:BB:CC:DD:EE:FF"));
        assert!(!table.contains("Headphones") && !table.contains('\x1b'));
        assert!(table.contains(r"R22_\u{1b}"));
    }

    #[test]
    fn empty_output_is_useful_text_or_valid_json() {
        for json in [false, true] {
            let mut out = vec![];
            write_scan(&mut out, &[], true, json).unwrap();
            if json {
                assert_eq!(out, b"[]\n");
            } else {
                assert!(String::from_utf8(out).unwrap().contains("Try --all"));
            }
        }
    }
}
