# GUI Distribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Nix derivations that turn the GUI into files you can hand to
someone: an AppImage for x86_64 and aarch64 Linux, a zip with a `.exe` for
x86_64 Windows (cross-compiled from Linux), and a zip with an ad-hoc signed
`.app` for Apple Silicon Macs. CI is generated from Nix with
`github-actions-nix`; it builds every artifact, smoke-tests each one on its
real OS, and attaches them to a GitHub release for `v*` tags.

**Architecture:** Two flake-parts modules under `nix/`. In `packaging.nix`, each
artifact is a chain of derivations: the crane build, then a platform-specific
bundle derivation, then a final archive or AppImage. Linux binaries are linked
by zig against glibc 2.28, so they use the host's glibc and graphics drivers
like a distro package would. The bundle derivations use `allowedReferences = [
]` wherever the result must not depend on `/nix/store`, so Nix itself rejects an
artifact that would only work on a Nix machine. In `ci.nix`, the workflow is
declared in Nix and written to `.github/workflows/`, and a flake check fails if
the committed YAML drifts from the Nix source. CI runners only install Nix and
call `nix build`; every build step is a derivation.

**Tech Stack:** Nix flakes, flake-parts, crane, `cargo-zigbuild` + `zig`
(Linux binaries against glibc 2.28), the static AppImage type 2 runtime
packaged by `ralismark/nix-appimage`, fenix (prebuilt Windows `rust-std`),
`pkgsCross` mingw-w64, `rcodesign`, `synapdeck/github-actions-nix`.

This is Plan C. It requires Plan B's `rongta-gui` crate and replaces Plan B's
ad-hoc macOS packaging step. Plan B, Task 6 now covers only the Linux runtime
wrapper and docs.

## Global Constraints

- Every artifact is built by a derivation. A CI step may only install Nix,
  restore a cache, run `nix build`/`nix flake check`, copy the result out, run
  a smoke test, or upload. No `cargo`, `brew` or `codesign` producing
  artifacts in CI. Installing packages a smoke test needs (FUSE, libdbus in a
  test container) is fine.
- Formatting follows the existing `flake.nix` style (nixfmt RFC style, two
  spaces).
- Comments explain *why* (a constraint, workaround or gotcha), for a reader
  seeing the file cold.
- Final artifact names: `Rongta-Labels-x86_64.AppImage`,
  `Rongta-Labels-aarch64.AppImage`, `Rongta-Labels-windows-x86_64.zip` and
  `Rongta-Labels-macos-arm64.zip`.
- macOS is arm64 only, because the pinned nixpkgs no longer builds
  x86_64-darwin. The app is ad-hoc signed and not notarized; see
  `notes/gui.md` for the Gatekeeper walkthrough.
- Don't claim an artifact works on an OS until it has been launched there,
  either by the CI smoke test or by a person. `--version` passing proves the
  bundle runs. It doesn't prove the window or Bluetooth work.

## Known risks (verify, don't assume)

1. **The GUI's link list.** The approach was proven with the CLI (Task 3
   records the evidence), but the GUI binary doesn't exist yet. If some crate
   links a library directly instead of `dlopen`ing it, the portable check
   fails. Task 3 Step 2 says how to fix that.
2. **What the host must provide.** glibc 2.28 or newer, `libdbus-1` (present
   on every systemd desktop), and graphics and display libraries (present on
   any desktop). Double-clicking the AppImage also needs FUSE (`fusermount3`,
   installed by default on current Ubuntu, Fedora and Debian). Without it,
   `--appimage-extract-and-run` works. NixOS can't run generic Linux binaries
   without `nix-ld`, so NixOS users should use the Nix package.
3. **Windows runtime DLLs.** nixpkgs' mingw toolchain may link
   `libmcfgthread-1.dll` or `libwinpthread-1.dll`. The packaging derivation
   audits the `.exe`'s imports and bundles or rejects each non-system DLL.

## File Structure

```
flake.nix                          # modify: inputs, import nix/*.nix, share crane args
flake.lock                         # modify
nix/packaging.nix                  # create: portable Linux binary + AppImage, windows zip, macOS zip
nix/ci.nix                         # create: workflow, write-workflows app, drift check
.github/workflows/ci.yml           # create (generated; committed)
rongta-rs/gui/src/main.rs          # modify: --version
rongta-rs/gui/tests/version.rs     # create
rongta-rs/gui/packaging/macos/Info.plist   # create
notes/gui.md                       # modify: Building section
```

---

### Task 1: `--version` for smoke tests, and the macOS Info.plist

**Files:**
- Modify: `rongta-rs/gui/src/main.rs` (top of `main`)
- Create: `rongta-rs/gui/tests/version.rs`,
  `rongta-rs/gui/packaging/macos/Info.plist`

**Interfaces:**
- Produces: `rongta-gui --version` prints `rongta-gui <CARGO_PKG_VERSION>`
  and exits 0 without opening a window. Tasks 3–6 smoke-test with it.

- [ ] **Step 1: Write the failing test**

`rongta-rs/gui/tests/version.rs`:

```rust
#[test]
fn version_prints_without_opening_a_window() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rongta-gui"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("rongta-gui {}\n", env!("CARGO_PKG_VERSION"))
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p rongta-gui --test version`
Expected: FAIL. The binary tries to open a window, treating `--version` as a
template path, and either errors or hangs. If it hangs on a desktop session,
close the window to let the test fail.

