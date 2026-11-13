# Label Templates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fill-in label templates (typed fields → text on the left, QR code on
the right) that render pixel-exact for the R22, plus a shared host crate so a
GUI can print the same way the CLI does.

**Architecture:** Three changes to the `rongta-rs/` workspace. The `rongta`
library gains `printable_size` and `place`, split out of `prepare`, so callers
can draw at the exact dot size and skip resampling. A new `rongta-host` crate
takes the Bluetooth backends, saved settings and a connect → handshake → print
`job::print` function out of the CLI. A new `rongta-template` crate parses
`.rongta.toml` templates and renders them with a bundled font and an
integer-module QR code. The CLI gains `rongta template`.

**Tech Stack:** Rust 2024, `ab_glyph` 0.2 (glyph rasterization), `qrcode` 0.14
(QR matrix), `regex` 1, `percent-encoding` 2, `toml` 0.9, `rqrr` 0.11 (test-only
QR decoder), Atkinson Hyperlegible Next 2.001 (SIL OFL 1.1) for label text.

This is Plan A of two. Plan B (`2026-11-12-gui-app.md`) builds the eframe app
on top of the `rongta-host` and `rongta-template` APIs defined here.

## Global Constraints

- Edition 2024 in every new crate; the workspace manifest keeps
  `resolver = "3"`.
- Win32 FFI uses the `windows` crate, never `windows-sys`.
- Comments explain *why* for a reader seeing the file cold: no references to
  other modules, this plan, or how the code got here. Match the density of the
  surrounding file.
- `cargo fmt --all --check`, `cargo clippy --locked --workspace --all-targets
  -- -D warnings` and `cargo test --locked --workspace` pass after every task
  (run from `rongta-rs/`, inside the dev shell).
- Hard-wrap edited Markdown prose at 80 columns and refill whole paragraphs.
- Commit messages follow the repo's `type: summary` style (`feat:`,
  `refactor:`, `build:`, `docs:`). No attribution trailers.
- Work on a new branch `typedrat/label-templates` stacked on
  `typedrat/macos-windows-bluetooth`.
- Templates render with **threshold conversion only**; dithering would break
  QR modules and text edges.
- Template output is drawn at exactly the printable size, one pixel per dot,
  so nothing is resampled between rendering and printing.

## Template format (version 1)

```toml
version = 1
name = "Access code"
media = "gap:50x25"          # same syntax as `rongta settings --media`; gap only

[[field]]
id = "code"                  # [A-Za-z0-9_-]+, unique
label = "Access code"        # shown next to the input
hint = "6 to 12 letters or digits"   # optional; shown when the pattern fails
pattern = "[A-Z0-9]{6,12}"   # optional; must match the whole value
case = "upper"               # optional: "upper" | "lower" (default: unchanged)

[qr]
content = "https://example.com/redeem?code={code}"  # values percent-encoded
error_correction = "M"       # optional: L | M | Q | H (default M)

[[text]]
content = "Scan to redeem"   # values inserted as typed
size_mm = 3.0                # em size, 1–20 mm; shrinks to fit, never below 2 mm
bold = false                 # optional
```

`{name}` inserts a field; `{{` and `}}` are literal braces. Layout is fixed:
the QR code is as large as the printable height allows (at least 3 dots per
module, including the 4-module quiet zone), flush right; text lines are
stacked, left-aligned, and centred vertically in the remaining width.

For `gap:50x25` the printable area is 384 × 184 dots. A ~40-character URL at
ECC M is QR version 3 (29 modules + 8 quiet = 37), so modules are 4 dots and
the code is 148 dots (18.5 mm) square, leaving a 236-dot text column.

## File Structure

```
rongta-rs/
  Cargo.toml                         # modify: workspace members
  src/image_processing.rs            # modify: printable_size, place
  tests/image_processing.rs          # modify: place test
  host/
    Cargo.toml                       # create
    src/lib.rs                       # create
    src/bluetooth/{mod,address,bridge,bluez,iobluetooth,winsock}.rs  # moved from cli/src/bluetooth/
    src/configuration.rs             # moved from cli/src/configuration.rs
    src/job.rs                       # create: connect → handshake → print
  template/
    Cargo.toml                       # create
    fonts/AtkinsonHyperlegibleNext-Regular.ttf  # create (binary)
    fonts/AtkinsonHyperlegibleNext-Bold.ttf     # create (binary)
    fonts/OFL.txt                    # create
    examples/access-code.rongta.toml # create
    src/lib.rs                       # create: Template, parse, normalize, check
    src/interpolate.rs               # create: {field} placeholders
    src/qr.rs                        # create: QR drawing
    src/text.rs                      # create: text lines
    src/render.rs                    # create: render, raster
    tests/template.rs                # create
  cli/
    Cargo.toml                       # modify: depend on host and template
    src/main.rs                      # modify: use host; Template command
    tests/template.rs                # create
flake.nix                            # modify: include .ttf/.toml in sources
README.md                            # modify: template docs
```

---

### Task 1: Split placement out of `prepare`

**Files:**
- Modify: `rongta-rs/src/image_processing.rs:28-131`
- Test: `rongta-rs/tests/image_processing.rs`

**Interfaces:**
- Produces:
  - `rongta::image_processing::printable_size(width_mm: f64, height_mm: f64) -> (u32, u32)`
  - `rongta::image_processing::place(content: &Raster, size: (u32, u32), media: &Media, head_width: u32, canvas_height: u32) -> anyhow::Result<Raster>`

- [ ] **Step 1: Write the failing test**

Append to `rongta-rs/tests/image_processing.rs`:

```rust
#[test]
fn placed_content_is_centred_on_the_label_canvas() {
    use rongta::{
        image_processing::{place, printable_size},
        raster::Raster,
    };
    assert_eq!(printable_size(50.0, 25.0), (384, 184));
    assert_eq!(printable_size(40.0, 30.0), (320, 224));
    let content = Raster::new(8, 2, vec![0xff, 0xff]).unwrap();
    let placed = place(&content, (8, 2), &"gap:50x25".parse().unwrap(), 384, 200).unwrap();
    assert_eq!((placed.width(), placed.height()), (384, 200));
    let black: Vec<(u32, u32)> = (0..200)
        .flat_map(|y| (0..384).map(move |x| (x, y)))
        .filter(|&(x, y)| placed.bytes()[(y * 48 + x / 8) as usize] & (0x80 >> (x % 8)) != 0)
        .collect();
    assert_eq!(black.len(), 16);
    assert_eq!(black[0], (188, 99));
    assert!(place(&content, (16, 2), &"gap:50x25".parse().unwrap(), 384, 200).is_err());
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p rongta --test image_processing placed_content`
Expected: compile error, `place` and `printable_size` not found.

- [ ] **Step 3: Implement**

