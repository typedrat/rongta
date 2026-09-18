use std::io::Write;
use std::process::{Command, Output, Stdio};

fn sdk_bytes(name: &str) -> Vec<u8> {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/sdk/framing.json")).unwrap();
    let vector = corpus["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["name"] == name)
        .unwrap();
    hex::decode(vector["wire_hex"].as_str().unwrap()).unwrap()
}

fn decode(input: &[u8], extra_args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rongta"))
        .arg("decode")
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn discovery_command_emits_original_sdk_bytes_without_status_text() {
    let output = usage::test::command!("rongta", "discovery").assert_success();
    assert_eq!(output.stdout, sdk_bytes("discovery"));
    assert!(output.stderr.is_empty());
}

#[test]
fn decoder_reports_unknown_commands_and_exact_payload_as_json_lines() {
    let output = decode(&sdk_bytes("embedded-magic"), &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let message: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        message,
        serde_json::json!({
            "mode": 1, "message_type": 1, "sub_message_type": 1,
            "superclass": 240, "subclass": 254, "payload_hex": "00a31e1cff"
        })
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn decoder_separates_concatenated_discovery_frames() {
    let output = decode(&sdk_bytes("discovery"), &[]);
    assert!(output.status.success());
    let messages: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["subclass"], 1);
    assert_eq!(messages[1]["subclass"], 6);
}

#[test]
fn decoder_exits_unsuccessfully_for_crc_failure_or_truncated_input() {
    let mut corrupt = sdk_bytes("one-byte");
    corrupt[11] ^= 1;
    for input in [corrupt, sdk_bytes("one-byte")[..12].to_vec()] {
        let output = decode(&input, &[]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn decoder_requires_explicit_override_for_oversized_sdk_vector() {
    let bytes = sdk_bytes("sequence-4096");
    assert!(!decode(&bytes, &[]).status.success());
    let output = decode(&bytes, &["--max-body-len", "4101"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let message: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(message["payload_hex"].as_str().unwrap().len(), 8192);
}

#[test]
fn info_help_exposes_connection_and_diagnostic_options() {
    let output = usage::test::command!("rongta", "info", "--help").assert_success();
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in ["--adapter", "--timeout", "--pin", "--verbose"] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
}

#[test]
fn invalid_info_arguments_fail_before_touching_bluetooth() {
    for (args, expected) in [
        (vec!["not-a-mac"], "Invalid printer Bluetooth address"),
        (
            vec!["AA:BB:CC:DD:EE:FF", "--timeout", "0"],
            "--timeout must be",
        ),
        (
            vec!["AA:BB:CC:DD:EE:FF", "--pin", "01234567890123456"],
            "--pin must contain",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rongta"))
            .arg("info")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let message = String::from_utf8(output.stderr).unwrap();
        assert!(message.contains(expected), "{message}");
    }
}

#[test]
fn image_command_emits_sdk_job_from_pnm_without_status_text() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/sdk/image.json")).unwrap();
    let fixture = corpus["raster_vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "asymmetric-bits")
        .unwrap();
    let path = std::env::temp_dir().join(format!("rongta-image-{}.pnm", std::process::id()));
    let argb = hex::decode(fixture["argb_hex"].as_str().unwrap()).unwrap();
    let mut pnm = b"P6\n16 8\n255\n".to_vec();
    for pixel in argb.as_chunks::<4>().0 {
        pnm.extend_from_slice(&pixel[1..]);
    }
    std::fs::write(&path, pnm).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rongta"))
        .arg("image")
        .arg(&path)
        .args(["--buffer-length", "4096"])
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(hex::encode(output.stdout), fixture["wire_hex"]);
}

#[test]
fn transparent_png_is_composited_on_white_for_the_image_command() {
    let path = std::env::temp_dir().join(format!("rongta-transparent-{}.png", std::process::id()));
    image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]))
        .save(&path)
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rongta"))
        .arg("image")
        .arg(&path)
        .args(["--buffer-length", "4096"])
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/sdk/image.json")).unwrap();
    let fixture = corpus["image_vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "white")
        .unwrap();
    assert_eq!(hex::encode(output.stdout), fixture["wire_hex"]);
}

#[test]
fn image_requires_explicit_buffer_length_and_rejects_missing_input() {
    for args in [
        vec!["image", "missing.png"],
        vec!["image", "missing.png", "--buffer-length", "4096"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rongta"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn print_help_exposes_job_and_transport_options() {
    let output = usage::test::command!("rongta", "print", "--help").assert_success();
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in [
        "--copies",
        "--density",
        "--speed",
        "--feed-length",
        "--threshold",
        "--adapter",
        "--pin",
        "--timeout",
        "--command-delay-ms",
        "--packet-delay-ms",
        "--verbose",
    ] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
}

#[test]
fn invalid_print_arguments_fail_before_touching_bluetooth() {
    for (args, expected) in [
        (vec!["--copies", "0"], "--copies must be"),
        (vec!["--density", "16"], "--density must be"),
        (vec!["--timeout", "0"], "--timeout must be"),
        (vec!["--pin", "01234567890123456"], "--pin must contain"),
        (vec![], "Cannot open"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rongta"))
            .args([
                "print",
                "AA:BB:CC:DD:EE:FF",
                "/nonexistent/rongta-label.png",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn scan_help_exposes_discovery_options() {
    let output = usage::test::command!("rongta", "scan", "--help").assert_success();
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in ["--adapter", "--timeout"] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
}

#[test]
fn scan_rejects_invalid_duration_before_touching_bluetooth() {
    for seconds in ["0", "3601"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rongta"))
            .args(["scan", "--timeout", seconds])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--timeout must be between"));
    }
}