- [ ] **Step 3: Implement**

At the top of `main()` in `rongta-rs/gui/src/main.rs`, before the
`template_path` line:

```rust
    // Packaging smoke tests run this on machines without a display.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "--version") {
        println!("rongta-gui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
```

- [ ] **Step 4: Create the Info.plist**

`rongta-rs/gui/packaging/macos/Info.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Rongta Labels</string>
  <key>CFBundleDisplayName</key>
  <string>Rongta Labels</string>
  <key>CFBundleIdentifier</key>
  <string>dev.rongta.labels</string>
  <key>CFBundleExecutable</key>
  <string>rongta-gui</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>@VERSION@</string>
  <key>CFBundleVersion</key>
  <string>@VERSION@</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSBluetoothAlwaysUsageDescription</key>
  <string>Rongta Labels uses Bluetooth to find and print to your label printer.</string>
</dict>
</plist>
```

`@VERSION@` is substituted by the packaging derivation in Task 5.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p rongta-gui`
Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add rongta-rs/gui
git commit -m "feat: add --version to the GUI for packaging smoke tests"
```

---

### Task 2: Flake inputs and shared build arguments

**Files:**
- Modify: `flake.nix`, `flake.lock`
- Create: `nix/packaging.nix` (empty module for now), `nix/ci.nix` (empty module for now)

**Interfaces:**
- Produces, inside `perSystem`:
  - `_module.args.rongtaBuild = { craneLib, commonArgs, cargoArtifacts, rongta }`
    for the `nix/*.nix` modules
  - flake inputs `fenix`, `nix-appimage`, `github-actions-nix`

- [ ] **Step 1: Add the inputs**

In `flake.nix` `inputs`, add:

```nix
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-appimage = {
      url = "github:ralismark/nix-appimage";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    github-actions-nix = {
      url = "https://flakehub.com/f/synapdeck/github-actions-nix/*";
      inputs.nixpkgs.follows = "nixpkgs";
    };
```

- [ ] **Step 2: Share the crane setup and import the modules**

Change `imports = [ inputs.devshell.flakeModule ];` to:

```nix
      imports = [
        inputs.devshell.flakeModule
        inputs.github-actions-nix.flakeModules.default
        ./nix/packaging.nix
        ./nix/ci.nix
      ];
```

In the `let` block, give the dependency build a name so other modules can
reuse it:

```nix
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          rongta = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              # (the nativeBuildInputs/postInstall wrapper from Plan B stays here)
              meta.mainProgram = "rongta";
            }
          );
```

In the `perSystem` body, next to `_module.args.pkgs`, add:

```nix
          _module.args.rongtaBuild = {
            inherit craneLib commonArgs cargoArtifacts rongta;
          };
```

Create `nix/packaging.nix` and `nix/ci.nix`, both containing:

```nix
{ ... }:
{
}
```

- [ ] **Step 3: Lock and verify nothing changed**

Run (from the repository root):

```bash
nix flake lock
nix flake check -L
nix build .#rongta && ./result/bin/rongta --version
```

Expected: the check passes and the version prints. `flake.lock` gains
`fenix`, `nix-appimage` and `github-actions-nix`.

- [ ] **Step 4: Commit**

```bash
git add flake.nix flake.lock nix
git commit -m "build: add packaging inputs and share the crane build"
```

---

### Task 3: Portable Linux binary and AppImages (x86_64 and aarch64)

**Files:**
- Modify: `nix/packaging.nix`

**Interfaces:**
- Consumes: `rongtaBuild.{craneLib, commonArgs}` (Task 2), and the static
  AppImage type 2 runtime from the `nix-appimage` input.
- Produces, on `x86_64-linux` and `aarch64-linux`:
  - `packages.rongta-gui-portable`: `bin/rongta-gui`, linked against glibc
    2.28's symbol versions with the standard FHS loader path, so it runs on
    any distro with glibc 2.28 or newer
  - `checks.rongta-gui-portable`: fails if the binary needs a newer glibc, has
    an RPATH, uses a non-FHS interpreter, or links any library outside the
    allowlist
  - `packages.rongta-gui-appimage`: a single file named
    `Rongta-Labels-<arch>.AppImage`

**Why this shape:** the binary is an ordinary Linux binary that uses the
host's glibc and the host's graphics drivers, as a distro package would.
winit and wgpu `dlopen` Wayland, X11, xkbcommon, Vulkan and EGL at run time, so
those come from the host and include NVIDIA's proprietary driver. The only
non-glibc link dependency should be `libdbus-1` (from the BlueZ crate), which
every systemd desktop has. The AppImage just packs that binary and a desktop
entry into one file with the official static runtime. There's no `/nix/store`
inside, no chroot and no user namespace.

This was checked on 2026-11-12 with the CLI, which links the same BlueZ
stack. `cargo zigbuild --target x86_64-unknown-linux-gnu.2.28` (nixpkgs zig
0.17, cargo-zigbuild 0.23.4) produced a binary with interpreter
`/lib64/ld-linux-x86-64.so.2`, no RPATH, no `/nix/store` strings, newest
symbol `GLIBC_2.28`, and libraries `libdbus-1.so.3`, `libc`, `libm`,
`libpthread`, `libdl` and the loader. It ran on `debian:10` (glibc 2.28),
`ubuntu:24.04` and `fedora:43` with only the distro's libdbus installed. The
GUI's own library list hasn't been checked yet; the check derivation does
that.