In `rongta-rs/src/image_processing.rs`, add above `prepare`:

```rust
/// Dots inside the area the R22 can mark on a label of this size: the head is
/// 48 mm wide, and 1 mm at the top and bottom stays blank.
pub fn printable_size(width_mm: f64, height_mm: f64) -> (u32, u32) {
    (
        (width_mm.min(48.0) * 8.0) as u32,
        ((height_mm - 2.0) * 8.0) as u32,
    )
}

/// Centre the top-left `size` dots of `content` on a head-width canvas
/// `canvas_height` dots tall. Dots outside a circular label stay white.
pub fn place(
    content: &Raster,
    size: (u32, u32),
    media: &Media,
    head_width: u32,
    canvas_height: u32,
) -> Result<Raster> {
    let (content_width, content_height) = size;
    ensure!(
        content_width <= content.width()
            && content_height <= content.height()
            && content_width <= head_width
            && content_height <= canvas_height,
        "Content does not fit the label canvas"
    );
    let offset_x = (head_width - content_width) / 2;
    let offset_y = (canvas_height - content_height) / 2;
    let stride = head_width as usize / 8;
    let mut bytes = vec![0; stride * canvas_height as usize];
    for y in 0..content_height {
        for x in 0..content_width {
            let dst_x = x + offset_x;
            let dst_y = y + offset_y;
            if let Media::Circle { diameter_mm } = media {
                let radius = diameter_mm * 4.0;
                let dx = f64::from(dst_x) + 0.5 - f64::from(head_width) / 2.0;
                let dy = f64::from(dst_y) + 0.5 - f64::from(canvas_height) / 2.0;
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
            }
            if content.bytes()[y as usize * (content.width() as usize / 8) + x as usize / 8]
                & (0x80 >> (x % 8))
                != 0
            {
                bytes[dst_y as usize * stride + dst_x as usize / 8] |= 0x80 >> (dst_x % 8);
            }
        }
    }
    Raster::new(head_width, canvas_height, bytes)
}
```

In `prepare`, replace

```rust
    let width = (content_width_mm * 8.0) as u32;
    let height = (content_height_mm * 8.0) as u32;
```

with

```rust
    let (width, height) = printable_size(width_mm, height_mm);
```

(keep `content_width_mm` and `content_height_mm`; the Label crop still uses
them), and replace everything from `let offset_x = ...` to the end of the
function with:

```rust
    place(
        &raster,
        (content_width, content_height),
        media,
        head_width,
        canvas_height,
    )
}
```

- [ ] **Step 4: Run the library tests**

Run: `cargo test -p rongta`
Expected: all pass. The existing `prepare` tests and `tests/fixtures/dither.json`
comparisons pin the unchanged output.

- [ ] **Step 5: Commit**

```bash
git add src/image_processing.rs tests/image_processing.rs
git commit -m "refactor: expose printable size and label placement"
```

---

### Task 2: Move the Bluetooth backends and settings into `rongta-host`

**Files:**
- Create: `rongta-rs/host/Cargo.toml`, `rongta-rs/host/src/lib.rs`
- Move: `rongta-rs/cli/src/bluetooth/` → `rongta-rs/host/src/bluetooth/`
- Move: `rongta-rs/cli/src/configuration.rs` → `rongta-rs/host/src/configuration.rs`
- Modify: `rongta-rs/Cargo.toml`, `rongta-rs/cli/Cargo.toml`,
  `rongta-rs/cli/src/main.rs:1-16`, `rongta-rs/host/src/bluetooth/mod.rs:26`

**Interfaces:**
- Produces (paths only change; signatures unchanged):
  - `rongta_host::bluetooth::{Address, Connection, SystemDiscovery}`
  - `Connection::connect(address: Address, adapter: Option<&str>, pin: Option<String>, timeout: Duration) -> Result<Connection>`;
    `connection.stream` implements `AsyncRead + AsyncWrite + Unpin`
  - `SystemDiscovery { adapter_name: Option<&str> }` implements
    `rongta::bluetooth::Discovery<DeviceId = Address>`
  - `rongta_host::configuration::{Storage, Settings, PrinterSettings, Model}`;
    `Storage::discover(config: Option<PathBuf>)`, `.effective()`, `.saved()`,
    `.update(|&mut Settings| -> Result<()>)`

This is a pure move. The existing CLI integration tests
(`cli/tests/cli.rs`, `cli/tests/settings.rs`) and the moved unit tests are the
safety net, so there's no new test.

- [ ] **Step 1: Move the files**

```bash
cd rongta-rs
mkdir -p host/src
git mv cli/src/bluetooth host/src/bluetooth
git mv cli/src/configuration.rs host/src/configuration.rs
```

- [ ] **Step 2: Create `rongta-rs/host/Cargo.toml`**

```toml
[package]
name = "rongta-host"
version = "0.1.0"
edition = "2024"
description = "Host Bluetooth transports and saved settings for Rongta printers"
publish = false

[dependencies]
rongta = { path = ".." }
anyhow = "1"
confique = { version = "0.4", features = ["toml"] }
directories = "6"
futures-util = "0.3"
getrandom = "0.4"
serde = { version = "1", features = ["derive"] }
tempfile = "3"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "io-util", "time", "sync"] }
toml = "0.9"
tracing = "0.1"
```

Then cut the three `[target.'cfg(...)'.dependencies]` sections (BlueZ,
objc2/IOBluetooth, `windows`) out of `cli/Cargo.toml` and paste them, as they
are, at the end of `host/Cargo.toml`.

- [ ] **Step 3: Create `rongta-rs/host/src/lib.rs`**

```rust
//! Host Bluetooth transports and saved printer settings for Rongta
//! applications on Linux, macOS, and Windows.

pub mod bluetooth;
pub mod configuration;
```

In `host/src/bluetooth/mod.rs`, change the `compile_error!` message to
`"rongta-host supports Bluetooth on Linux, macOS, and Windows only"`.

- [ ] **Step 4: Point the workspace and CLI at the new crate**

In `rongta-rs/Cargo.toml`:

```toml
[workspace]
members = ["cli", "host"]
default-members = [".", "cli", "host"]
resolver = "3"
```

`rongta-rs/cli/Cargo.toml` `[dependencies]` becomes exactly:

```toml
[dependencies]
rongta = { path = ".." }
rongta-host = { path = "../host" }
anyhow = "1"
bytes = "1"
dialoguer = { version = "0.12", default-features = false }
getrandom = "0.4"
hex = "0.4"
image = { version = "0.25", default-features = false, features = ["png", "pnm"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "io-util", "time", "signal", "sync"] }
tokio-util = { version = "0.7", features = ["codec"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
usage = { package = "usage-rs", version = "6", features = ["completions"] }
```

