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
      # Pulls whatever the Determinate CI job already built from FlakeHub
      # Cache, so these jobs only package and smoke-test.
      cache = {
        name = "FlakeHub Cache";
        uses = "DeterminateSystems/flakehub-cache-action@main";
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
            push.branches = [ "master" ];
            pullRequest = { };
            workflowDispatch = { };
            # The release workflow calls this one, so a tagged build runs the
            # same jobs and smoke tests as every pull request.
            workflowCall = { };
          };
          concurrency = {
            group = "\${{ github.workflow }}-\${{ github.event.pull_request.number || github.ref }}";
            cancelInProgress = true;
          };
          # FlakeHub Cache authenticates with the job's OIDC token.
          permissions = {
            id-token = "write";
            contents = "read";
          };

          jobs = {
            # Builds every package, check and dev shell on each system the
            # flake supports, which runs the test suites, pushes the results to
            # FlakeHub Cache, and publishes the flake to FlakeHub from master
            # and from tags.
            nix = {
              uses = "DeterminateSystems/ci/.github/workflows/workflow.yml@main";
              permissions = {
                id-token = "write";
                contents = "read";
              };
              with_.visibility = "public";
            };

            linux = {
              name = "AppImage (\${{ matrix.arch }})";
              runsOn = "\${{ matrix.runner }}";
              needs = [ "nix" ];
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
                    sudo apt-get -qq update >/dev/null
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
                          set -e
                          grep -qs "^10" /etc/debian_version && echo "deb http://archive.debian.org/debian buster main" > /etc/apt/sources.list
                          if command -v apt-get >/dev/null; then
                            apt-get -qq -o Acquire::Check-Valid-Until=false update >/dev/null && apt-get -qq install -y libdbus-1-3 >/dev/null
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
              needs = [ "nix" ];
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
              needs = [ "nix" ];
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
                    if otool -L "out/Rongta Labels.app/Contents/MacOS/rongta-gui" | grep -q /nix/store; then
                      echo "binary links into /nix/store" >&2
                      exit 1
                    fi
                    "out/Rongta Labels.app/Contents/MacOS/rongta-gui" --version
                  '';
                }
                (upload "Rongta-Labels-macos-arm64.zip" "Rongta-Labels-macos-arm64.zip")
              ];
            };
          };
        };

        workflows.release = {
          name = "Release";
          on.push.tags = [ "v*" ];
          permissions.contents = "read";

          jobs = {
            version = {
              name = "Check the tag matches the version";
              runsOn = "ubuntu-24.04";
              steps = [
                checkout
                {
                  name = "Compare";
                  # The binaries report the crate version, so a release must
                  # not be published under a different tag.
                  run = ''
                    version=$(python3 -c 'import tomllib; print(tomllib.load(open("rongta-rs/Cargo.toml", "rb"))["workspace"]["package"]["version"])')
                    if [ "v$version" != "$GITHUB_REF_NAME" ]; then
                      echo "Tag $GITHUB_REF_NAME doesn't match workspace version $version" >&2
                      exit 1
                    fi
                  '';
                }
              ];
            };

            build = {
              needs = [ "version" ];
              uses = "./.github/workflows/ci.yml";
              permissions = {
                id-token = "write";
                contents = "read";
              };
            };

            publish = {
              name = "Publish the release";
              needs = [ "build" ];
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
                  run = ''gh release create "$GITHUB_REF_NAME" --repo "$GITHUB_REPOSITORY" --verify-tag --generate-notes artifacts/*'';
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
            runtimeInputs = [ pkgs.git ];
            text = ''
              root=$(git rev-parse --show-toplevel)
              mkdir -p "$root/.github/workflows"
              install -m 644 ${config.githubActions.workflowsDir}/*.yml "$root/.github/workflows/"
            '';
          }
        );
      };

      # Releases are squash-merged PRs, so a tag made on a feature branch would
      # point at a commit master never contains; the bump goes straight onto
      # master instead.
      apps.bump = {
        type = "app";
        program = lib.getExe (
          pkgs.writeShellApplication {
            name = "bump";
            runtimeInputs = [
              pkgs.git
              pkgs.cargo
              pkgs.gnused
            ];
            text = ''
              if [ $# -ne 1 ] || ! [[ $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
                echo "usage: nix run .#bump -- <major.minor.patch>" >&2
                exit 1
              fi
              version=$1
              tag="v$version"

              root=$(git rev-parse --show-toplevel)
              cd "$root"
              if [ "$(git branch --show-current)" != master ]; then
                echo "Check out master first." >&2
                exit 1
              fi
              if [ -n "$(git status --porcelain)" ]; then
                echo "The working tree has uncommitted changes." >&2
                exit 1
              fi
              if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
                echo "Tag $tag already exists." >&2
                exit 1
              fi

              sed -i '/^\[workspace\.package\]$/,/^\[/ s/^version = ".*"$/version = "'"$version"'"/' rongta-rs/Cargo.toml
              if git diff --quiet rongta-rs/Cargo.toml; then
                echo "rongta-rs/Cargo.toml is already at $version." >&2
                exit 1
              fi
              cargo update --manifest-path rongta-rs/Cargo.toml --workspace --offline

              git commit -m "chore: bump version to $version" rongta-rs/Cargo.toml rongta-rs/Cargo.lock
              git tag -a "$tag" -m "$tag"
              echo "Tagged $tag. Publish it with: git push --follow-tags origin master"
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