- [ ] **Step 1: Write the derivations**

Replace `nix/packaging.nix` with this. Task 4 later adds to the same `let`
block and `packages` attrset.

```nix
{ inputs, ... }:
{
  perSystem =
    {
      pkgs,
      lib,
      system,
      rongtaBuild,
      ...
    }:
    let
      inherit (pkgs.stdenv.hostPlatform) isLinux;
      inherit (rongtaBuild) craneLib commonArgs;
      arch = pkgs.stdenv.hostPlatform.uname.processor;
      rustTarget = pkgs.stdenv.hostPlatform.rust.rustcTarget;

      # Debian 10, RHEL 8 and Ubuntu 20.04 ship this glibc; anything newer runs
      # binaries linked against it.
      glibcFloor = "2.28";
      interpreter =
        {
          x86_64 = "/lib64/ld-linux-x86-64.so.2";
          aarch64 = "/lib/ld-linux-aarch64.so.1";
        }
        .${arch};

      # zig links against a chosen glibc's symbol versions and writes the
      # standard loader path, unlike Nix's toolchain, whose binaries need the
      # store's own glibc.
      portableArgs = commonArgs // {
        pname = "rongta-gui-portable";
        cargoExtraArgs = "--locked -p rongta-gui --target ${rustTarget}.${glibcFloor}";
        cargoBuildCommand = "cargo zigbuild --profile release";
        # Plain `cargo check` rejects zigbuild's glibc-suffixed target triple.
        cargoCheckCommand = "true";
        doCheck = false;
        nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
          pkgs.cargo-zigbuild
          pkgs.zig
        ];
        # zig and cargo-zigbuild both keep caches under $HOME by default.
        preBuild = ''
          export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-cache"
          export XDG_CACHE_HOME="$TMPDIR/cache"
        '';
      };
      portable = craneLib.buildPackage (
        portableArgs
        // {
          cargoArtifacts = craneLib.buildDepsOnly portableArgs;
          allowedReferences = [ ];
          meta.mainProgram = "rongta-gui";
        }
      );

      portableCheck =
        pkgs.runCommand "rongta-gui-portable-check" { nativeBuildInputs = [ pkgs.binutils ]; }
          ''
            bin=${lib.getExe portable}
            if ! readelf -l "$bin" | grep -qF '[Requesting program interpreter: ${interpreter}]'; then
              echo "rongta-gui must use the FHS loader ${interpreter}" >&2
              readelf -l "$bin" | grep interpreter >&2
              exit 1
            fi
            if readelf -d "$bin" | grep -Eq '\((RPATH|RUNPATH)\)'; then
              echo "rongta-gui has an RPATH; it must find libraries the way the host does" >&2
              exit 1
            fi
            # Everything graphical is dlopened; a desktop is only guaranteed to
            # have glibc, libgcc and (with systemd) libdbus.
            extra=$(readelf -d "$bin" \
              | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p' \
              | grep -Ev '^(libc\.so\.6|libm\.so\.6|libdl\.so\.2|libpthread\.so\.0|librt\.so\.1|libgcc_s\.so\.1|ld-linux-[a-z0-9_-]+\.so\.[0-9]+|libdbus-1\.so\.3)$' \
              || true)
            if [ -n "$extra" ]; then
              echo "rongta-gui links libraries a desktop may not have:" >&2
              echo "$extra" >&2
              exit 1
            fi
            newest=$(objdump -T "$bin" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1)
            if [ "$(printf '%s\n%s\n' "$newest" ${glibcFloor} | sort -V | tail -1)" != ${glibcFloor} ]; then
              echo "rongta-gui needs glibc $newest; the floor is ${glibcFloor}" >&2
              exit 1
            fi
            touch $out
          '';

      appImageRuntime = inputs.nix-appimage.packages.${system}.appimage-runtimes.appimage-type2-runtime;
      # Label text uses Atkinson Hyperlegible Next and the UI uses Inter; the
      # OFL requires their licenses to ship with any copy of the fonts.
      fontLicenses = pkgs.runCommand "rongta-font-licenses" { } ''
        mkdir $out
        cp ${../rongta-rs/template/fonts/OFL.txt} $out/AtkinsonHyperlegibleNext-OFL.txt
        cp ${../rongta-rs/gui/fonts/LICENSE.txt} $out/Inter-OFL.txt
      '';
      desktopEntry = pkgs.writeText "rongta-labels.desktop" ''
        [Desktop Entry]
        Type=Application
        Name=Rongta Labels
        Comment=Print labels on a Rongta printer
        Exec=rongta-gui %f
        Categories=Office;
        Terminal=false
      '';
      appImage =
        pkgs.runCommand "Rongta-Labels-${arch}.AppImage"
          {
            nativeBuildInputs = [ pkgs.squashfsTools ];
            allowedReferences = [ ];
          }
          ''
            mkdir -p AppDir/usr/bin
            cp ${lib.getExe portable} AppDir/usr/bin/rongta-gui
            # The runtime mounts the image and executes AppRun from it.
            ln -s usr/bin/rongta-gui AppDir/AppRun
            cp ${desktopEntry} AppDir/rongta-labels.desktop
            # The SIL OFL requires each bundled font's license to travel with it.
            mkdir -p AppDir/usr/share/licenses/rongta-labels
            cp ${fontLicenses}/* AppDir/usr/share/licenses/rongta-labels/
            # A type 2 AppImage is the runtime followed directly by a squashfs.
            offset=$(stat -L -c%s ${appImageRuntime})
            mksquashfs AppDir $out -offset "$offset" -all-root -noappend -no-xattrs -quiet
            dd if=${appImageRuntime} of=$out conv=notrunc status=none
            chmod 755 $out
          '';
    in
    {
      packages = lib.optionalAttrs isLinux {
        rongta-gui-portable = portable;
        rongta-gui-appimage = appImage;
      };
      checks = lib.optionalAttrs isLinux {
        rongta-gui-portable = portableCheck;
      };
    };
}
```

