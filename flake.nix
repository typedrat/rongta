{
  description = "Rongta printer tools and analysis with Android Emulator and Bumble";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1";
    crane.url = "github:ipetkov/crane";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    devshell = {
      url = "github:numtide/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
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
  };

  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.devshell.flakeModule
        inputs.github-actions-nix.flakeModules.default
        ./nix/packaging.nix
        ./nix/ci.nix
      ];
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      perSystem =
        { pkgs, system, ... }:
        let
          craneLib = inputs.crane.mkLib pkgs;
          rongtaIcon = pkgs.callPackage ./nix/icon.nix { };
          commonArgs = {
            src = pkgs.lib.cleanSourceWith {
              src = ./rongta-rs;
              # Tests embed JSON fixtures; the template crate embeds fonts and an
              # example; the GUI's icon source sits beside its crate.
              filter =
                path: type:
                craneLib.filterCargoSources path type
                || pkgs.lib.any (suffix: pkgs.lib.hasSuffix suffix path) [
                  ".json"
                  ".ttf"
                  ".toml"
                  ".png"
                  ".svg"
                ];
            };
            strictDeps = true;
            cargoExtraArgs = "--workspace";
            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.pkg-config ];
            # BlueZ is reached over D-Bus; macOS links its system frameworks instead.
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.dbus ];
            # The GUI's build script embeds this as the window icon.
            RONGTA_ICON_PNG = "${rongtaIcon}/png/256.png";
          };
          # winit and wgpu dlopen these at run time rather than linking them.
          guiLibraries = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            with pkgs;
            [
              libGL
              libxkbcommon
              vulkan-loader
              wayland
              libx11
              libxcursor
              libxi
              libxrandr
            ]
          );
          # Mesa's software Vulkan driver, named by architecture.
          lavapipeIcd = "${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.${pkgs.stdenv.hostPlatform.uname.processor}.json";
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          rongta = craneLib.buildPackage (
            commonArgs
            // {
              # The GUI snapshot tests only run on lavapipe, the renderer their
              # reference images come from; VK_DRIVER_FILES makes it the only
              # driver the Vulkan loader sees.
              preCheck = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                export VK_DRIVER_FILES=${lavapipeIcd}
                export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [ pkgs.vulkan-loader ]}
              '';
              nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.makeWrapper ];
              postInstall = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                wrapProgram $out/bin/rongta-gui \
                  --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath guiLibraries}
              '';
              inherit cargoArtifacts;
              meta.mainProgram = "rongta";
            }
          );
          # Google publishes the Android Emulator for x86_64 Linux hosts only, so
          # elsewhere a system image would have nothing to run it.
          # TODO(#1): aarch64 hosts have no Android VM; Cuttlefish may replace it.
          hasEmulator = pkgs.stdenv.hostPlatform.isx86_64;
          android = pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ "35" ];
            buildToolsVersions = [ "35.0.0" ];
            toolsVersion = null;
            includeEmulator = hasEmulator;
            includeSystemImages = hasEmulator;
            systemImageTypes = [ "google_apis" ];
            # The stock app's ARM64 libraries run through this image's translation.
            abiVersions = [ "x86_64" ];
          };
          sdkRoot = "${android.androidsdk}/libexec/android-sdk";
        in
        {
          _module.args.pkgs = import inputs.nixpkgs {
            inherit system;
            config = {
              allowUnfree = true;
              android_sdk.accept_license = true;
            };
          };

          # Shared with the nix/*.nix modules so packaging reuses one dependency build.
          _module.args.rongtaBuild = {
            inherit
              craneLib
              commonArgs
              cargoArtifacts
              rongta
              rongtaIcon
              ;
          };

          packages = {
            inherit rongta;
            rongta-icon = rongtaIcon;
            default = rongta;
          };

          # Android emulation and BlueZ capture tooling is Linux-only; other hosts
          # get just the Rust toolchain.
          devshells.default =
            if pkgs.stdenv.hostPlatform.isLinux then
              {
                packages = [
                  android.androidsdk
                  pkgs.python3Packages.bumble
                  pkgs.bluez
                  pkgs.wireshark
                  pkgs.cargo
                  pkgs.rustc
                  pkgs.rustfmt
                  pkgs.clippy
                  pkgs.rust-analyzer
                  pkgs.stdenv.cc
                  pkgs.pkg-config
                  pkgs.dbus
                  pkgs.jdk
                ];

                env = [
                  {
                    name = "PKG_CONFIG_PATH";
                    prefix = "${pkgs.dbus.dev}/lib/pkgconfig";
                  }
                  {
                    name = "RUST_SRC_PATH";
                    value = "${pkgs.rustPlatform.rustLibSrc}";
                  }
                  {
                    name = "ANDROID_HOME";
                    value = sdkRoot;
                  }
                  {
                    name = "ANDROID_SDK_ROOT";
                    value = sdkRoot;
                  }
                  {
                    name = "LD_LIBRARY_PATH";
                    prefix = pkgs.lib.makeLibraryPath guiLibraries;
                  }
                  {
                    name = "RONGTA_ICON_PNG";
                    value = commonArgs.RONGTA_ICON_PNG;
                  }
                ];

                # Offers lavapipe, which the GUI snapshot tests insist on,
                # alongside the real GPU, which `cargo run` still prefers. The
                # loader ignores VK_ADD_DRIVER_FILES whenever VK_DRIVER_FILES
                # is set, as NixOS desktops with NVIDIA drivers do, so then
                # lavapipe has to join that list instead.
                devshell.startup.lavapipe.text = ''
                  if [[ -n "''${VK_DRIVER_FILES:-}" ]]; then
                    export VK_DRIVER_FILES="$VK_DRIVER_FILES:${lavapipeIcd}"
                  else
                    export VK_ADD_DRIVER_FILES=${lavapipeIcd}
                  fi
                '';

                commands = [
                  {
                    name = "gui-snapshots";
                    category = "gui";
                    help = "Run the GUI tests on lavapipe alone; UPDATE_SNAPSHOTS=force re-records the images";
                    command = ''
                      cd "$(${pkgs.git}/bin/git rev-parse --show-toplevel)/rongta-rs"
                      VK_DRIVER_FILES=${lavapipeIcd} exec cargo test --locked -p rongta-gui "$@"
                    '';
                  }
                  {
                    name = "bt-bridge";
                    category = "android";
                    help = "Bridge hci0 to Android on port 8877 and capture traffic (requires hci0 down)";
                    command = ''
                      set -euo pipefail

                      if ! adapter_state="$(LC_ALL=C ${pkgs.bluez}/bin/hciconfig hci0)"; then
                        printf '%s\n' 'Cannot read hci0 state; Bluetooth bridge was not started.' >&2
                        exit 1
                      fi

                      if ${pkgs.gnugrep}/bin/grep -Eq '^[[:space:]]*UP([[:space:]]|$)' <<< "$adapter_state"; then
                        printf '%s\n' \
                          'hci0 is UP. Run bt-prepare as root, then retry bt-bridge.' \
                          'bt-prepare stops host Bluetooth and disconnects its peripherals.' >&2
                        exit 1
                      fi

                      if ! ${pkgs.gnugrep}/bin/grep -Eq '^[[:space:]]*DOWN([[:space:]]|$)' <<< "$adapter_state"; then
                        printf '%s\n' 'Cannot determine whether hci0 is DOWN; Bluetooth bridge was not started.' >&2
                        exit 1
                      fi

                      capture_dir="$(${pkgs.coreutils}/bin/mktemp -d /tmp/rongta-capture.XXXXXXXX)"
                      (
                        umask 077
                        : > "$capture_dir/rongta-0.btsnoop"
                        : > "$capture_dir/rongta-1.btsnoop"
                      )
                      if (( EUID == 0 )) && [[ -n "''${SUDO_UID:-}" && -n "''${SUDO_GID:-}" ]]; then
                        ${pkgs.coreutils}/bin/chown "''${SUDO_UID}:''${SUDO_GID}" \
                          "$capture_dir" "$capture_dir/"*.btsnoop
                      fi
                      printf 'Captures: %s\n' "$capture_dir" >&2

                      exec env \
                        BUMBLE_SNOOPER="btsnoop:file:$capture_dir/rongta-{instance}.btsnoop" \
                        ${pkgs.python3Packages.bumble}/bin/bumble-hci-bridge \
                        android-netsim:127.0.0.1:8877,mode=controller \
                        hci-socket:0 "$@"
                    '';
                  }
                  {
                    name = "bt-prepare";
                    category = "android";
                    help = "Stop host Bluetooth and take hci0 down (run as root)";
                    command = ''
                      set -euo pipefail

                      if (( EUID != 0 )); then
                        printf '%s\n' 'Run bt-prepare as root using your preferred privilege tool.' >&2
                        exit 1
                      fi

                      ${pkgs.systemd}/bin/systemctl stop bluetooth.service
                      ${pkgs.bluez}/bin/hciconfig hci0 down
                    '';
                  }
                  {
                    name = "emu";
                    category = "android";
                    help = "Launch the rongta-arm64 Pixel 5 AVD connected to the Bluetooth bridge";
                    command =
                      if hasEmulator then
                        ''
                          exec ${android.androidsdk}/bin/emulator \
                            -avd rongta-arm64 \
                            -packet-streamer-endpoint 127.0.0.1:8877 \
                            -no-snapshot "$@"
                        ''
                      else
                        ''
                          printf '%s\n' 'The Android Emulator is only available on x86_64 Linux hosts.' >&2
                          exit 1
                        '';
                  }
                ];
              }
            else
              {
                packages = [
                  pkgs.cargo
                  pkgs.rustc
                  pkgs.rustfmt
                  pkgs.clippy
                  pkgs.rust-analyzer
                ];
                env = [
                  {
                    name = "RUST_SRC_PATH";
                    value = "${pkgs.rustPlatform.rustLibSrc}";
                  }
                  {
                    name = "RONGTA_ICON_PNG";
                    value = commonArgs.RONGTA_ICON_PNG;
                  }
                ];
              };

          formatter = pkgs.nixfmt-tree;
        };
    };
}
