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
information query over Classic Bluetooth SPP, plus offline protocol tools.
Framing and handshake behavior are checked against the original Android SDK
and simulated printer streams. The Bluetooth connection still needs its first
hardware test; printing is not implemented yet. With the dev shell loaded:

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
Bluetooth as described below before running `info`. Find the printer's MAC with
`bluetoothctl scan on` (then `scan off`). From `rongta-rs/`:

```sh
cargo run -- info AA:BB:CC:DD:EE:FF --adapter hci0 --verbose
# Save the result and timestamped TX/RX hex logs for comparison tomorrow:
cargo run -- info AA:BB:CC:DD:EE:FF --verbose > info.json 2> info.log
```

`info` discovers the selected MAC if BlueZ does not know it yet, pairs if needed,
and resolves the SPP channel through BlueZ/SDP. Pairing uses your existing BlueZ
agent. If necessary, pair interactively with `bluetoothctl` first, or add
`--pin YOUR_PRINTER_PIN` for legacy PIN pairing. The printer's PIN is not yet
known; none is assumed. No root privileges are normally required when your user
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