(`[dev-dependencies]` is unchanged; the target-specific sections are gone.)

In `rongta-rs/cli/src/main.rs`, replace

```rust
use crate::bluetooth::Connection;
use crate::configuration::{Model, Storage};
```

with

```rust
use rongta_host::bluetooth::Connection;
use rongta_host::configuration::{Model, Storage};
```

and replace

```rust
mod bluetooth;
mod configuration;
mod settings_ui;
```

with

```rust
use rongta_host::{bluetooth, configuration};
mod settings_ui;
```

`settings_ui.rs` imports `crate::{bluetooth, configuration::...}`, which
still resolve through the `use` at the crate root, so it needs no change.

- [ ] **Step 5: Build and test**

Run: `cargo test --locked --workspace && cargo clippy --locked --workspace --all-targets -- -D warnings`
Expected: all pass. If clippy reports `dead_code` in the host crate for an
item the CLI used to keep alive, make that item `pub`. Don't add
`#[allow]`.

- [ ] **Step 6: Type-check macOS and Windows from Linux**

The dev shell only has the x86_64-linux std. Use a fenix toolchain with the
extra targets and a separate target directory:

```bash
TOOLCHAIN='let f = (builtins.getFlake "github:nix-community/fenix").packages.x86_64-linux; in f.combine [ f.stable.cargo f.stable.rustc f.stable.clippy f.targets.aarch64-apple-darwin.stable.rust-std f.targets.x86_64-pc-windows-msvc.stable.rust-std ]'
for target in aarch64-apple-darwin x86_64-pc-windows-msvc; do
  CARGO_TARGET_DIR="$TMPDIR/rongta-cross" nix shell --impure --expr "$TOOLCHAIN" \
    -c cargo clippy --locked -p rongta-host -p rongta-cli --target "$target" -- -D warnings
done
```

Expected: no errors for either target.

- [ ] **Step 7: Commit**

```bash
git add -A host cli Cargo.toml Cargo.lock
git commit -m "refactor: move Bluetooth backends and settings into rongta-host"
```

---

### Task 3: Shared print job in `rongta-host`

**Files:**
- Create: `rongta-rs/host/src/job.rs`
- Modify: `rongta-rs/host/src/lib.rs`, `rongta-rs/cli/src/main.rs` (the
  `Commands::Print` arm)

**Interfaces:**
- Consumes: `Connection::connect` (Task 2); `rongta::client::Session`,
  `rongta::handshake::PrinterInfo`, `rongta::printing::{PrintOptions, PrintReport}`.
