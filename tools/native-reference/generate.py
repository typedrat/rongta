#!/usr/bin/env python3
"""Build/run a tiny Android APK and capture output from the original Rongta SDK."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import zipfile
from handshake_inputs import scenarios, reply, number
from image_inputs import inputs as image_inputs

ROOT = Path(__file__).resolve().parents[2]
SOURCE = Path(__file__).resolve().parent
PACKAGE = "dev.rongta.reference"


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="ADB serial of the test emulator")
    parser.add_argument("--suite", choices=["framing", "handshake", "image", "print"], default="framing")
    parser.add_argument("--library", type=Path,
                        default=ROOT / "analysis/native/arm64-v8a/librt_printer_sdk.so")
    parser.add_argument("--output", type=Path,
                        default=ROOT / "analysis/native-reference.json")
    args = parser.parse_args()
    library = args.library.resolve()
    library_hash = hashlib.sha256(library.read_bytes()).hexdigest()
    sdk = Path(os.environ["ANDROID_HOME"])
    build_tools = sdk / "build-tools/35.0.0"
    android_jar = sdk / "platforms/android-35/android.jar"
    adb = ["adb", "-s", args.serial]
    run(*adb, "get-state")

    with tempfile.TemporaryDirectory(prefix="rongta-reference-") as temporary:
        build = Path(temporary)
        classes = build / "classes"
        dex = build / "dex"
        classes.mkdir()
        dex.mkdir()
        run("javac", "-source", "8", "-target", "8", "-Xlint:-options",
            "-classpath", android_jar, "-d", classes, *sorted((SOURCE / "src").rglob("*.java")))
        run(build_tools / "d8", "--lib", android_jar, "--min-api", "26",
            "--output", dex, *sorted(classes.rglob("*.class")))
        apk = build / "unsigned.apk"
        run(build_tools / "aapt2", "link", "-I", android_jar,
            "--manifest", SOURCE / "AndroidManifest.xml", "-o", apk)
        with zipfile.ZipFile(apk, "a", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.write(dex / "classes.dex", "classes.dex")
            archive.write(library, "lib/arm64-v8a/librt_printer_sdk.so")
            archive.writestr("assets/handshake-input.json", json.dumps(scenarios()))
            archive.writestr("assets/image-input.json", json.dumps(image_inputs()))
            archive.writestr("assets/print-input.json", json.dumps([
                dict(name=f"status-{value}", value=value, wire_hex=reply(19, number(1, value)))
                for value in [0, 128, 1, 2, 0x8080]
            ] + [dict(name="notify-printing", value=128, wire_hex=reply(18, number(1,128), 0x21))]))
        aligned = build / "reference.apk"
        run(build_tools / "zipalign", "-f", "4", apk, aligned)
        # Persist only the debug key so repeated installs use the same signer.
        key = ROOT / "analysis/native-reference.keystore"
        key.parent.mkdir(parents=True, exist_ok=True)
        if not key.exists():
            run("keytool", "-genkeypair", "-keystore", key, "-storepass", "android",
                "-keypass", "android", "-alias", "reference", "-keyalg", "RSA",
                "-validity", "3650", "-dname", "CN=Rongta reference harness", "-noprompt")
        run(build_tools / "apksigner", "sign", "--ks", key, "--ks-key-alias", "reference",
            "--ks-pass", "pass:android", "--key-pass", "pass:android", aligned)
        run(*adb, "install", "-r", aligned)
        run(*adb, "shell", "am", "force-stop", PACKAGE)
        run(*adb, "shell", "run-as", PACKAGE, "rm", "-f",
            "files/reference.json", "files/reference.pending", "files/error.txt")
        run(*adb, "shell", "am", "start", "-W", "-n", PACKAGE + "/.MainActivity",
            "--es", "suite", args.suite)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            result = subprocess.run(adb + ["exec-out", "run-as", PACKAGE, "cat", "files/reference.json"],
                                    capture_output=True)
            if result.returncode == 0:
                reference = json.loads(result.stdout)
                break
            error = subprocess.run(adb + ["exec-out", "run-as", PACKAGE, "cat", "files/error.txt"],
                                   capture_output=True)
            if error.returncode == 0:
                raise RuntimeError(error.stdout.decode())
            time.sleep(0.25)
        else:
            raise TimeoutError("SDK harness produced no output; inspect adb logcat")
        reference["library_sha256"] = library_hash
        reference["android_fingerprint"] = run(
            *adb, "shell", "getprop", "ro.build.fingerprint", capture_output=True,
            text=True).stdout.strip()
        reference["evidence"] = "Original SDK output on Android; no printer connected"
        if args.suite == "handshake":
            reference["evidence"] = "Simulated printer replies checked by original SDK; no hardware"
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(reference, indent=2) + "\n")
        print(f"Saved {args.suite} reference cases to {args.output}")


if __name__ == "__main__":
    main()
