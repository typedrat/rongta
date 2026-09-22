# Rongta R22 investigation

The flake uses flake-parts and numtide/devshell to provide an x86_64 Linux
development shell with Android Emulator,
an Android 15 Google APIs x86_64 system image with ARM64 translation, ADB,
Bumble, BlueZ, Wireshark, and the Rust toolchain with D-Bus build dependencies.
The SDK configuration accepts the Android SDK license and enables unfree
packages. The first shell load downloads the SDK and system image.

```sh
direnv allow
# Alternatively:
nix develop
```

Entering the shell does not launch Android or change Bluetooth services.

## Rust library and CLI

`rongta-rs/` is a Cargo workspace with two packages:

- `rongta`: portable Bluetooth discovery traits and printer detection, framing,
  handshake, media geometry, image processing, and printing over a caller-supplied
  Tokio `AsyncRead + AsyncWrite + Unpin` stream.
- `rongta-cli` (`rongta-rs/cli/`): the Linux `rongta` executable, including BlueZ
  discovery/transport, OS randomness, terminal UI, and file/environment settings.

Other applications can depend on the library without pulling in BlueZ, D-Bus,
or CLI dependencies. No feature flags are needed:

```toml
[dependencies]
rongta = { path = "/path/to/rongta/rongta-rs" }
```

Use `rongta::client::Session::new(stream)` with your platform's transport and
pass a caller-generated nonce to `handshake`. Async sessions need a Tokio
runtime with time enabled; pure framing, handshake state, and image processing
do not need a runtime. Media types are available at `rongta::media::Media`.
For discovery, implement `rongta::bluetooth::Discovery` with your platform's API
and call `rongta::bluetooth::scan` with `DeviceFilter::Printers` or
`DeviceFilter::All`. `ScanDevice<Id>` keeps the backend's identifier type, so it
can hold a MAC address, UUID, or native handle. The library applies the shared
stock-app name rules; the backend handles adapter selection and discovery
resources. Discovery futures need not be `Send`. The CLI's `BlueZDiscovery`
implements this same interface for both `scan` and the settings picker.

The library uses `std`; this split does not add other platform Bluetooth backends.

From `rongta-rs/`, `cargo check -p rongta --lib` and `cargo test -p rongta`
select only the portable package. Workspace-wide builds include the Linux CLI.
`cargo run -- ...` still selects the `rongta` executable. To install it from
source, run `cargo install --path cli --locked`.

The Rust project lives in `rongta-rs/`. It implements the R22 handshake and
information query and image job submission over Classic Bluetooth SPP, plus
offline protocol tools. Framing, handshake, image encoding, and print controls
are checked against the original Android SDK and simulated printer streams.
The first R22 hardware test verified pairing, handshake, and the small test
label. Transfer and completion details are in [hardware notes](notes/hardware.md).
With the dev shell loaded:

```sh
cd rongta-rs
cargo run -- --help
cargo run -- --version
cargo run -- discovery > /tmp/rongta-discovery.bin
cargo run -- decode /tmp/rongta-discovery.bin
cargo test --locked --workspace
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Dependency choices and protocol boundaries are recorded in
[notes/rust-cli.md](notes/rust-cli.md). `rongta-rs/Cargo.lock` pins the resolved
dependencies.
The native transport uses host BlueZ. Stop the emulator bridge and restore host
Bluetooth as described below before running `scan`, `info`, or `print`. From `rongta-rs/`:

```sh
cargo run -- scan --adapter hci0 --timeout 10
# Use the printer address from the scan result:
cargo run -- info AA:BB:CC:DD:EE:FF --adapter hci0 --verbose
# Save the result and timestamped TX/RX hex logs for comparison tomorrow:
cargo run -- info AA:BB:CC:DD:EE:FF --verbose > info.json 2> info.log
```

`scan` searches Classic Bluetooth and BLE for 10 seconds, then shows an
address-sorted table of likely printers by default. `--printers` is enabled by
default; `--all` disables the filter. `--json` outputs an array instead of a table:

```sh
cargo run -- scan                       # likely printers, readable table
cargo run -- scan --all                 # include all devices
cargo run -- scan --json                # likely printers as JSON
cargo run -- scan --all --json          # all devices as JSON
```

The printer filter ports the name rules from RONGTA 3.1.6's normal app channel:
names must match a model in its built-in list, including its permitted variant
and separator suffixes. Blank names and names ending in `BLE` (ignoring case)
are excluded. For example, `R22`, `R22_1234`, and `R22Pro` match; `My R22`,
`RONGTA Printer`, and `R22_BLE` do not. The stock app's case quirk is preserved:
`r22-1234` matches but `r22Pro` does not. Device class and SPP UUIDs do not affect
this filter. The settings picker uses the same rules.

Classic+BLE discovery is unchanged. Only the stock app's name rules are ported;
its Android device-type check and app-channel restrictions are not applied.
The filter is not a compatibility check: discovery recognizes all stock model
names, while this CLI currently supports printing only with R22. Use `--all`
if your printer is missing. See [discovery filter notes](notes/discovery.md) for
the source mapping and Java reference fixtures.

The table shows address, RSSI in dBm, known SPP support, pairing state, whether
BlueZ knew the device before scanning, and its name. JSON additionally includes
`class` and `connected`. Missing properties are null in JSON. Progress goes to
stderr. Scanning does not pair or connect. BlueZ includes cached devices, so a
listed address is not proof of current proximity. `known_before_scan` (`KNOWN`
in the table) identifies entries that already existed. `spp` reflects only
BlueZ's known service UUIDs: false (`unlisted`) or null (`?`) does not rule out
SPP, since service discovery may require a connection.

An empty result gives a readable message (or `[]` with `--json`); adapter and
discovery failures return nonzero. `--timeout` sets the discovery interval;
adapter setup and reading result properties can take additional time.

`info` discovers the selected MAC if BlueZ does not know it yet, pairs if needed,
and resolves the SPP channel through BlueZ/SDP. Pairing uses your existing BlueZ
agent. If necessary, pair interactively with `bluetoothctl` first, or add
`--pin YOUR_PRINTER_PIN` for legacy PIN pairing. The tested R22 paired through BlueZ without an explicit PIN; none is assumed. No root privileges are normally required when your user
has BlueZ access. The CLI does not change adapter power or host services.

Output is JSON containing protocol parameters, serials, device/firmware/hardware
versions, model geometry, battery readings, and verified challenge fields.
Battery `voltage_hundredths` is hundredths of a volt; `percent_tenths` is tenths
of a percent. Enum values are preserved as numbers pending hardware evidence.
The command only succeeds once every handshake step completes; failed checks,
malformed frames, disconnects, and timeouts return nonzero without partial JSON.
`--timeout SECONDS` defaults to 30, applied once to connection/pairing and again
to the entire handshake (not reset per reply). `--verbose` logs raw received
bytes before parsing and submitted command bytes to stderr. `RUST_LOG` overrides
the default log filter. TX submission is not proof the printer received bytes.
Keep wanted logs in the ignored `captures/` directory; these logs are text,
not inputs to `decode` without extracting and concatenating their hex bytes.

`decode` accepts a filename or reads stdin and outputs one JSON object per
frame. Input is raw application data from the RFCOMM stream, not a whole
btsnoop/pcap file. Its default inner-message limit is 2050 bytes, matching the
SDK's receive decoder; `--max-body-len` overrides that for offline inspection.

The [native reference harness](tools/native-reference/README.md) regenerates
fixtures using the original library in Android. The
[framing notes](notes/framing.md) document the layout and CRC;
[handshake notes](notes/handshake.md) cover the proof, reply layouts, and evidence
limits.

## Send a print job

Run `rongta settings` for a terminal menu to select a printer (scan or enter its
MAC), model, and loaded media. The menu has explicit Save and Cancel actions;
changing printers clears the old media selection. R22 is currently the only
model. Selection does not pair with or print to the device.

From `rongta-rs/`, the equivalent noninteractive workflow is:

```sh
cargo run -- settings --address AA:BB:CC:DD:EE:FF --model r22 --media gap:50x30
cargo run -- status                         # effective selection and storage paths
cargo run -- status --json
cargo run -- info                           # query the selected printer
cargo run -- preview label.png --output /tmp/label-preview.png
cargo run -- print label.png
```

Saved selections live in `directories::ProjectDirs::state_dir()` (local data
directory fallback where the platform has no state directory). On Linux this
is `$XDG_STATE_HOME/rongta/state.toml`, normally `~/.local/state/rongta/state.toml`.
Updates are locked and atomically replaced. A menu opened before another process
changes the selection refuses to overwrite the newer state.

`confique` loads optional TOML overrides from the platform config directory,
normally `~/.config/rongta/config.toml`. `--config PATH` or `RONGTA_CONFIG` selects
an alternative file; an explicitly selected file must exist. Example:

```toml
[printer]
address = "AA:BB:CC:DD:EE:FF"
model = "r22"