- [ ] **Step 2: Build the portable binary and run its check**

```bash
nix build -L .#rongta-gui-portable
nix build -L .#checks.x86_64-linux.rongta-gui-portable
```

Expected: both succeed. Possible failures:

- *crane installs nothing* (`no binaries found`): crane finds binaries
  through cargo's JSON build log. Check that `cargo zigbuild` passed
  `--message-format json-render-diagnostics` through. If it didn't, set
  `installPhaseCommand` to copy `target/${rustTarget}/release/rongta-gui` into
  `$out/bin`.
- *The check reports an extra library*: a crate is linking something directly
  instead of `dlopen`ing it. Find it with `cargo tree -p rongta-gui -i <crate>`
  and enable that crate's `dlopen` feature, or vendor it statically. Don't
  widen the allowlist without checking that every desktop distro ships the
  library.
- *Linking fails against Nix's libdbus*: enable the `dbus` crate's `vendored`
  feature for Linux in `rongta-rs/host/Cargo.toml`, which builds libdbus in,
  and remove `libdbus-1\.so\.3` from the allowlist.
- *`allowedReferences` fails*: run
  `strings result/bin/rongta-gui | grep /nix/store` to find the embedded path.

- [ ] **Step 3: Build the AppImage and run it on other distros**

NixOS has no `/lib64/ld-linux-x86-64.so.2` unless `nix-ld` is enabled, so
test in containers:

```bash
nix build -L .#rongta-gui-appimage
cp -L result "$TMPDIR/Rongta-Labels.AppImage"
for image in debian:10 ubuntu:24.04 fedora:latest; do
  echo "== $image"
  docker run --rm -e APPIMAGE_EXTRACT_AND_RUN=1 \
    -v "$TMPDIR/Rongta-Labels.AppImage:/app.AppImage:ro" "docker.io/library/$image" sh -c '
      # Debian 10 is archived; its packages moved off the main mirrors.
      grep -qs "^10" /etc/debian_version && echo "deb http://archive.debian.org/debian buster main" > /etc/apt/sources.list
      if command -v apt-get >/dev/null; then
        apt-get -qq update >/dev/null && apt-get -qq install -y libdbus-1-3 >/dev/null
      else
        dnf -q -y install dbus-libs >/dev/null
      fi
      /app.AppImage --version'
done
```

Expected: `rongta-gui 0.1.0` from all three. `APPIMAGE_EXTRACT_AND_RUN`
avoids FUSE, which containers don't have.

- [ ] **Step 4: Open the window on a real non-Nix desktop**

If a non-Nix desktop machine or VM is available (ideally stock Ubuntu 24.04),
copy the AppImage there, `chmod +x` it, and double-click it. Expected: the
themed window opens with hardware rendering. Check with
`WGPU_DEBUG=1`/`RUST_LOG=wgpu_core=info` that the adapter is the real GPU, not
llvmpipe. Choosing a printer should work too. Record the result, including the
distro and GPU, or that this wasn't done.

On NixOS, run it with `nix run nixpkgs#appimage-run -- "$TMPDIR/Rongta-Labels.AppImage"`
for a quick look. It's not a substitute for a non-Nix desktop.

- [ ] **Step 5: aarch64**

If an aarch64 Linux builder is configured (`nix config show | grep builders`,
or binfmt with `extra-platforms` including `aarch64-linux`), run
`nix build -L .#packages.aarch64-linux.rongta-gui-appimage .#checks.aarch64-linux.rongta-gui-portable`.
Otherwise CI's `ubuntu-24.04-arm` job (Task 6) is the first aarch64 build.
Record which.

- [ ] **Step 6: Commit**

```bash
git add nix/packaging.nix
git commit -m "build: portable glibc 2.28 GUI binary and AppImages"
```

---

### Task 4: Windows x86_64 zip, cross-compiled

**Files:**
- Modify: `nix/packaging.nix`

**Interfaces:**
- Produces, on `x86_64-linux` only:
  - `packages.rongta-gui-windows-exe`: the crane cross build, with
    `bin/rongta-gui.exe`
  - `packages.rongta-gui-windows`: a file `Rongta-Labels-windows-x86_64.zip`
    holding `Rongta Labels/Rongta Labels.exe`, plus any non-system DLLs it
    imports

- [ ] **Step 1: Write the derivations**

In `nix/packaging.nix`'s `let` block, add:

