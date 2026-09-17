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

The Rust project lives in `rongta-rs/`. The initial scaffold exposes help and
version information. Printer commands and protocol handling are not implemented
yet. Run these commands from the repository root with the dev shell loaded:

```sh
cd rongta-rs
cargo run -- --help
cargo run -- --version
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

Dependency choices and protocol boundaries are recorded in
[notes/rust-cli.md](notes/rust-cli.md). `rongta-rs/Cargo.lock` pins the resolved
dependencies.
The planned native transport uses host BlueZ and Bluetooth Classic SPP/RFCOMM.
Stop the emulator bridge and restore host Bluetooth as described below before
using native Bluetooth commands once they are implemented.

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
