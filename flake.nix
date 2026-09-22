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
  };

  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ inputs.devshell.flakeModule ];
      systems = [ "x86_64-linux" ];

      perSystem =
        { pkgs, system, ... }:
        let
          craneLib = inputs.crane.mkLib pkgs;
          commonArgs = {
            src = pkgs.lib.cleanSourceWith {
              src = ./rongta-rs;
              # The Rust tests embed JSON fixtures with include_str!.
              filter = path: type: craneLib.filterCargoSources path type || pkgs.lib.hasSuffix ".json" path;
            };
            strictDeps = true;
            cargoExtraArgs = "--workspace";
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.dbus ];
          };
          rongta = craneLib.buildPackage (
            commonArgs
            // {
              cargoArtifacts = craneLib.buildDepsOnly commonArgs;
              meta.mainProgram = "rongta";
            }
          );
          android = pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ "35" ];
            buildToolsVersions = [ "35.0.0" ];
            toolsVersion = null;
            includeEmulator = true;
            includeSystemImages = true;
            systemImageTypes = [ "google_apis" ];
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

          packages = {
            inherit rongta;
            default = rongta;
          };

          devshells.default = {
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
            ];

            commands = [
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
                command = ''
                  exec ${android.androidsdk}/bin/emulator \
                    -avd rongta-arm64 \
                    -packet-streamer-endpoint 127.0.0.1:8877 \
                    -no-snapshot "$@"
                '';
              }
            ];
          };

          formatter = pkgs.nixfmt-tree;
        };
    };
}