```nix
      # Rust's windows-gnu target expects the msvcrt C runtime, not UCRT.
      pkgsWindows = import inputs.nixpkgs {
        localSystem = system;
        crossSystem = {
          config = "x86_64-w64-mingw32";
          libc = "msvcrt";
        };
      };
      # fenix ships a prebuilt Windows std; nixpkgs would compile one from source.
      windowsToolchain =
        let
          f = inputs.fenix.packages.${system};
        in
        f.combine [
          f.stable.cargo
          f.stable.rustc
          f.targets.x86_64-pc-windows-gnu.stable.rust-std
        ];
      craneWindows = (inputs.crane.mkLib pkgsWindows).overrideToolchain (_: windowsToolchain);
      windowsArgs = {
        inherit (rongtaBuild.commonArgs) src;
        strictDeps = true;
        pname = "rongta-gui";
        cargoExtraArgs = "--locked -p rongta-gui";
        CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
        # Tests run natively on Linux; a Windows test binary can't run here.
        doCheck = false;
      };
      windowsExe = craneWindows.buildPackage (
        windowsArgs // { cargoArtifacts = craneWindows.buildDepsOnly windowsArgs; }
      );
      # Anything outside this list must ship beside the .exe or be rejected.
      windowsSystemDlls = lib.concatStringsSep "|" [
        "advapi32" "bcrypt" "bluetoothapis" "cfgmgr32" "combase"
        "comctl32" "comdlg32" "crypt32" "d3d12" "d3dcompiler_47" "dwmapi" "dxgi"
        "gdi32" "hid" "imm32" "kernel32" "msvcrt" "ntdll" "ole32" "oleaut32"
        "opengl32" "powrprof" "propsys" "secur32" "setupapi" "shell32" "shlwapi"
        "synchronization" "user32" "userenv" "uxtheme" "version" "winmm"
        "ws2_32" "api-ms-win-[a-z0-9-]+"
      ];
      windowsRuntimeDlls = with pkgsWindows.windows; [
        mcfgthreads
        pthreads
      ];
```

And in `packages`, add:

```nix
      // lib.optionalAttrs (system == "x86_64-linux") {
        rongta-gui-windows-exe = windowsExe;
        rongta-gui-windows =
          pkgs.runCommand "Rongta-Labels-windows-x86_64.zip"
            {
              nativeBuildInputs = [
                pkgs.zip
                pkgsWindows.buildPackages.binutils
              ];
              # A zip that mentions /nix/store would only work next to Nix.
              allowedReferences = [ ];
            }
            ''
              dir="Rongta Labels"
              mkdir "$dir"
              cp ${windowsExe}/bin/rongta-gui.exe "$dir/Rongta Labels.exe"
              mkdir "$dir/licenses"
              cp ${fontLicenses}/* "$dir/licenses/"
              for dll in $(x86_64-w64-mingw32-objdump -p "$dir/Rongta Labels.exe" \
                  | awk '/DLL Name:/ { print tolower($3) }'); do
                # Win32's Bluetooth device APIs are imported from bthprops.cpl.
                if echo "$dll" | grep -Eqx '(${windowsSystemDlls})\.dll|bthprops\.cpl'; then
                  continue
                fi
                found=$(find ${lib.concatMapStringsSep " " (d: "${d}") windowsRuntimeDlls} \
                  -iname "$dll" -print -quit)
                if [ -z "$found" ]; then
                  echo "rongta-gui.exe imports $dll, which is neither a Windows system DLL nor bundled" >&2
                  exit 1
                fi
                cp "$found" "$dir/"
              done
              zip -r -X $out "$dir"
            '';
      }
```

Two facts above are unverified. The attribute names `pkgsWindows.windows.mcfgthreads`
and `.pthreads` should be checked with
`nix eval --raw .#…` or `nix repl` before building. Drop whichever doesn't
exist; the audit loop fails loudly if a needed DLL is missing.

- [ ] **Step 2: Build**

```bash
nix build -L .#rongta-gui-windows-exe
x86_64-w64-mingw32-objdump -p result/bin/rongta-gui.exe | grep 'DLL Name' || \
  nix shell nixpkgs#pkgsCross.mingwW64.buildPackages.binutils -c \
    x86_64-w64-mingw32-objdump -p result/bin/rongta-gui.exe | grep 'DLL Name'
nix build -L .#rongta-gui-windows
unzip -l result
```

