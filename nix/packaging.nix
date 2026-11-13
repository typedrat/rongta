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
      inherit (rongtaBuild) craneLib commonArgs rongtaIcon;
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
        Icon=rongta-labels
        Categories=Office;
        Terminal=false
        StartupWMClass=rongta-labels
      '';
      appImage =
        pkgs.runCommand "Rongta-Labels-${arch}.AppImage"
          {
            nativeBuildInputs = [
              pkgs.perl
              pkgs.squashfsTools
            ];
            allowedReferences = [ ];
          }
          ''
            mkdir -p AppDir/usr/bin
            cp ${lib.getExe portable} AppDir/usr/bin/rongta-gui
            # The runtime mounts the image and executes AppRun from it.
            ln -s usr/bin/rongta-gui AppDir/AppRun
            cp ${desktopEntry} AppDir/rongta-labels.desktop
            # The runtime and appimaged read the icon named by the desktop
            # entry from the AppDir root, and file managers read .DirIcon;
            # desktops that integrate the AppImage copy the hicolor tree.
            cp ${rongtaIcon}/png/256.png AppDir/rongta-labels.png
            ln -s rongta-labels.png AppDir/.DirIcon
            for size in 16 32 48 64 128 256; do
              dir=AppDir/usr/share/icons/hicolor/''${size}x''${size}/apps
              mkdir -p $dir
              cp ${rongtaIcon}/png/$size.png $dir/rongta-labels.png
            done
            # Label text uses Atkinson Hyperlegible Next and the UI uses Inter;
            # the SIL OFL requires their licenses to ship with any copy.
            mkdir -p AppDir/usr/share/licenses/rongta-labels
            cp ${fontLicenses}/* AppDir/usr/share/licenses/rongta-labels/
            # The runtime's static libfuse runs mount(8) and umount(8) by their
            # util-linux store paths when root has a regular /etc/mtab. A run of
            # slashes the same length keeps the binary's layout and resolves to
            # the host's /bin/mount instead. Only those paths are rewritten: any
            # other store reference in the runtime must stay so that
            # allowedReferences rejects it. The runtime embeds each path twice
            # (the exec target and its error message), for mount and umount.
            perl -0777 -pe '
              s{/nix/store/[0-9a-z]{32}-util-linux-[^/\0]*(?=/bin/u?mount)}{$n++; "/" x length $&}ge;
              END { die "rewrote $n util-linux paths in the runtime, expected 4\n" unless $n == 4 }
            ' ${appImageRuntime} > runtime
            [ "$(stat -c%s runtime)" = "$(stat -c%s ${appImageRuntime})" ]
            # A type 2 AppImage is the runtime followed directly by a squashfs.
            offset=$(stat -c%s runtime)
            mksquashfs AppDir $out -offset "$offset" -comp zstd -all-root -noappend -no-xattrs -quiet
            dd if=runtime of=$out conv=notrunc status=none
            chmod 755 $out
          '';

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
        inherit (commonArgs) src;
        strictDeps = true;
        pname = "rongta-gui";
        cargoExtraArgs = "--locked -p rongta-gui";
        CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
        # These args don't inherit commonArgs' icon, and only a Windows target
        # embeds the .ico as the executable's icon resource.
        RONGTA_ICON_PNG = "${rongtaIcon}/png/256.png";
        RONGTA_ICON_ICO = "${rongtaIcon}/rongta-labels.ico";
        # Tests run natively on Linux; a Windows test binary can't run here.
        doCheck = false;
      };
      windowsExe = craneWindows.buildPackage (
        windowsArgs
        // {
          cargoArtifacts = craneWindows.buildDepsOnly windowsArgs;
          # The default strip removes only debug info; a PE's COFF symbol
          # table is another third of the file and nothing reads it in a GUI
          # build.
          stripAllList = [ "bin" ];
          # The zip's own check can't see inside deflated data, so this is
          # where a store path leaking into the binary gets caught.
          allowedReferences = [ ];
        }
      );
      # Anything outside this list must ship beside the .exe or be rejected.
      windowsSystemDlls = lib.concatStringsSep "|" [
        "advapi32"
        "bcrypt"
        "bcryptprimitives"
        "bluetoothapis"
        "cfgmgr32"
        "combase"
        "comctl32"
        "comdlg32"
        "crypt32"
        "d3d12"
        "d3dcompiler_47"
        "dwmapi"
        "dwrite"
        "dxgi"
        "gdi32"
        "hid"
        "imm32"
        "kernel32"
        "msvcrt"
        "ntdll"
        "ole32"
        "oleaut32"
        "opengl32"
        "powrprof"
        "propsys"
        "rpcrt4"
        "secur32"
        "setupapi"
        "shell32"
        "shlwapi"
        "synchronization"
        "uiautomationcore"
        "user32"
        "userenv"
        "uxtheme"
        "version"
        "winmm"
        "ws2_32"
        "api-ms-win-[a-z0-9-]+"
      ];
      windowsRuntimeDlls = with pkgsWindows.windows; [
        mcfgthreads
        pthreads
      ];
      windowsZip =
        pkgs.runCommand "Rongta-Labels-windows-x86_64.zip"
          {
            nativeBuildInputs = [
              pkgs.zip
              pkgsWindows.buildPackages.binutils
            ];
            # Deflate hides store hashes, so the meaningful check is on the exe.
            allowedReferences = [ ];
          }
          ''
            dir="Rongta Labels"
            mkdir "$dir"
            cp ${windowsExe}/bin/rongta-gui.exe "$dir/Rongta Labels.exe"
            mkdir "$dir/licenses"
            cp ${fontLicenses}/* "$dir/licenses/"
            # A failed or empty listing would pass every DLL check below
            # vacuously, so both fail the build.
            headers=$(x86_64-w64-mingw32-objdump -p "$dir/Rongta Labels.exe")
            imports=$(awk '/DLL Name:/ { print tolower($3) }' <<< "$headers")
            if [ -z "$imports" ]; then
              echo "objdump listed no DLL imports for rongta-gui.exe" >&2
              exit 1
            fi
            for dll in $imports; do
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
            # zip records each file's mtime, and the oldest date it can store
            # is 1980; pinning one keeps the archive reproducible.
            find "$dir" -exec touch -d 1980-01-01T00:00:00Z {} +
            find "$dir" -print | LC_ALL=C sort | zip -X -@ $out
          '';

      # nixpkgs links these by store path; macOS ships each under the same
      # name in /usr/lib. Joined as a shell case pattern.
      macSystemDylibs = lib.concatMapStringsSep " | " (name: "${builtins.storeDir}/*/lib/${name}") [
        "libiconv.2.dylib"
        "libresolv.9.dylib"
        "libc++.1.dylib"
        "libc++abi.dylib"
        "libz.1.dylib"
      ];
      # Nix finds store references only in uncompressed bytes, and zip's
      # deflate hides them, so allowedReferences checks the bundle here rather
      # than on the zip.
      macApp =
        pkgs.runCommand "Rongta-Labels.app"
          {
            nativeBuildInputs = [
              pkgs.cctools
              pkgs.libxml2
              pkgs.rcodesign
            ];
            # The .app must run on a Mac without Nix.
            allowedReferences = [ ];
          }
          ''
            app="Rongta Labels.app"
            contents="$app/Contents"
            bin="$contents/MacOS/rongta-gui"
            mkdir -p "$contents/MacOS" "$contents/Resources/licenses"
            cp ${rongtaBuild.rongta}/bin/rongta-gui "$bin"
            chmod u+w "$bin"
            substitute ${../rongta-rs/gui/packaging/macos/Info.plist} "$contents/Info.plist" \
              --replace-fail @VERSION@ ${rongtaBuild.rongta.version}
            # A malformed Info.plist still signs, but Finder then refuses to
            # open the app with no useful error.
            xmllint --noout "$contents/Info.plist"
            cp ${rongtaIcon}/rongta-labels.icns "$contents/Resources/"
            # The SIL OFL requires the fonts' licenses to ship with any copy.
            cp ${fontLicenses}/* "$contents/Resources/licenses/"

            # The first line of `otool -L` names the binary itself; each line
            # after it is an install name followed by its versions.
            deps=$(otool -L "$bin" | tail -n +2 | sed -E 's/^[[:space:]]*(.*) \(compatibility version .*/\1/')
            if [ -z "$deps" ]; then
              echo "otool listed no libraries for rongta-gui" >&2
              exit 1
            fi
            while IFS= read -r lib; do
              case "$lib" in
                /usr/lib/* | /System/Library/*) ;;
                ${macSystemDylibs})
                  install_name_tool -change "$lib" "/usr/lib/$(basename "$lib")" "$bin"
                  ;;
                ${builtins.storeDir}/*)
                  echo "rongta-gui links $lib, which macOS does not provide" >&2
                  exit 1
                  ;;
                *)
                  echo "rongta-gui links $lib, which is not a system library" >&2
                  exit 1
                  ;;
              esac
            done <<< "$deps"
            # `otool -l` prints each load command as a block of "key value"
            # lines. The path in an LC_RPATH block isn't at a fixed line offset
            # from its cmd line, and other blocks can mention LC_RPATH, so this
            # tracks which command each line belongs to instead of counting.
            rpaths=$(otool -l "$bin" | awk '
              $1 == "cmd" { rpath = ($2 == "LC_RPATH") }
              rpath && $1 == "path" { print $2 }
            ')
            for rpath in $rpaths; do
              case "$rpath" in
                ${builtins.storeDir}/*) install_name_tool -delete_rpath "$rpath" "$bin" ;;
              esac
            done

            # Apple Silicon refuses unsigned code, and the edits above
            # invalidated the linker's signature. Without a certificate,
            # rcodesign signs ad hoc, which also seals Info.plist and the
            # resources into the bundle's signature.
            rcodesign sign "$app"
            mv "$app" $out
          '';
      macZip =
        pkgs.runCommand "Rongta-Labels-macos-arm64.zip"
          {
            nativeBuildInputs = [ pkgs.zip ];
            allowedReferences = [ ];
          }
          ''
            app="Rongta Labels.app"
            cp -R ${macApp} "$app"
            # Store files are read-only, and Finder asks for an administrator
            # password to move a read-only app to the Trash. The signature
            # covers file contents, not modes or times.
            chmod -R u+w "$app"
            # zip records each file's mtime, and the oldest date it can store
            # is 1980; pinning one keeps the archive reproducible.
            find "$app" -exec touch -h -d 1980-01-01T00:00:00Z {} +
            find "$app" -print | LC_ALL=C sort | zip -X -y -@ $out
          '';
    in
    {
      packages =
        lib.optionalAttrs isLinux {
          rongta-gui-portable = portable;
          rongta-gui-appimage = appImage;
        }
        // lib.optionalAttrs (system == "x86_64-linux") {
          rongta-gui-windows-exe = windowsExe;
          rongta-gui-windows = windowsZip;
        }
        // lib.optionalAttrs (system == "aarch64-darwin") {
          rongta-gui-macos-app = macApp;
          rongta-gui-macos = macZip;
        };
      checks = lib.optionalAttrs isLinux {
        rongta-gui-portable = portableCheck;
      };
    };
}
