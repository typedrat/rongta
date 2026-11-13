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
