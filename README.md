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

## Rust CLI

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
cargo test --locked
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
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

The printer heuristic matches the Bluetooth imaging/printer device class or a
case-insensitive name containing `RONGTA`, `Printer`, or the token `R22`. The
class bits follow [Bluetooth Assigned Numbers, section 2.8.2.6](https://www.bluetooth.com/wp-content/uploads/Files/Specification/HTML/Assigned_Numbers/out/en/index-en.html).
It is not a compatibility check, and generic names or missing classes can hide
real printers. Use `--all` if your printer is missing. SPP alone is not enough
to classify a device as a printer.

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
Pixels are dots: it centers narrower images on a full-width white canvas and
rejects wider images rather than resizing. Transparency is composited on white;
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