- Produces:
  - `rongta_host::job::Stage { Connecting, Handshaking, Printing }` (`Clone, Copy, Debug, PartialEq, Eq`)
  - `rongta_host::job::Target<'a> { address: Address, adapter: Option<&'a str>, pin: Option<String>, timeout: Duration }`
  - `rongta_host::job::print(target: Target<'_>, raster: impl FnOnce(&PrinterInfo) -> Result<Raster>, options: PrintOptions, progress: impl FnMut(Stage)) -> Result<(PrinterInfo, PrintReport)>`
  - On failure after the job starts, the error context begins with
    `"Print job failed"` (Plan B's error messages match on this).

The session and printing logic are covered by `tests/printing.rs` against
SDK fixtures. This function only sequences calls, and they need a real
transport, so it's verified by the CLI building and, in Step 4, by a hardware
print.

- [ ] **Step 1: Create `rongta-rs/host/src/job.rs`**

```rust
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
```

Add `pub mod job;` to `host/src/lib.rs`.

- [ ] **Step 2: Use it from the CLI's `print`**

In `cli/src/main.rs`'s `Commands::Print` arm, replace everything from
`init_logging(args.verbose);` to the end of the arm with:

```rust
            init_logging(args.verbose);
            let options = rongta::printing::PrintOptions {
                copies: args.copies,
                density: args.density,
                speed: args.speed,
                feed_length: args.feed_length,
                timeout: Duration::from_secs(args.timeout),
                command_delay: Duration::from_millis(args.command_delay_ms),
                packet_delay: Duration::from_millis(args.packet_delay_ms),
                ..Default::default()
            };
            let (_, report) = rongta_host::job::print(
                rongta_host::job::Target {
                    address,
                    adapter: args.adapter.as_deref(),
                    pin: args.pin,
                    timeout: Duration::from_secs(args.timeout),
                },
                |info| {
                    prepare_image(
                        &source,
                        media.as_ref(),
                        &args.preparation,
                        info.model.width_dots,
                        conversion,
                    )
                },
                options,
                |_| {},
            )
            .await?;
            serde_json::to_writer_pretty(&mut output, &report)?;
            writeln!(output)?;
```

Remove the now-unused `use rongta::client` import if clippy flags it. The
`Info` arm still uses `client::query_info`, so keep it if it's still used.

- [ ] **Step 3: Run the workspace checks**

Run: `cargo test --locked --workspace && cargo clippy --locked --workspace --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 4: Hardware smoke test (Linux, R22 on and paired)**

Run: `cargo run -- print examples/first-label.png`
Expected: the label prints and the JSON report has `"outcome"` set as before.
If no printer is available, record that this step was skipped and don't
claim it passed.

- [ ] **Step 5: Commit**

```bash
git add host/src/job.rs host/src/lib.rs cli/src/main.rs
git commit -m "refactor: share the connect, handshake and print sequence"
```

---

### Task 4: `rongta-template` crate, bundled font, placeholders

**Files:**
- Create: `rongta-rs/template/Cargo.toml`, `rongta-rs/template/src/lib.rs`,
  `rongta-rs/template/src/interpolate.rs`, `rongta-rs/template/fonts/*`
- Modify: `rongta-rs/Cargo.toml`, `flake.nix`

**Interfaces:**
- Produces:
  - crate-private `fonts::{REGULAR, BOLD}: &'static [u8]`
  - `crate::interpolate::{Piece, parse(&str) -> Result<Vec<Piece>>, fill(&[Piece], impl Fn(&str) -> String) -> String}` (crate-private)

- [ ] **Step 1: Add the font files**

```bash
cd rongta-rs
mkdir -p template/fonts template/src template/tests template/examples
FONT=$(nix build --no-link --print-out-paths nixpkgs#atkinson-hyperlegible-next)
cp "$FONT/share/fonts/truetype/AtkinsonHyperlegibleNext-Regular.ttf" template/fonts/
cp "$FONT/share/fonts/truetype/AtkinsonHyperlegibleNext-Bold.ttf" template/fonts/
chmod 644 template/fonts/*.ttf
curl -fsSL https://raw.githubusercontent.com/googlefonts/atkinson-hyperlegible-next/main/OFL.txt -o template/fonts/OFL.txt
head -3 template/fonts/OFL.txt
```

Expected: the license names The Atkinson Hyperlegible Next Project Authors and
SIL OFL 1.1. Use the static faces, not `AtkinsonHyperlegibleNext[wght].ttf`:
`ab_glyph` would draw a variable font at its default instance only.

- [ ] **Step 2: Create `rongta-rs/template/Cargo.toml`**

```toml
[package]
name = "rongta-template"
version = "0.1.0"
edition = "2024"
description = "Fill-in label templates rendered for Rongta printers"
publish = false

[dependencies]
rongta = { path = ".." }
ab_glyph = "0.2"
anyhow = "1"
image = { version = "0.25", default-features = false }
percent-encoding = "2"
qrcode = { version = "0.14", default-features = false, features = ["std"] }
regex = "1"
serde = { version = "1", features = ["derive"] }
toml = "0.9"

[dev-dependencies]
rqrr = "0.11"
```

Add `"template"` to `members` and `default-members` in `rongta-rs/Cargo.toml`.

- [ ] **Step 3: Write the failing placeholder tests**

Create `rongta-rs/template/src/interpolate.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::{Piece::*, *};

    #[test]
    fn literals_fields_and_escaped_braces() {
        assert_eq!(
            parse("a{code}b{{c}}").unwrap(),
            vec![
                Literal("a".into()),
                Field("code".into()),
                Literal("b{c}".into())
            ]
        );
        let pieces = parse("x={code}&y={code}").unwrap();
        assert_eq!(fill(&pieces, |f| f.to_uppercase()), "x=CODE&y=CODE");
    }

    #[test]
    fn malformed_placeholders_are_rejected() {
        for source in ["{", "{}", "}", "{a b}", "{a"] {
            assert!(parse(source).is_err(), "{source}");
        }
    }
}
```

Create `rongta-rs/template/src/lib.rs`:

```rust
//! Fill-in label templates: typed fields, lines of text on the left and a QR
//! code on the right, drawn at the printer's dot pitch.

mod interpolate;

/// Atkinson Hyperlegible Next (SIL Open Font License 1.1; see `fonts/OFL.txt`).
/// Its distinct letterforms keep codes like `O0Il1` readable at thermal
/// resolution, and bundling it makes labels identical on every host.
mod fonts {
    pub static REGULAR: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Regular.ttf");
    pub static BOLD: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Bold.ttf");
}
```

- [ ] **Step 4: Run to verify it fails**

Run: `cargo test -p rongta-template`
Expected: compile errors, `Piece`, `parse` and `fill` not found.

- [ ] **Step 5: Implement**

Put this above the `#[cfg(test)]` module in `interpolate.rs`:

```rust
//! `{field}` placeholders; `{{` and `}}` are literal braces.
use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Literal(String),
    Field(String),
}

pub fn parse(source: &str) -> Result<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => {
                            name.push(c)
                        }
                        Some(c) => bail!("Invalid character {c:?} in a placeholder in {source:?}"),
                        None => bail!("Unclosed placeholder in {source:?}"),
                    }
                }
                if name.is_empty() {
                    bail!("Empty placeholder in {source:?}");
                }
                if !literal.is_empty() {
                    pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                }
                pieces.push(Piece::Field(name));
            }
            '}' => bail!("Unmatched }} in {source:?}; write }}}} for a literal brace"),
            c => literal.push(c),
        }
    }
    if !literal.is_empty() {
        pieces.push(Piece::Literal(literal));
    }
    Ok(pieces)
}

pub fn fill(pieces: &[Piece], value: impl Fn(&str) -> String) -> String {
    pieces
        .iter()
        .map(|piece| match piece {
            Piece::Literal(text) => text.clone(),
            Piece::Field(name) => value(name),
        })
        .collect()
}
```

Clippy will report `dead_code` until later tasks use these. Add
`#![allow(dead_code)]` at the top of `interpolate.rs` (Task 5 Step 5 removes
it) and `#[allow(dead_code)]` on `mod fonts` (Task 6 Step 5 removes it).

- [ ] **Step 6: Run the tests**

Run: `cargo test -p rongta-template`
Expected: 2 passed.

- [ ] **Step 7: Include fonts and templates in the Nix source filter**

In `flake.nix`, change the `filter` line to:

```nix
              # Tests embed JSON fixtures; the template and GUI crates embed fonts.
              filter =
                path: type:
                craneLib.filterCargoSources path type
                || pkgs.lib.any (suffix: pkgs.lib.hasSuffix suffix path) [ ".json" ".ttf" ".toml" ];
```

Run: `nix build .#rongta` (from the repository root)
Expected: builds. The test phase runs the new crate's tests.

- [ ] **Step 8: Commit**

```bash
git add template Cargo.toml Cargo.lock ../flake.nix
git commit -m "feat: add the template crate with a bundled label font"
```

---

### Task 5: Template schema, normalization and validation

**Files:**
- Modify: `rongta-rs/template/src/lib.rs`
- Create: `rongta-rs/template/examples/access-code.rongta.toml`,
  `rongta-rs/template/tests/template.rs`

**Interfaces:**
- Consumes: `interpolate::{Piece, parse, fill}` (Task 4).
- Produces:
  - `pub type Values = BTreeMap<String, String>;`
  - `pub enum Case { Preserve, Upper, Lower }`
  - `pub enum ErrorCorrection { L, M, Q, H }`
  - `pub struct Field { pub id: String, pub label: String, pub hint: Option<String>, pub case: Case, /* private */ pattern: Option<Regex> }`
  - `pub struct FieldError { pub field: String, pub message: String }` with `Display` = `"{field}: {message}"`
  - `pub struct Template { pub name: String, pub media: Media, pub fields: Vec<Field>, /* private */ ... }`
  - `Template::parse(&str) -> Result<Template>`, `Template::load(&Path) -> Result<Template>`
  - `Template::normalize(&self, &Values) -> Values` (trims, applies case)
  - `Template::check(&self, &Values) -> Vec<FieldError>` (empty = valid)
  - `Template::printable_size(&self) -> (u32, u32)`

- [ ] **Step 1: Write the example template**

`rongta-rs/template/examples/access-code.rongta.toml`:

```toml
version = 1
name = "Access code"
media = "gap:50x25"

[[field]]
id = "code"
label = "Access code"
hint = "6 to 12 letters or digits"
pattern = "[A-Z0-9]{6,12}"
case = "upper"

[qr]
content = "https://example.com/redeem?code={code}"
error_correction = "M"

[[text]]
content = "Scan to redeem"
size_mm = 3.0

[[text]]
content = "{code}"
size_mm = 5.0
bold = true
```

- [ ] **Step 2: Write the failing tests**

`rongta-rs/template/tests/template.rs`:

```rust
use rongta_template::{Case, Template, Values};

const EXAMPLE: &str = include_str!("../examples/access-code.rongta.toml");

