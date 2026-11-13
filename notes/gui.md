# Desktop app

## What it is

`rongta-gui` is an eframe/egui window for printing to the R22 without the
command line. It has two tabs and a bar across the top:

- **Template** opens a label template, shows a form with one input per field,
  and checks each value against the template's pattern as you type.
- **Image** opens a picture and prints it, with the same layout, dither and
  threshold choices as `rongta print`.
- The **printer bar** scans for printers, picks one, and sets the loaded media.
  A status bar at the bottom shows scan, connect and print progress.

Settings are the CLI's own: the selected printer, model and media live in the
same saved state file, so a printer chosen with `rongta settings` is already
selected in the app, and the reverse. Bluetooth and printing go through
`rongta-host`, the same runner the CLI uses.

The preview is the exact print raster, not an approximation. It is produced by
the same code that builds the bytes sent to the printer, so margins, the
printable-area crop and the monochrome conversion are what will come out.

## Building

For development, from the repository root:

```sh
nix build .#rongta && ./result/bin/rongta-gui
```

or, in the dev shell, from `rongta-rs/`:

```sh
cargo run -p rongta-gui
```

On Linux, winit and wgpu load the windowing and graphics libraries at run
time. The dev shell puts them on `LD_LIBRARY_PATH`, and the Nix package wraps
`rongta-gui` to do the same.

Distributable artifacts (a macOS app, AppImages, a Windows zip) come from the
distribution plan, which will fill in this section. Until then, on Windows,
run `cargo build --release -p rongta-gui` on a machine with the MSVC
toolchain and zip `rongta-gui.exe`. No console window opens, because the
binary is built with `windows_subsystem`.

## Sending it to someone on a Mac

The app is ad-hoc signed, not notarized, so macOS blocks it the first time.
These are the macOS 15 and later steps to read out over the phone:

- Double-click the app. It's blocked; click **Done**. (Right-click, then Open,
  no longer works around this on macOS 15 and later.)
- Open **System Settings > Privacy & Security**, scroll down, click **Open
  Anyway** next to Rongta Labels, and enter the password.
- Click **Allow** when asked about Bluetooth.
- If any of that fails, or macOS says the app "is damaged", run
  `xattr -dr com.apple.quarantine "/Applications/Rongta Labels.app"` in
  Terminal.
- Each new build is a new ad-hoc signature, so expect to repeat "Open Anyway",
  and possibly the Bluetooth prompt, after every update. Browsers, Mail,
  Messages and AirDrop all quarantine the download, so changing how you send
  it doesn't help.
- Notarizing with a paid Apple Developer account would remove all of this.

## Templates

The template format is described under Label templates in the
[README](../README.md#label-templates). In the app, open a template with
**Open...**, by dragging the file onto the window, or by passing its path as
the first argument. The last is what "Open with" does on Windows and Linux.

## Status

The GUI has not been run against a printer on any platform yet. Only the CLI
has printed, and only on Linux. macOS and Windows are untested on hardware,
for the GUI and the CLI alike; their Bluetooth backends type-check in cross
builds only.

The UI itself is covered by headless egui_kittest snapshot tests. The
snapshots are in `rongta-rs/gui/tests/snapshots/`, rendered with Mesa's
lavapipe software Vulkan driver, and regenerated with:

```sh
UPDATE_SNAPSHOTS=true cargo test -p rongta-gui
```