[printer.media]
type = "gap"
width_mm = 50
height_mm = 30
```

Saved state uses the same `[printer]` and `[printer.media]` blocks, with a
top-level `version = 2`. Older version-1 selections load without modification
and migrate to the new structure on the next settings save. `status --json`
also groups the address, model, and structured media under `printer`.
Circular media uses `type = "circle"` with `diameter_mm`; continuous stock uses
`type = "continuous"` with `width_mm`. Media is replaced as a complete value
when overriding configuration; dimensions are never merged across media types.

For these settings, precedence is explicit command arguments, then
`RONGTA_ADDRESS` / `RONGTA_MODEL` / `RONGTA_MEDIA`, then the chosen config file,
then saved state. The default model is R22. `--config` takes precedence over
`RONGTA_CONFIG`. The menu and `settings` flags always edit saved state; they
do not rewrite config files or persist environment overrides. The menu shows
active overrides, and `status` shows their effective values. A different
printer address supplied by config, environment, or a command does not inherit
the previous printer's saved media; select media for that printer explicitly.

CLI and environment media shorthand uses millimetres: `gap:50x30`, `circle:40`,
or `continuous:50`. Config files and saved state use the structured tables above.
The menu contains the original app's R22 presets and a custom-size input.
`settings --clear-media` restores pixel-for-pixel image handling. Width is
limited to 50 mm; fixed/derived lengths must exceed 2 mm and are limited to
1000 mm by this implementation. These checks are not a claim of hardware
validation for every size. Media selection controls image geometry; it does
not change the printer's paper-sensor configuration.

With media selected, `print` and `preview` use the R22 app's 8 dots/mm geometry.
The default `--layout label` treats the input as a full-label design, crops its
central printable area (width capped at 48 mm, height reduced by 2 mm), and
centres it on a white full-head-width raster. A 50 × 30 mm design corresponds
to 400 × 240 pixels; the printable crop is 384 × 224, sent on a 384 × 240 raster.
Circular media additionally masks pixels outside the circle. `--layout fit`
fits the entire image proportionally within the printable area. Resampling
uses the `image` crate's triangle filter; exact Android resampling equivalence
has not been established. By default, monochrome conversion uses the SDK
threshold (`--dither none`, `--threshold 208`).

For photos, use `--dither floyd-steinberg`. This adapts the 1-bit path from
`synapdeck-togo/nix/packages/img2idx/img2idx.py`: composite transparency on white
in linear light, resize linear RGB with Lanczos3, compute Rec.709 luminance,
then dither to black/white. Diffusion alternates direction each row. Additional
methods are `atkinson`, `jarvis`, `stucki`, `sierra`, `sierra-lite`, and an 8×8
ordered `bayer` pattern. `--threshold` only applies to `--dither none`.
The reference's tone adjustments and blue-noise generation are not ported.

The decoded R22 raster format has **one bit per dot**, so gray tones are
represented by patterns of black dots. The shared SDK's grayscale enum does
not establish grayscale support on the R22. Dithering runs before adding white
feed margins and the circular media mask. No additional dependencies are needed.

```sh
cargo run -- preview photo.png --media gap:50x30 --layout fit --dither floyd-steinberg --output /tmp/photo-preview.png
cargo run -- print photo.png --layout fit --dither floyd-steinberg
cargo run -- print label.png --media gap:40x30    # one-job override
cargo run -- print strip.png --media continuous:50 --length-mm 80
cargo run -- print prepared.png --raw             # image pixels are dots
cargo run -- print label.png --address AA:BB:CC:DD:EE:FF --media gap:50x30
```

Continuous media derives job length from image aspect ratio unless
`--length-mm` is supplied. `preview` has the same media/layout/length/raw and
threshold/dither options as `print`; its PNG represents the packed monochrome raster,
including margins. It uses a 384-dot head by default (`--head-width` overrides
this); printing reads the actual head width during handshake. Offset adjustments
are not yet exposed. The offline `image` command remains a raw pixel/protocol
tool and does not apply saved media. It accepts `--dither` but does not resize.

With host Bluetooth restored and the dev shell loaded, from `rongta-rs/`:

```sh
# Small 128×64-dot asymmetric pattern for the first hardware test:
cargo run -- print AA:BB:CC:DD:EE:FF examples/first-label.png --verbose \
  > print-result.json 2> print.log