fn values(pairs: &[(&str, &str)]) -> Values {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn example_template_parses() {
    let template = Template::parse(EXAMPLE).unwrap();
    assert_eq!(template.name, "Access code");
    assert_eq!(template.media.to_string(), "gap:50x25");
    assert_eq!(template.fields.len(), 1);
    assert_eq!(template.fields[0].id, "code");
    assert_eq!(template.fields[0].case, Case::Upper);
    assert_eq!(template.printable_size(), (384, 184));
}

#[test]
fn values_are_trimmed_cased_and_checked_against_the_whole_pattern() {
    let template = Template::parse(EXAMPLE).unwrap();
    let normalized = template.normalize(&values(&[("code", "  ab12cd ")]));
    assert_eq!(normalized["code"], "AB12CD");
    assert!(template.check(&normalized).is_empty());

    let missing = template.check(&Values::new());
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].to_string(), "code: Required");

    // The pattern must match the whole value, not a substring.
    let long = template.normalize(&values(&[("code", "AB12CD-EXTRA")]));
    assert_eq!(
        template.check(&long)[0].message,
        "6 to 12 letters or digits"
    );
}

#[test]
fn invalid_templates_explain_the_problem() {
    for (source, needle) in [
        (EXAMPLE.replace("version = 1", "version = 2"), "version"),
        (EXAMPLE.replace("gap:50x25", "circle:40"), "gap"),
        (EXAMPLE.replace("code={code}", "code={coed}"), "coed"),
        (EXAMPLE.replace("[A-Z0-9]{6,12}", "[A-Z"), "pattern"),
        (EXAMPLE.replace("size_mm = 3.0", "size_mm = 40.0"), "size_mm"),
        (format!("{EXAMPLE}\nsurprise = true\n"), "surprise"),
    ] {
        let error = format!("{:#}", Template::parse(&source).unwrap_err());
        assert!(error.contains(needle), "{needle:?} not in {error:?}");
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p rongta-template --test template`
Expected: compile errors, `Template`, `Case` and `Values` not found.

- [ ] **Step 4: Implement**

Replace `rongta-rs/template/src/lib.rs` with:

```rust
//! Fill-in label templates: typed fields, lines of text on the left and a QR
//! code on the right, drawn at the printer's dot pitch.

mod interpolate;

/// Atkinson Hyperlegible Next (SIL Open Font License 1.1; see `fonts/OFL.txt`).
/// Its distinct letterforms keep codes like `O0Il1` readable at thermal
/// resolution, and bundling it makes labels identical on every host.
mod fonts {
    pub static REGULAR: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Regular.ttf");
    pub static BOLD: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Bold.ttf");
}

use anyhow::{Context, Result, bail, ensure};
use interpolate::Piece;
use regex::Regex;
use rongta::media::Media;
use serde::Deserialize;
use std::{collections::BTreeMap, fmt, path::Path};

/// Field values by field id.
pub type Values = BTreeMap<String, String>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Case {
    #[default]
    Preserve,
    Upper,
    Lower,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum ErrorCorrection {
    L,
    #[default]
    M,
    Q,
    H,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
    pub case: Case,
    pattern: Option<Regex>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}
impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

#[derive(Clone, Debug)]
struct TextLine {
    content: Vec<Piece>,
    size_mm: f64,
    bold: bool,
}

#[derive(Clone, Debug)]
pub struct Template {
    pub name: String,
    pub media: Media,
    pub fields: Vec<Field>,
    qr: Vec<Piece>,
    error_correction: ErrorCorrection,
    lines: Vec<TextLine>,
    width_mm: f64,
    height_mm: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    version: u32,
    name: String,
    media: String,
    #[serde(default, rename = "field")]
    fields: Vec<RawField>,
    qr: RawQr,
    #[serde(default, rename = "text")]
    lines: Vec<RawText>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    id: String,
    label: String,
    hint: Option<String>,
    pattern: Option<String>,
    #[serde(default)]
    case: Case,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQr {
    content: String,
    #[serde(default)]
    error_correction: ErrorCorrection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawText {
    content: String,
    size_mm: f64,
    #[serde(default)]
    bold: bool,
}

impl Template {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read template {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("Invalid template {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let raw: RawTemplate = toml::from_str(text)?;
        ensure!(
            raw.version == 1,
            "Unsupported template version {}; this app reads version 1",
            raw.version
        );
        let media: Media = raw.media.parse()?;
        let Media::Gap {
            width_mm,
            height_mm,
        } = media
        else {
            bail!("Templates support gap labels only, not {media}");
        };

        let mut fields: Vec<Field> = Vec::new();
        for field in raw.fields {
            ensure!(
                !field.id.is_empty()
                    && field
                        .id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "Field id {:?} must use only letters, digits, - and _",
                field.id
            );
            ensure!(
                fields.iter().all(|f| f.id != field.id),
                "Field {:?} is declared twice",
                field.id
            );
            let pattern = field
                .pattern
                .map(|p| Regex::new(&format!("^(?:{p})$")))
                .transpose()
                .with_context(|| format!("Field {:?} has an invalid pattern", field.id))?;
            fields.push(Field {
                id: field.id,
                label: field.label,
                hint: field.hint,
                case: field.case,
                pattern,
            });
        }

        let declared = |pieces: &[Piece]| -> Result<()> {
            for piece in pieces {
                if let Piece::Field(name) = piece {
                    ensure!(
                        fields.iter().any(|f| &f.id == name),
                        "{{{name}}} does not name a declared field"
                    );
                }
            }
            Ok(())
        };
        let qr = interpolate::parse(&raw.qr.content).context("In [qr] content")?;
        declared(&qr)?;
        let mut lines = Vec::new();
        for line in raw.lines {
            ensure!(
                line.size_mm.is_finite() && (1.0..=20.0).contains(&line.size_mm),
                "Text size_mm must be between 1 and 20, not {}",
                line.size_mm
            );
            let content = interpolate::parse(&line.content).context("In [[text]] content")?;
            declared(&content)?;
            lines.push(TextLine {
                content,
                size_mm: line.size_mm,
                bold: line.bold,
            });
        }

        Ok(Self {
            name: raw.name,
            media,
            fields,
            qr,
            error_correction: raw.qr.error_correction,
            lines,
            width_mm,
            height_mm,
        })
    }

    /// Trims surrounding whitespace and applies each field's case rule.
    /// Values for undeclared ids are dropped.
    pub fn normalize(&self, values: &Values) -> Values {
        self.fields
            .iter()
            .filter_map(|field| {
                let value = values.get(&field.id)?.trim();
                let value = match field.case {
                    Case::Preserve => value.to_owned(),
                    Case::Upper => value.to_uppercase(),
                    Case::Lower => value.to_lowercase(),
                };
                Some((field.id.clone(), value))
            })
            .collect()
    }

    /// One error per invalid field; empty when every field may be rendered.
    pub fn check(&self, values: &Values) -> Vec<FieldError> {
        self.fields
            .iter()
            .filter_map(|field| {
                let value = values.get(&field.id).map(String::as_str).unwrap_or("");
                let message = if value.is_empty() {
                    "Required".to_owned()
                } else if value.chars().any(char::is_control) {
                    "Contains control characters".to_owned()
                } else if let Some(pattern) = &field.pattern
                    && !pattern.is_match(value)
                {
                    field
                        .hint
                        .clone()
                        .unwrap_or_else(|| "Doesn't match the expected format".to_owned())
                } else {
                    return None;
                };
                Some(FieldError {
                    field: field.id.clone(),
                    message,
                })
            })
            .collect()
    }

    /// Size in dots of the area `render` draws.
    pub fn printable_size(&self) -> (u32, u32) {
        rongta::image_processing::printable_size(self.width_mm, self.height_mm)
    }
}
```

- [ ] **Step 5: Run the tests**

Remove the `#![allow(dead_code)]` line from `interpolate.rs`, then run:
`cargo test -p rongta-template`
Expected: all pass. Clippy may still flag the unused `qr`, `lines` and
`error_correction` fields and `mod fonts`. Task 6 uses them, so keep
`#[allow(dead_code)]` on `Template` and `mod fonts` for this commit only;
Task 6 removes both.

- [ ] **Step 6: Commit**

```bash
git add template
git commit -m "feat: parse and validate label templates"
```

---

### Task 6: Render templates (text, QR code, raster)

**Files:**
- Create: `rongta-rs/template/src/qr.rs`, `rongta-rs/template/src/text.rs`,
  `rongta-rs/template/src/render.rs`
- Modify: `rongta-rs/template/src/lib.rs` (module declarations; remove the
  temporary `allow`), `rongta-rs/template/tests/template.rs`

**Interfaces:**
- Consumes: `Template` internals (Task 5), `printable_size`/`place` (Task 1),
  `rongta::raster::Raster::from_rgba(&RgbaImage, threshold: u8)`.
- Produces:
  - `Template::render(&self, values: &Values) -> Result<image::RgbaImage>`: the
    printable area, one pixel per dot, black or white only. It errors with the
    joined `FieldError`s if `check` fails.
  - `Template::raster(&self, values: &Values, head_width: u32) -> Result<Raster>`:
    the full label canvas, ready for `Session::print`. Plan B calls this with
    `head_width = 384` for previews.

- [ ] **Step 1: Write the failing tests**

Append to `rongta-rs/template/tests/template.rs`:

```rust
fn decode_qr(image: &image::RgbaImage) -> String {
    let gray = image::DynamicImage::ImageRgba8(image.clone()).into_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    let grids = prepared.detect_grids();
    assert_eq!(grids.len(), 1, "expected exactly one QR code");
    grids[0].decode().unwrap().1
}

#[test]
fn rendered_label_has_a_scannable_qr_with_the_encoded_value() {
    let template = Template::parse(EXAMPLE).unwrap();
    let values = template.normalize(&values(&[("code", "ab12cd")]));
    let image = template.render(&values).unwrap();
    assert_eq!(image.dimensions(), (384, 184));
    assert!(image.pixels().all(|p| p.0 == [0, 0, 0, 255] || p.0 == [255; 4]));
    assert_eq!(decode_qr(&image), "https://example.com/redeem?code=AB12CD");
    // The text column holds ink; it lies left of the QR's 148-dot square.
    let text_ink = image
        .enumerate_pixels()
        .filter(|(x, _, p)| *x < 384 - 148 && p.0[0] == 0)
        .count();
    assert!(text_ink > 200, "{text_ink}");
}

#[test]
fn qr_values_are_percent_encoded_but_text_is_not() {
    let source = EXAMPLE
        .replace("pattern = \"[A-Z0-9]{6,12}\"\n", "")
        .replace("case = \"upper\"\n", "");
    let template = Template::parse(&source).unwrap();
    let image = template.render(&values(&[("code", "a b&c")])).unwrap();
    assert_eq!(
        decode_qr(&image),
        "https://example.com/redeem?code=a%20b%26c"
    );
}

#[test]
fn raster_fills_the_label_canvas() {
    let template = Template::parse(EXAMPLE).unwrap();
    let raster = template
        .raster(&values(&[("code", "AB12CD")]), 384)
        .unwrap();
    assert_eq!((raster.width(), raster.height()), (384, 200));
}

#[test]
fn render_refuses_invalid_values_and_oversized_content() {
    let template = Template::parse(EXAMPLE).unwrap();
    let error = template.render(&Values::new()).unwrap_err().to_string();
    assert!(error.contains("code: Required"), "{error}");

    let long_url = EXAMPLE.replace(
        "https://example.com/redeem?code={code}",
        &format!("https://example.com/{}?code={{code}}", "x".repeat(300)),
    );
    let error = Template::parse(&long_url)
        .unwrap()
        .render(&values(&[("code", "AB12CD")]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("QR"), "{error}");
}
```

Add `image = { version = "0.25", default-features = false }` to
`[dev-dependencies]` in `template/Cargo.toml`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p rongta-template --test template`
Expected: compile errors, no method `render` or `raster`.

- [ ] **Step 3: Implement the QR drawing — `template/src/qr.rs`**

```rust
//! QR codes drawn with whole-dot modules; resampled modules print as grey
//! smears that phone cameras misread.
use crate::ErrorCorrection;
use anyhow::{Context, Result, ensure};
use image::{GrayImage, Luma};
use qrcode::{EcLevel, QrCode};

/// The QR specification's minimum quiet zone, in modules.
const QUIET_MODULES: u32 = 4;
/// Smallest module, in 0.125 mm dots, that scans reliably off thermal paper.
const MIN_MODULE_DOTS: u32 = 3;

impl From<ErrorCorrection> for EcLevel {
    fn from(level: ErrorCorrection) -> Self {
        match level {
            ErrorCorrection::L => EcLevel::L,
            ErrorCorrection::M => EcLevel::M,
            ErrorCorrection::Q => EcLevel::Q,
            ErrorCorrection::H => EcLevel::H,
        }
    }
}

/// Draws `data` as large as the canvas height allows, flush right, and
/// returns the x where its quiet zone begins.
pub fn draw(canvas: &mut GrayImage, data: &str, level: ErrorCorrection) -> Result<u32> {
    let code = QrCode::with_error_correction_level(data, level.into())
        .context("The QR content is too long to encode")?;
    let modules = code.width() as u32;
    let module = canvas.height() / (modules + 2 * QUIET_MODULES);
    ensure!(
        module >= MIN_MODULE_DOTS,
        "The QR code needs {modules} modules per side, too many to scan on this label; \
         shorten the link or use a taller label"
    );
    let side = (modules + 2 * QUIET_MODULES) * module;
    ensure!(
        side <= canvas.width(),
        "The QR code is wider than this label"
    );
    let left = canvas.width() - side;
    let top = (canvas.height() - side) / 2;
    for (index, dark) in code.to_vec().into_iter().enumerate() {
        if !dark {
            continue;
        }
        let x0 = left + (QUIET_MODULES + index as u32 % modules) * module;
        let y0 = top + (QUIET_MODULES + index as u32 / modules) * module;
        for y in y0..y0 + module {
            for x in x0..x0 + module {
                canvas.put_pixel(x, y, Luma([0]));
            }
        }
    }
    Ok(left)
}
```

- [ ] **Step 4: Implement the text drawing — `template/src/text.rs`**

```rust
//! Single-line text at the printer's dot pitch.
use crate::fonts;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use anyhow::{Result, ensure};
use image::{GrayImage, Luma};

/// 2 mm: smaller text loses strokes on a 203 dpi head.
const MIN_SIZE_DOTS: f32 = 16.0;

pub struct Line {
    pub text: String,
    pub size_dots: f32,
    pub bold: bool,
}

fn font(bold: bool) -> FontRef<'static> {
    FontRef::try_from_slice(if bold { fonts::BOLD } else { fonts::REGULAR })
        .expect("bundled font is valid")
}

fn advance(font: &FontRef<'_>, scale: PxScale, text: &str) -> f32 {
    let scaled = font.as_scaled(scale);
    let mut width = 0.0;
    let mut previous = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(previous) = previous {
            width += scaled.kern(previous, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

/// Stacks `lines` left-aligned and vertically centred within the first
/// `column` dots of `canvas`, shrinking any line too wide to fit.
pub fn draw(canvas: &mut GrayImage, lines: &[Line], column: u32) -> Result<()> {
    let mut fitted = Vec::with_capacity(lines.len());
    for line in lines {
        let font = font(line.bold);
        let natural = advance(&font, PxScale::from(line.size_dots), &line.text);
        let size = if natural > column as f32 {
            (line.size_dots * column as f32 / natural).floor()
        } else {
            line.size_dots
        };
        ensure!(
            size >= MIN_SIZE_DOTS,
            "{:?} is too long to fit beside the QR code",
            line.text
        );
        fitted.push((font, PxScale::from(size), line.text.as_str()));
    }
    let heights: Vec<f32> = fitted
        .iter()
        .map(|(font, scale, _)| font.as_scaled(*scale).height())
        .collect();
    let total: f32 = heights.iter().sum();
    ensure!(
        total <= canvas.height() as f32,
        "The text is too tall for this label"
    );

    let mut top = (canvas.height() as f32 - total) / 2.0;
    for ((font, scale, text), height) in fitted.iter().zip(&heights) {
        let scaled = font.as_scaled(*scale);
        let baseline = top + scaled.ascent();
        let mut x = 0.0;
        let mut previous = None;
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            if let Some(previous) = previous {
                x += scaled.kern(previous, id);
            }
            let glyph = id.with_scale_and_position(*scale, point(x, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    // Dots are on or off; a half-coverage cut keeps strokes at
                    // their designed weight.
                    if coverage >= 0.5
                        && px >= 0
                        && py >= 0
                        && (px as u32) < column
                        && (py as u32) < canvas.height()
                    {
                        canvas.put_pixel(px as u32, py as u32, Luma([0]));
                    }
                });
            }
            x += scaled.h_advance(id);
            previous = Some(id);
        }
        top += height;
    }
    Ok(())
}
```

- [ ] **Step 5: Implement `render` and `raster` — `template/src/render.rs`**

```rust
use crate::{Template, Values, interpolate, qr, text};
use anyhow::{Result, ensure};
use image::{DynamicImage, GrayImage, Luma, RgbaImage};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rongta::{image_processing, raster::Raster};

/// RFC 3986 unreserved characters pass through; everything else in a value is
/// encoded so it cannot change the URL's structure.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

impl Template {
    pub fn render(&self, values: &Values) -> Result<RgbaImage> {
        let errors = self.check(values);
        ensure!(
            errors.is_empty(),
            "{}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
        let (width, height) = self.printable_size();
        let mut canvas = GrayImage::from_pixel(width, height, Luma([255]));
        let url = interpolate::fill(&self.qr, |name| {
            utf8_percent_encode(&values[name], COMPONENT).to_string()
        });
        let column = qr::draw(&mut canvas, &url, self.error_correction)?;
        let lines: Vec<text::Line> = self
            .lines
            .iter()
            .map(|line| text::Line {
                text: interpolate::fill(&line.content, |name| values[name].clone()),
                size_dots: (line.size_mm * 8.0) as f32,
                bold: line.bold,
            })
            .collect();
        text::draw(&mut canvas, &lines, column)?;
        Ok(DynamicImage::ImageLuma8(canvas).into_rgba8())
    }

    pub fn raster(&self, values: &Values, head_width: u32) -> Result<Raster> {
        let image = self.render(values)?;
        // The image is already pure black and white, so any mid threshold is exact.
        let content = Raster::from_rgba(&image, 127)?;
        image_processing::place(
            &content,
            image.dimensions(),
            &self.media,
            head_width,
            (self.height_mm * 8.0) as u32,
        )
    }
}
```

In `lib.rs`, add `mod qr; mod render; mod text;` after `mod interpolate;`,
and delete the temporary `#[allow(dead_code)]` attributes on `Template` and
`mod fonts`.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p rongta-template`
Expected: all pass. If the QR test fails to detect a grid, check that the
quiet zone isn't clipped: `left` must leave 4 white modules on every side.

- [ ] **Step 7: Look at the output**

```bash
cargo test -p rongta-template --test template -- --nocapture
```

Then write a scratch render to look at by hand. This is done in Task 7 with
`rongta template ... --output`, so you can skip it here.

- [ ] **Step 8: Commit**

```bash
git add template
git commit -m "feat: render templates with whole-dot QR codes and text"
```

---

### Task 7: `rongta template` command and docs

**Files:**
- Modify: `rongta-rs/cli/Cargo.toml`, `rongta-rs/cli/src/main.rs`, `README.md`
- Create: `rongta-rs/cli/tests/template.rs`

**Interfaces:**
- Consumes: `Template::{load, normalize, check, raster}` (Tasks 5–6),
  `rongta_host::job::{print, Target}` (Task 3).
- Produces: the subcommand
  `rongta template TEMPLATE [NAME=VALUE...] [--output PNG] [--address MAC] [--adapter NAME] [--pin PIN] [--timeout S] [--copies N] [--density D] [-v]`.
  With `--output`, it saves a 384-dot preview and never connects.

- [ ] **Step 1: Write the failing CLI tests**

`rongta-rs/cli/tests/template.rs`:

```rust
use std::process::Command;

fn example() -> String {
    format!(
        "{}/../template/examples/access-code.rongta.toml",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn rongta() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rongta"));
    for (key, _) in std::env::vars().filter(|(key, _)| key.starts_with("RONGTA_")) {
        command.env_remove(key);
    }
    command
}

#[test]
fn preview_saves_the_full_label_canvas() {
    let dir = std::env::temp_dir().join(format!("rongta-template-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("label.png");
    let output = rongta()
        .args(["template", &example(), "code=ab12cd", "--output"])
        .arg(&png)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let image = image::open(&png).unwrap();
    assert_eq!((image.width(), image.height()), (384, 200));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_values_fail_before_connecting() {
    for (args, needle) in [
        (vec!["code=no"], "6 to 12 letters or digits"),
        (vec![], "code: Required"),
        (vec!["colour=red"], "no field \"colour\""),
        (vec!["code"], "NAME=VALUE"),
    ] {
        let output = rongta()
            .args(["template", &example()])
            .args(args)
            .args(["--address", "00:00:00:00:00:00"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{needle:?} not in {stderr}");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p rongta-cli --test template`
Expected: both fail because the `template` subcommand is unknown.

- [ ] **Step 3: Implement**

Add `rongta-template = { path = "../template" }` to `cli/Cargo.toml`
`[dependencies]`.

In `cli/src/main.rs`, add `Template(TemplateArgs),` to `enum Commands` after
`Print(Print),`, and add:

```rust
/// Fill in a label template, then print it or save a preview
///
/// Values are NAME=VALUE pairs for the template's fields. Templates set their
/// own media, so the saved media selection is not used.
#[derive(Args)]
struct TemplateArgs {
    /// Template file (.rongta.toml)
    template: PathBuf,
    /// Field values as NAME=VALUE
    values: Vec<String>,
    /// Save a PNG preview instead of printing
    #[usage(long)]
    output: Option<PathBuf>,
    /// Override the selected printer for this job
    #[usage(long)]
    address: Option<String>,
    /// Linux only: BlueZ adapter; defaults to the system's default adapter
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
    /// Log raw transmitted/received bytes to stderr
    #[usage(long, short = 'v')]
    verbose: bool,
}
```

Add the match arm before `Commands::Decode`:

```rust
        Commands::Template(args) => {
            let template = rongta_template::Template::load(&args.template)?;
            let values = template_values(&template, &args.values)?;
            if let Some(path) = &args.output {
                let raster = template.raster(&values, 384)?;
                rongta::image_processing::preview(&raster)
                    .save_with_format(path, image::ImageFormat::Png)
                    .with_context(|| format!("Cannot save preview {}", path.display()))?;
                writeln!(output, "Saved preview to {}", path.display())?;
            } else {
                let settings = Storage::discover(cli.config)?.effective()?;
                let address = resolve_address(args.address.as_deref(), settings.printer.address)?;
                let address = validate_connection(&address, args.timeout, args.pin.as_deref())?;
                ensure!(
                    (1..=100).contains(&args.copies),
                    "--copies must be between 1 and 100"
                );
                ensure!(args.density <= 15, "--density must be between 0 and 15");
                if settings.printer.media.as_ref() != Some(&template.media) {
                    eprintln!(
                        "This template is for {} labels; check that they are loaded.",
                        template.media
                    );
                }
                init_logging(args.verbose);
                let (_, report) = rongta_host::job::print(
                    rongta_host::job::Target {
                        address,
                        adapter: args.adapter.as_deref(),
                        pin: args.pin,
                        timeout: Duration::from_secs(args.timeout),
                    },
                    |info| template.raster(&values, info.model.width_dots),
                    rongta::printing::PrintOptions {
                        copies: args.copies,
                        density: args.density,
                        timeout: Duration::from_secs(args.timeout),
                        ..Default::default()
                    },
                    |_| {},
                )
                .await?;
                serde_json::to_writer_pretty(&mut output, &report)?;
                writeln!(output)?;
            }
        }
```

And the helper, next to `resolve_media`:

```rust
fn template_values(
    template: &rongta_template::Template,
    pairs: &[String],
) -> Result<rongta_template::Values> {
    let mut values = rongta_template::Values::new();
    for pair in pairs {
        let (name, value) = pair
            .split_once('=')
            .with_context(|| format!("Expected NAME=VALUE, got {pair:?}"))?;
        ensure!(
            template.fields.iter().any(|field| field.id == name),
            "This template has no field {name:?}"
        );
        values.insert(name.to_owned(), value.to_owned());
    }
    let values = template.normalize(&values);
    let errors = template.check(&values);
    ensure!(
        errors.is_empty(),
        "{}",
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    );
    Ok(values)
}
```

Add `image = { version = "0.25", default-features = false, features = ["png"] }`
to `[dev-dependencies]` in `cli/Cargo.toml` for the test's PNG read.

- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --workspace && cargo clippy --locked --workspace --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 5: Look at a preview**

```bash
cargo run -- template template/examples/access-code.rongta.toml code=AB12CD --output "$TMPDIR/label.png"
```

Open the PNG. Check: the QR is on the right with a clear margin, "Scan to
redeem" and "AB12CD" sit left and are vertically centred, nothing is clipped,
and there's a 1 mm blank band at the top and bottom.

- [ ] **Step 6: Hardware check (if a printer and 50 × 25 mm labels are available)**

```bash
cargo run -- template template/examples/access-code.rongta.toml code=AB12CD
```

Scan the printed QR with a phone. Expected: it opens
`https://example.com/redeem?code=AB12CD`. If you can't do this, record it as
not done.

- [ ] **Step 7: Document**

Add a `## Label templates` section to `README.md` after the Rust library and CLI
section. Hard-wrap it at 80 columns. It should cover: the template format (copy
the annotated example from this plan's "Template format" section), the `rongta
template` command with a preview example, the fixed layout (QR right, text
left), and that the font is Atkinson Hyperlegible Next under the OFL. Also
mention the `rongta-host` and `rongta-template` crates in the workspace package
list near the top of that section.

- [ ] **Step 8: Commit**

```bash
git add cli template ../README.md Cargo.lock
git commit -m "feat: print and preview label templates from the CLI"
```

---

## Self-review notes

- Spec coverage: image printing is already in the CLI. Template printing:
  Tasks 4–7. Shared host for the GUI: Tasks 2–3. GUI and packaging: Plan B.
- `Template::raster(values, head_width)` and `rongta_host::job::print` are the
  two entry points Plan B relies on. Their signatures are fixed above.