Expected: a DLL list (record it in the commit message) and a zip with
`Rongta Labels/Rongta Labels.exe` plus any bundled runtime DLLs. If
`allowedReferences` fails, a store path is embedded in the exe. Find it with
`strings result/bin/rongta-gui.exe | grep /nix/store` and fix it at the source
(usually crane's vendored-source reference removal, or a `--remap-path-prefix`).

- [ ] **Step 3: Optional local smoke test under Wine**

```bash
unzip -q -o result -d "$TMPDIR/win"
nix run nixpkgs#wineWow64Packages.stable -- "$TMPDIR/win/Rongta Labels/Rongta Labels.exe" --version
```

Expected: `rongta-gui 0.1.0`. Wine passing is encouraging but not proof; the
CI job on `windows-latest` (Task 6) is the real check.

- [ ] **Step 4: Commit**

```bash
git add nix/packaging.nix
git commit -m "build: cross-compile the GUI for Windows and zip it"
```

---

### Task 5: macOS app zip (built on aarch64-darwin)

**Files:**
- Modify: `nix/packaging.nix`

**Interfaces:**
- Consumes: `rongtaBuild.rongta` on `aarch64-darwin`,
  `rongta-rs/gui/packaging/macos/Info.plist` (Task 1).
- Produces, on `aarch64-darwin`: `packages.rongta-gui-macos`, a file
  `Rongta-Labels-macos-arm64.zip` holding an ad-hoc signed
  `Rongta Labels.app`

- [ ] **Step 1: Write the derivation**

In `packages`, add:

```nix
      // lib.optionalAttrs (system == "aarch64-darwin") {
        rongta-gui-macos =
          pkgs.runCommand "Rongta-Labels-macos-arm64.zip"
            {
              nativeBuildInputs = [
                pkgs.cctools
                pkgs.rcodesign
                pkgs.zip
              ];
              # The .app must run on a Mac without Nix.
              allowedReferences = [ ];
            }
            ''
              app="Rongta Labels.app"
              bin="$app/Contents/MacOS/rongta-gui"
              mkdir -p "$app/Contents/MacOS"
              cp ${rongtaBuild.rongta}/bin/rongta-gui "$bin"
              chmod u+w "$bin"
              substitute ${../rongta-rs/gui/packaging/macos/Info.plist} "$app/Contents/Info.plist" \
                --replace-fail @VERSION@ ${rongtaBuild.rongta.version}
              mkdir -p "$app/Contents/Resources/licenses"
              cp ${fontLicenses}/* "$app/Contents/Resources/licenses/"

              # nixpkgs links a few system libraries by store path; macOS ships
              # the same ones in /usr/lib.
              for lib in $(otool -L "$bin" | awk '/\/nix\/store\// { print $1 }'); do
                name=$(basename "$lib")
                case "$name" in
                  libiconv.2.dylib|libresolv.9.dylib|libc++.1.dylib|libc++abi.dylib|libz.1.dylib)
                    install_name_tool -change "$lib" "/usr/lib/$name" "$bin" ;;
                  *)
                    echo "rongta-gui links $lib, which macOS does not provide" >&2
                    exit 1 ;;
                esac
              done
              for rpath in $(otool -l "$bin" | awk '/LC_RPATH/ { getline; getline; print $2 }' | grep /nix/store || true); do
                install_name_tool -delete_rpath "$rpath" "$bin"
              done

              # Apple Silicon refuses unsigned code, and the edits above
              # invalidated the linker's signature. Without a certificate,
              # rcodesign signs ad hoc.
              rcodesign sign "$app"
              zip -r -y -X $out "$app"
            '';
      }
```

- [ ] **Step 2: Build it, if a Mac or remote aarch64-darwin builder is available**

```bash
nix build -L .#packages.aarch64-darwin.rongta-gui-macos
```

On a Mac:

```bash
unzip -q -o result -d "$TMPDIR/mac"
codesign --verify --deep --strict --verbose=2 "$TMPDIR/mac/Rongta Labels.app"
otool -L "$TMPDIR/mac/Rongta Labels.app/Contents/MacOS/rongta-gui"
"$TMPDIR/mac/Rongta Labels.app/Contents/MacOS/rongta-gui" --version
open "$TMPDIR/mac/Rongta Labels.app"
```

Expected: `valid on disk` and `satisfies its Designated Requirement`, only
`/usr/lib` and `/System` libraries, the version string, and the window. macOS
asks for Bluetooth permission on the first scan. If neither a Mac nor a
remote builder is available, CI's `macos-latest` job (Task 6) is the first
build. Record that this step wasn't done locally.

- [ ] **Step 3: Commit**

```bash
git add nix/packaging.nix
git commit -m "build: bundle and ad-hoc sign the macOS app with Nix"
```

---

### Task 6: CI generated with github-actions-nix

**Files:**
- Modify: `nix/ci.nix`
- Create: `.github/workflows/ci.yml` (generated, committed)

**Interfaces:**
- Produces:
  - `apps.write-workflows` on every system: copies the generated workflows
    into `.github/workflows/`
  - `checks.workflows-current` on `x86_64-linux`: fails if the committed YAML
    differs from the Nix source
  - workflow `ci`. Jobs: `check`; `linux` (matrix over x86_64 and aarch64);
    `windows-build`; `windows-smoke`; `macos`; and `release` on `v*` tags.

- [ ] **Step 1: Write `nix/ci.nix`**

```nix
{ ... }:
{
  perSystem =
    {
      config,
      pkgs,
      lib,
      system,
      ...
    }:
    let
      checkout = {
        name = "Check out";
        uses = "actions/checkout@v4";
      };
      installNix = {
        name = "Install Nix";
        uses = "DeterminateSystems/determinate-nix-action@v3";
      };
      # Keyed on everything that changes the build; restores the closest
      # earlier store when the exact key misses.
      cache = {
        name = "Nix store cache";
        uses = "nix-community/cache-nix-action@v6";
        with_ = {
          primary-key = "nix-\${{ runner.os }}-\${{ runner.arch }}-\${{ hashFiles('flake.lock', 'nix/**', 'flake.nix', 'rongta-rs/Cargo.lock') }}";
          restore-prefixes-first-match = "nix-\${{ runner.os }}-\${{ runner.arch }}-";
          gc-max-store-size-linux = "8G";
          gc-max-store-size-macos = "8G";
        };
      };
      setup = [
        checkout
        installNix
        cache
      ];
      upload = name: path: {
        name = "Upload ${name}";
        uses = "actions/upload-artifact@v4";
        with_ = {
          inherit name path;
          if-no-files-found = "error";
        };
      };
    in
    {
      githubActions = {
        enable = true;
        workflows.ci = {
          name = "CI";
          on = {
            push = {
              branches = [ "master" ];
              tags = [ "v*" ];
            };
            pullRequest = { };
            workflowDispatch = { };
          };
          concurrency = {
            group = "\${{ github.workflow }}-\${{ github.event.pull_request.number || github.ref }}";
            cancelInProgress = true;
          };
          permissions.contents = "read";

          jobs = {
            check = {
              name = "Tests and checks";
              runsOn = "ubuntu-24.04";
              steps = setup ++ [
                {
                  name = "nix flake check";
                  run = "nix flake check -L";
                }
              ];
            };

            linux = {
              name = "AppImage (\${{ matrix.arch }})";
              runsOn = "\${{ matrix.runner }}";
              strategy = {
                failFast = false;
                matrix.include = [
                  {
                    arch = "x86_64";
                    runner = "ubuntu-24.04";
                  }
                  {
                    arch = "aarch64";
                    runner = "ubuntu-24.04-arm";
                  }
                ];
              };
              steps = setup ++ [
                {
                  name = "Build";
                  run = ''
                    nix build -L .#rongta-gui-appimage
                    cp -L result "Rongta-Labels-''${{ matrix.arch }}.AppImage"
                  '';
                }
                {
                  # Exercises the real FUSE mount path, as a double-click would.
                  name = "Smoke test on the runner";
                  run = ''
                    sudo apt-get -qq install -y fuse3 libdbus-1-3 >/dev/null
                    "./Rongta-Labels-''${{ matrix.arch }}.AppImage" --version
                  '';
                }
                {
                  # The oldest supported glibc, today's Ubuntu LTS, and current
                  # Fedora; containers have no FUSE, so extract instead.
                  name = "Smoke test on other distros";
                  run = ''
                    for image in debian:10 ubuntu:24.04 fedora:latest; do
                      echo "::group::$image"
                      docker run --rm -e APPIMAGE_EXTRACT_AND_RUN=1 \
                        -v "$PWD/Rongta-Labels-''${{ matrix.arch }}.AppImage:/app.AppImage:ro" \
                        "docker.io/library/$image" sh -c '
                          grep -qs "^10" /etc/debian_version && echo "deb http://archive.debian.org/debian buster main" > /etc/apt/sources.list
                          if command -v apt-get >/dev/null; then
                            apt-get -qq update >/dev/null && apt-get -qq install -y libdbus-1-3 >/dev/null
                          else
                            dnf -q -y install dbus-libs >/dev/null
                          fi
                          /app.AppImage --version'
                      echo "::endgroup::"
                    done
                  '';
                }
                (upload "Rongta-Labels-\${{ matrix.arch }}.AppImage" "Rongta-Labels-\${{ matrix.arch }}.AppImage")
              ];
            };

            windows-build = {
              name = "Windows zip (cross-compiled)";
              runsOn = "ubuntu-24.04";
              steps = setup ++ [
                {
                  name = "Build";
                  run = ''
                    nix build -L .#rongta-gui-windows
                    cp -L result Rongta-Labels-windows-x86_64.zip
                  '';
                }
                (upload "Rongta-Labels-windows-x86_64.zip" "Rongta-Labels-windows-x86_64.zip")
              ];
            };

            windows-smoke = {
              name = "Windows smoke test";
              runsOn = "windows-latest";
              needs = [ "windows-build" ];
              steps = [
                {
                  name = "Download";
                  uses = "actions/download-artifact@v4";
                  with_.name = "Rongta-Labels-windows-x86_64.zip";
                }
                {
                  name = "Run --version";
                  shell = "pwsh";
                  run = ''
                    Expand-Archive Rongta-Labels-windows-x86_64.zip -DestinationPath out
                    # A GUI-subsystem exe has no console; redirect to see its output.
                    $p = Start-Process -FilePath "out/Rongta Labels/Rongta Labels.exe" -ArgumentList "--version" -Wait -NoNewWindow -PassThru -RedirectStandardOutput version.txt
                    Get-Content version.txt
                    if ($p.ExitCode -ne 0 -or -not (Select-String -Path version.txt -Pattern '^rongta-gui ')) { exit 1 }
                  '';
                }
              ];
            };

            macos = {
              name = "macOS app (arm64)";
              runsOn = "macos-latest";
              steps = setup ++ [
                {
                  name = "Build";
                  run = ''
                    nix build -L .#rongta-gui-macos
                    cp -L result Rongta-Labels-macos-arm64.zip
                  '';
                }
                {
                  name = "Verify signature and run --version";
                  run = ''
                    ditto -x -k Rongta-Labels-macos-arm64.zip out
                    codesign --verify --deep --strict --verbose=2 "out/Rongta Labels.app"
                    ! otool -L "out/Rongta Labels.app/Contents/MacOS/rongta-gui" | grep /nix/store
                    "out/Rongta Labels.app/Contents/MacOS/rongta-gui" --version
                  '';
                }
                (upload "Rongta-Labels-macos-arm64.zip" "Rongta-Labels-macos-arm64.zip")
              ];
            };

            release = {
              name = "Release";
              if_ = "startsWith(github.ref, 'refs/tags/v')";
              needs = [
                "check"
                "linux"
                "windows-smoke"
                "macos"
              ];
              runsOn = "ubuntu-24.04";
              permissions.contents = "write";
              steps = [
                {
                  name = "Download artifacts";
                  uses = "actions/download-artifact@v4";
                  with_ = {
                    path = "artifacts";
                    merge-multiple = true;
                  };
                }
                {
                  name = "Create release";
                  env.GH_TOKEN = "\${{ github.token }}";
                  run = ''gh release create "$GITHUB_REF_NAME" --repo "$GITHUB_REPOSITORY" --generate-notes artifacts/*'';
                }
              ];
            };
          };
        };
      };

      apps.write-workflows = {
        type = "app";
        program = lib.getExe (
          pkgs.writeShellApplication {
            name = "write-workflows";
            text = ''
              root=$(git rev-parse --show-toplevel)
              mkdir -p "$root/.github/workflows"
              install -m 644 ${config.githubActions.workflowsDir}/*.yml "$root/.github/workflows/"
            '';
          }
        );
      };

      checks = lib.optionalAttrs (system == "x86_64-linux") {
        workflows-current = pkgs.runCommand "workflows-current" { } ''
          if ! diff -ru ${../.github/workflows} ${config.githubActions.workflowsDir}; then
            echo "Generated workflows changed; run: nix run .#write-workflows" >&2
            exit 1
          fi
          touch $out
        '';
      };
    };
}
```

Before generating, check two details against `github-actions-nix`'s types in
`modules/types/job.nix`. First, `strategy.matrix.include` accepts a list of
attrsets. Second, a step's `with_` is free-form, so `with_.name = …` works.
Adjust the Nix to whatever the types accept; don't hand-edit the YAML.

- [ ] **Step 2: Generate and commit the YAML**

The drift check reads `.github/workflows`, so the file must exist and be
tracked by git before the first `nix flake check`:

```bash
nix run .#write-workflows
# A flake in a git repository sees only tracked files.
git add .github/workflows/ci.yml
nix run nixpkgs#actionlint -- .github/workflows/ci.yml
nix flake check -L
```

Expected: actionlint reports nothing, and the check passes, including
`workflows-current`.

- [ ] **Step 3: Push and watch the first run**

Push the branch, open a draft PR against `master`, and watch it:

```bash
git push -u origin HEAD
gh pr create --draft --fill
gh run watch
```

Expected: every job green, including the Debian 10, Ubuntu 24.04 and Fedora
container tests on both architectures. Download the four artifacts from the
run, with
`gh run download <run-id> -D "$TMPDIR/artifacts"`, and check their names
match the Global Constraints.

- [ ] **Step 4: Commit**

```bash
git add nix/ci.nix .github/workflows/ci.yml
git commit -m "ci: build, smoke-test and release GUI artifacts from Nix"
```

---

### Task 7: Docs

**Files:**
- Modify: `notes/gui.md`, `README.md`

- [ ] **Step 1: Update `notes/gui.md` "Building"**

Replace the Building section with the Nix commands, hard-wrapped at 80
columns:

- `nix build .#rongta-gui-appimage` on x86_64 or aarch64 Linux
- `nix build .#rongta-gui-windows` on x86_64 Linux (cross-compiled)
- `nix build .#rongta-gui-macos` on an Apple Silicon Mac, or through an
  aarch64-darwin remote builder
- CI builds all four on every push and attaches them to the GitHub release
  for `v*` tags
- After changing `nix/ci.nix`, run `nix run .#write-workflows` and commit the
  YAML. `nix flake check` fails until you do.

Add a "Linux requirements" paragraph from risk 2: glibc 2.28+, libdbus, FUSE
or `--appimage-extract-and-run`, and that NixOS users should use the Nix
package. Base it on what Task 6's CI run actually showed.

- [ ] **Step 2: README**

In the README's Desktop app section, add one sentence pointing to the
release page for downloads and to `notes/gui.md` for building. Refill at 80
columns.

- [ ] **Step 3: Commit**

```bash
git add notes/gui.md README.md
git commit -m "docs: describe the Nix-built GUI artifacts"
```

---

## Self-review notes

- Requirements covered: AppImages for x86_64 and aarch64 (Task 3). The binary is
  built with zig against glibc 2.28 so it uses the host's glibc and graphics
  drivers, and only nix-appimage's static runtime is reused. Windows is
  cross-compiled from Linux as a chain of derivations (Task 4: toolchain, crane
  deps, crane exe, audited zip). macOS built and signed in Nix (Task 5). CI
  declared in Nix with `github-actions-nix`, using a Mac runner for the macOS
  build (Task 6).
- The parts that can't be done in Nix are the CI smoke tests that need the real
  OS (`windows-latest`, `macos-latest`, and distro containers on both Linux
  architectures). They only run and inspect artifacts Nix built.
- Unverified details are marked where they occur: `pkgsWindows.windows.*`
  attribute names, `github-actions-nix` matrix `include` typing, crane's install
  step under `cargo zigbuild`, and the GUI's link list (risk 1).