# Your own label, with optional copies and density (0 light, 8 normal, 15 dark):
cargo run -- print AA:BB:CC:DD:EE:FF label.png --copies 2 --density 8
```

`print` handshakes on the same connection it uses for the job. It takes the
buffer length and head width from the printer, checks status, announces the
app connection, sets print mode and density, then sends the compressed image.
Without selected media, or with `--raw`, pixels are dots: it centers narrower
images on a full-width white canvas and rejects wider images. Transparency is composited on white;
`--threshold` defaults to 208. Centering includes any byte-alignment padding.
Use a canvas matching your label and printer geometry for precise placement.

The default is one copy, normal density, no speed override, and no extra feed.
`--speed N` passes a raw SDK speed value; `--feed-length N` feeds forward after
each copy. Feed units still need hardware verification. Command batches use the
app's 100 ms delay (`--command-delay-ms`). By default, each complete image is
submitted as one buffer, matching the Android app. `--packet-delay-ms` forces
separate frame writes for diagnostics; 100 ms spacing produced incomplete output
on the tested R22, so leave it at zero. Status is polled every 500 ms and
unsolicited status notifications are processed throughout transmission.
Connection/pairing options match `info`; `--timeout` defaults to 120 seconds,
applied separately to connection, handshake, and submission/waiting for all copies.

JSON success means `sent_printer_idle`: the bytes were written, printing was
observed through a query or notification, and a later status reply was idle.
Each copy must have its own printing-to-idle cycle. Initial idle replies are
not completion: the tested printer starts after a delay. The report confirms
printer state, not image quality. Missing completion, faults, rejected commands,
malformed replies, disconnects, and timeouts fail without automatically retrying.
A failed or interrupted job can have been partially sent; check the printer
before retrying. See [print-job notes](notes/printing.md) for protocol details.

## Encode image data offline

With the dev shell loaded, from `rongta-rs/`:

```sh
# Replace 4096 with protocol.buffer_length from your printer's info output.
cargo run -- image label.png --buffer-length 4096 > /tmp/rongta-image.bin
cargo run -- decode /tmp/rongta-image.bin --max-body-len 65535
```

PNG and PNM are supported. Pixels are printer dots: there is no automatic
resizing. Prepare the canvas to the width reported by `info`; transparency is
composited onto white and partial rows are padded to eight dots. The default
black/white threshold is 208 (`--threshold` overrides it). Buffer length is
required explicitly; the example value is not a measured R22 capability.

Output contains image start, Huffman trees, compressed data, and image end.
It does not connect to Bluetooth or include copies, heat, feed, or pacing.
The offline decoder override allows tree frames larger than its default receive
limit. [Image-data notes](notes/image-data.md) describe the format, SDK reference
cases, and rejected native edge cases.

## Emulator commands

Run `menu` to list the available commands:

| Command | Purpose |
| --- | --- |
| `bt-bridge` | Bridge `hci0` to Android on port 8877 and capture HCI traffic. |
| `bt-prepare` | Stop host Bluetooth and take `hci0` down; run as root. |
| `emu` | Launch the `rongta-arm64` Pixel 5 AVD connected to that bridge, without snapshots. |

`bt-bridge` and `emu` forward extra arguments to the underlying program; for example,
`emu -no-window` starts the emulator without its GUI.

## Create the virtual phone

```sh
avdmanager create avd --name rongta-arm64 --device pixel_5 \
  --package 'system-images;android-35;google_apis;x86_64'
