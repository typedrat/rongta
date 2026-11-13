# Desktop app

## What it is

`rongta-gui` is an eframe/egui window for printing to the R22 without the
command line. It has two tabs and a bar across the top:

- **Template** opens a label template, shows a form with one input per field,
  and checks each value against the template's pattern as you type.
- **Image** opens a picture and prints it. It either fills the label or shows
  the whole picture, in one of four styles: sharp black and white (threshold
  208) for text and logos, two photo dithers, and a patterned one. The label
  size defaults to the printer's saved media and can be typed in.
- The **printer bar** scans for printers and picks one. A status bar at the
  bottom shows connect and print progress.

Settings are the CLI's own: the selected printer, model and media live in the
same saved state file, so a printer chosen with `rongta settings` is already
selected in the app, and the reverse. The app only changes the printer; set the
loaded media with `rongta settings`. Bluetooth and printing go through
`rongta-host`, the same runner the CLI uses.

The preview is the exact print raster, not an approximation. It is produced by
the same code that builds the bytes sent to the printer, so margins, the
printable-area crop and the monochrome conversion are what will come out. It is
drawn at a whole number of screen pixels per printer dot, so no dot is dropped
or blurred; only a window too narrow for that shrinks it.

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

Nothing builds AppImages, a Windows zip or a macOS app yet. On Windows, run
`cargo build --release -p rongta-gui` on a machine with the MSVC toolchain and
zip `rongta-gui.exe`. No console window opens, because the binary is built
with `windows_subsystem`.

## Sending it to someone on a Mac

There is no macOS app bundle yet. Once there is, it will be ad-hoc signed, not
notarized, so macOS will block it the first time. These are the macOS 15 and
later steps to read out over the phone:

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
the first argument. The last is what "Open with" does on Windows. On Linux,
"Open with" needs a `.desktop` entry, which nothing installs yet.

The app keeps its own copy of each template it opens and reopens the last one
at start, so the template still works after the emailed original is moved or
deleted. The copies are in the `templates` folder of rongta's data folder:
`~/.local/share/rongta/templates` on Linux (or under `$XDG_DATA_HOME`),
`~/Library/Application Support/rongta/templates` on macOS, and
`%APPDATA%\rongta\data\templates` on Windows. Only which template was open is
remembered. Typed values are not, since each label's access code is different
and one left filled in could print a duplicate. A file passed on the command
line opens instead of the remembered one.

## Status

The GUI has not been run against a printer on any platform yet. Only the CLI
has printed, and only on Linux. macOS and Windows are untested on hardware,
for the GUI and the CLI alike; their Bluetooth backends type-check in cross
builds only.

The UI itself is covered by headless egui_kittest snapshot tests. The
snapshots are in `rongta-rs/gui/tests/snapshots/` and are rendered with Mesa's
lavapipe software Vulkan driver; the tests refuse to run on any other
renderer, because a GPU draws a few pixels differently. In the dev shell,
`gui-snapshots` runs the GUI tests with lavapipe as the only Vulkan driver,
and re-records the images with:

```sh
UPDATE_SNAPSHOTS=force gui-snapshots
```

Plain `cargo test` in the dev shell also finds lavapipe, which the shell adds
alongside the GPU. The Nix package's checks run every snapshot test on the same
lavapipe.
