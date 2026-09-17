# Native SDK reference harness

This small Android APK loads the original ARM64 `librt_printer_sdk.so` through
its JNI exports and records generated command bytes. It declares no Bluetooth
or network permissions and sends nothing to a printer.

From the repository root, enter `nix develop` or load direnv. Start the existing
AVD (a Bluetooth bridge is unnecessary for this harness), then run:

```sh
adb devices
python3 tools/native-reference/generate.py --serial emulator-5554
```

The default input is `analysis/native/arm64-v8a/librt_printer_sdk.so`; override
it with `--library PATH`. The script builds, debug-signs, installs, and starts
`dev.rongta.reference` in the selected emulator. It does not modify the stock
RONGTA app. A repeatable debug signing key lives in the ignored `analysis/`
directory. The harness remains installed for inspection.

Output defaults to `analysis/native-reference.json` (`--output PATH` overrides
it). The output records library SHA-256, Android fingerprint, model selection,
JNI calls and their inputs, and the unmodified output bytes. No Rust encoder
participates in generation. Review regenerated output before replacing the
checked-in fixture at `rongta-rs/tests/fixtures/sdk/framing.json`.

The current corpus covers discovery (two concatenated frames), empty and
nonempty generic commands, embedded magic bytes, lengths crossing 255/256,
and larger payloads. Generic commands set the mode nibble to 1 but remain
cleartext; discovery sets it to 0. These are SDK output vectors, not captured
printer responses or evidence that every generated command is accepted.

If startup fails, the script reports Java/native-load errors saved by the APK.
For native crashes, inspect `adb -s SERIAL logcat -d`. Regeneration requires
the proprietary library locally; Cargo tests only require the checked-in JSON.

For handshake reference cases:

```sh
python3 -B tools/native-reference/generate.py --serial emulator-5554 \
  --suite handshake --output analysis/handshake-reference.json
```

`handshake_inputs.py` constructs simulated printer replies independently with
Python's `struct`, `hashlib`, and `zlib`. The APK feeds them to the original SDK,
records its callbacks, and captures `getShakeData()` output after discovery.
Two serial numbers succeed; a deliberately wrong challenge digest fails.
The returned challenge deliberately differs from the SDK's random request.
The checked-in `handshake.json` preserves these inputs and SDK observations.
These are simulated exchanges, not hardware captures; numeric model/protocol
values are fixture inputs and are not known R22 capabilities. The random
challenge in generated requests changes when regenerating the fixture.