```

The Pixel 5 profile provides a 1080×2340 display at 440 dpi instead of the
default 320×640 display. Answer `no` if asked to create a custom hardware
profile. AVD state uses the Android tools' default user directory. The host
needs access to `/dev/kvm`. The new name preserves any older `rongta` AVD;
changing the SDK image in the flake does not upgrade existing AVDs.

## Bridge Bluetooth and capture traffic

Bumble takes exclusive ownership of the selected controller. The commands
below use `hci0` and temporarily disconnect the host's Bluetooth peripherals.
`bt-bridge` checks the adapter state without root. If it is UP, it exits with
instructions to run `bt-prepare`. An unreadable or unknown state also prevents
the bridge from starting.

Neither script invokes `sudo` or another privilege tool. Run `bt-prepare` as
root using your preferred method, then run `bt-bridge` with the privileges
needed for an HCI socket (normally root). Resolve the commands with
`command -v bt-prepare` and `command -v bt-bridge` if your privilege tool resets
PATH. The commands to run in the elevated environment are:

```sh
bt-prepare
bt-bridge
```

The HCI socket uses the kernel-managed controller, so no USB passthrough or
separate dongle is required to attempt this setup. The `{instance}` placeholder
keeps the two bridge transports from overwriting the same capture file. Each
run prints a fresh private directory under `/tmp/rongta-capture.*` and creates
both capture files there before starting Bumble. When launched through `sudo`,
the directory and files belong to the invoking user so Wireshark can read them
without root. Other elevation methods retain the launching user's ownership.

In another terminal with the dev shell loaded:

```sh
emu
```

After Android finishes booting, check the supported ABIs and install the
ARM64 XAPK splits extracted into `analysis/xapk-arm64/`:

```sh
adb -e shell getprop ro.product.cpu.abilist
adb -e install-multiple analysis/xapk-arm64/*.apk
adb -e shell am start -n com.rongta.pn81/.RootActivity
```

The ABI list must include `arm64-v8a` (the selected image reports
`x86_64,arm64-v8a`). The original XAPK in the project root contains only
`armeabi-v7a` libraries and cannot run on this image. Use the ARM64 variant;
do not mix its splits with those in `analysis/xapk/`. Close other emulators
before using `adb -e`, or select one explicitly with `adb -s SERIAL`.
RONGTA 3.1.6's ARM64 variant was verified to install and reach its main screen
on this Pixel 5 AVD. Bluetooth discovery through the host adapter was also
confirmed in Android's pairing screen. Printer communication still needs a
live test when the R22 arrives.

Select R22 in the app, connect, and print a tiny label. Stop the bridge before
opening the `.btsnoop` files in the printed directory in Wireshark. New runs
preserve earlier captures, but `/tmp` is temporary: copy any wanted captures
into `captures/` for longer-term storage.

After stopping the emulator and bridge, restore host Bluetooth by running
`systemctl start bluetooth.service` with appropriate privileges, then powering
the controller on as your normal user:

```sh
bluetoothctl power on
```

## References

- [Bumble Android Emulator bridge](https://google.github.io/bumble/platforms/android.html#connecting-a-custom-virtual-controller)
- [Bumble Linux HCI sockets](https://google.github.io/bumble/platforms/linux.html#using-hci-sockets)
- [NixOS Android tooling](https://wiki.nixos.org/wiki/Android)
