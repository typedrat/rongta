use std::{
    path::PathBuf,
    process::{Command, Output},
};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("rongta-settings-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rongta"));
        for (key, _) in std::env::vars().filter(|(key, _)| key.starts_with("RONGTA_")) {
            cmd.env_remove(key);
        }
        // macOS ignores the XDG variables and keeps everything under
        // ~/Library, so the home directory has to move into the sandbox too.
        cmd.env("XDG_STATE_HOME", self.0.join("state"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("HOME", self.0.join("home"));
        cmd
    }
    fn state_path(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.0
                .join("home/Library/Application Support/rongta/state.toml")
        } else {
            self.0.join("state/rongta/state.toml")
        }
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn dithered_preview_downsamples_in_linear_light_and_preserves_media_margins() {
    let sandbox = Sandbox::new("dither");
    let input = sandbox.0.join("checker.png");
    let output = sandbox.0.join("preview.png");
    // Half black, half white should still have about 50% ink after downsampling.
    image::RgbaImage::from_fn(1600, 960, |x, y| {
        let v = if (x + y) % 2 == 0 { 0 } else { 255 };
        image::Rgba([v, v, v, 255])
    })
    .save(&input)
    .unwrap();
    success(
        sandbox
            .command()
            .arg("preview")
            .arg(&input)
            .args([
                "--media",
                "gap:50x30",
                "--dither",
                "floyd-steinberg",
                "--output",
            ])
            .arg(&output)
            .output()
            .unwrap(),
    );
    let preview = image::open(&output).unwrap().into_luma8();
    assert_eq!(preview.dimensions(), (384, 240));
    assert!(preview.rows().take(8).flatten().all(|p| p.0 == [255]));
    let black = preview.pixels().filter(|p| p.0 == [0]).count();
    assert!((black as f64 / (384.0 * 224.0) - 0.5).abs() < 0.01);
}

#[test]
fn preview_and_offline_encoding_use_the_same_reference_dithered_pixels() {
    let sandbox = Sandbox::new("dither-reference");
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/dither.json")).unwrap();
    let input = sandbox.0.join("alpha.png");
    let output = sandbox.0.join("preview.png");
    image::RgbaImage::from_raw(
        19,
        9,
        fixture["rgba"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect(),
    )
    .unwrap()
    .save(&input)
    .unwrap();
    let expected = rongta::raster::Raster::new(
        24,
        9,
        hex::decode(fixture["rasters"]["floyd-steinberg"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    success(
        sandbox
            .command()
            .arg("preview")
            .arg(&input)
            .args([
                "--raw",
                "--head-width",
                "24",
                "--dither",
                "floyd-steinberg",
                "--output",
            ])
            .arg(&output)
            .output()
            .unwrap(),
    );
    assert_eq!(
        image::open(output).unwrap().into_luma8(),
        rongta::image_processing::preview(&expected)
    );
    let output = sandbox
        .command()
        .arg("image")
        .arg(&input)
        .args(["--buffer-length", "4096", "--dither", "floyd-steinberg"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected_wire: Vec<u8> = rongta::image_data::encode_image(&expected, 4096)
        .unwrap()
        .iter()
        .flat_map(|frame| frame.encode().unwrap())
        .collect();
    assert_eq!(output.stdout, expected_wire);
}

#[test]
fn invalid_dither_options_fail_before_reading_images_or_connecting() {
    let sandbox = Sandbox::new("dither-invalid");
    for (flags, message) in [
        (vec!["--dither", "invalid"], "Dither must be"),
        (
            vec!["--dither", "bayer", "--threshold", "208"],
            "--threshold only applies",
        ),
    ] {
        let output = sandbox
            .command()
            .args(["print", "missing.png", "--address", "AA:BB:CC:DD:EE:FF"])
            .args(flags)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{output:?}"
        );
    }
}

#[test]
fn settings_persist_printer_and_media_in_platform_state_directory() {
    let sandbox = Sandbox::new("persist");
    success(
        sandbox
            .command()
            .args([
                "settings",
                "--address",
                "AA:BB:CC:DD:EE:FF",
                "--model",
                "r22",
                "--media",
                "gap:50x30",
            ])
            .output()
            .unwrap(),
    );
    let state: toml::Value =
        toml::from_str(&std::fs::read_to_string(sandbox.state_path()).unwrap()).unwrap();
    assert_eq!(state["version"].as_integer(), Some(2));
    assert_eq!(
        state["printer"]["address"].as_str(),
        Some("AA:BB:CC:DD:EE:FF")
    );
    assert_eq!(state["printer"]["media"]["type"].as_str(), Some("gap"));
    assert_eq!(state["printer"]["media"]["width_mm"].as_float(), Some(50.0));
    let status: serde_json::Value = serde_json::from_str(&success(
        sandbox
            .command()
            .args(["status", "--json"])
            .output()
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(status["printer"]["address"], "AA:BB:CC:DD:EE:FF");
    assert_eq!(status["printer"]["model"], "r22");
    assert_eq!(
        status["printer"]["media"],
        serde_json::json!({"type":"gap","width_mm":50.0,"height_mm":30.0})
    );
}

#[test]
fn environment_overrides_explicit_config_and_config_overrides_saved_state() {
    let sandbox = Sandbox::new("precedence");
    success(
        sandbox
            .command()
            .args([
                "settings",
                "--address",
                "AA:BB:CC:DD:EE:FF",
                "--media",
                "gap:50x30",
            ])
            .output()
            .unwrap(),
    );
    let config = sandbox.0.join("override.toml");
    std::fs::write(&config, "[printer]\naddress = '11:22:33:44:55:66'\nmodel = 'r22'\n[printer.media]\ntype = 'circle'\ndiameter_mm = 40\n").unwrap();
    let output = success(
        sandbox
            .command()
            .arg("--config")
            .arg(&config)
            .args(["status", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["printer"]["media"],
        serde_json::json!({"type":"circle","diameter_mm":40.0})
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["printer"]["address"],
        "11:22:33:44:55:66"
    );
    let output = success(
        sandbox
            .command()
            .env("RONGTA_MEDIA", "continuous:30")
            .arg("--config")
            .arg(config)
            .args(["status", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["printer"]["media"],
        serde_json::json!({"type":"continuous","width_mm":30.0})
    );
    let output = success(
        sandbox
            .command()
            .args(["status", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["printer"]["media"],
        serde_json::json!({"type":"gap","width_mm":50.0,"height_mm":30.0})
    );
}

#[test]
fn changing_printer_clears_media_and_invalid_updates_leave_state_intact() {
    let sandbox = Sandbox::new("updates");
    success(
        sandbox
            .command()
            .args([
                "settings",
                "--address",
                "AA:BB:CC:DD:EE:FF",
                "--media",
                "gap:50x30",
            ])
            .output()
            .unwrap(),
    );
    let path = sandbox.state_path();
    let before = std::fs::read(&path).unwrap();
    let output = sandbox
        .command()
        .args(["settings", "--media", "gap:0x30"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    success(
        sandbox
            .command()
            .args(["settings", "--address", "11:22:33:44:55:66"])
            .output()
            .unwrap(),
    );
    let output = success(
        sandbox
            .command()
            .args(["status", "--json"])
            .output()
            .unwrap(),
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["printer"]["media"].is_null()
    );
}

#[test]
fn settings_menu_requires_terminal_and_explicit_missing_config_is_an_error() {
    let sandbox = Sandbox::new("errors");
    let output = sandbox.command().arg("settings").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("terminal"));
    let output = sandbox
        .command()
        .args(["--config", "/nonexistent/rongta.toml", "status"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn preview_uses_saved_media_and_explicit_override_without_changing_state() {
    let sandbox = Sandbox::new("preview");
    success(
        sandbox
            .command()
            .args(["settings", "--media", "gap:50x30"])
            .output()
            .unwrap(),
    );
    let input = sandbox.0.join("source.png");
    let output = sandbox.0.join("preview.png");
    image::RgbaImage::from_pixel(400, 240, image::Rgba([0, 0, 0, 255]))
        .save(&input)
        .unwrap();
    success(
        sandbox
            .command()
            .arg("preview")
            .arg(&input)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap(),
    );
    let preview = image::open(&output).unwrap().into_luma8();
    assert_eq!(preview.dimensions(), (384, 240));
    assert_eq!(preview.get_pixel(0, 0).0, [255]);
    assert_eq!(preview.get_pixel(0, 8).0, [0]);
    success(
        sandbox
            .command()
            .env("RONGTA_MEDIA", "gap:50x80")
            .arg("preview")
            .arg(input)
            .arg("--output")
            .arg(&output)
            .args(["--media", "gap:40x40"])
            .output()
            .unwrap(),
    );
    assert_eq!(image::open(output).unwrap().height(), 320);
    let status = success(
        sandbox
            .command()
            .args(["status", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&status).unwrap()["printer"]["media"],
        serde_json::json!({"type":"gap","width_mm":50.0,"height_mm":30.0})
    );
}

#[test]
fn config_flag_is_global_and_beats_config_environment_path() {
    let sandbox = Sandbox::new("global");
    let config = sandbox.0.join("alternate.toml");
    std::fs::write(
        &config,
        "[printer.media]\ntype = 'circle'\ndiameter_mm = 30\n",
    )
    .unwrap();
    let output = success(
        sandbox
            .command()
            .env("RONGTA_CONFIG", "/nonexistent/rongta.toml")
            .args(["status", "--json", "--config"])
            .arg(config)
            .output()
            .unwrap(),
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["printer"]["media"],
        serde_json::json!({"type":"circle","diameter_mm":30.0})
    );
}

#[test]
fn corrupt_state_and_invalid_environment_fail_without_overwriting_state() {
    let sandbox = Sandbox::new("corrupt");
    success(
        sandbox
            .command()
            .args(["settings", "--media", "gap:50x30"])
            .output()
            .unwrap(),
    );
    let path = sandbox.state_path();
    let before = std::fs::read(&path).unwrap();
    for (key, value) in [
        ("RONGTA_MEDIA", "gap:NaNx30"),
        ("RONGTA_MODEL", "r26"),
        ("RONGTA_ADDRESS", "invalid"),
    ] {
        let output = sandbox
            .command()
            .env(key, value)
            .arg("status")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    std::fs::write(&path, "broken = [").unwrap();
    let output = sandbox
        .command()
        .args(["settings", "--media", "circle:30"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "broken = [");
}

#[test]
fn address_overrides_do_not_inherit_saved_media_and_saved_address_reaches_print() {
    let sandbox = Sandbox::new("address");
    success(
        sandbox
            .command()
            .args([
                "settings",
                "--address",
                "AA:BB:CC:DD:EE:FF",
                "--media",
                "gap:50x30",
            ])
            .output()
            .unwrap(),
    );
    let output = success(
        sandbox
            .command()
            .env("RONGTA_ADDRESS", "11:22:33:44:55:66")
            .args(["status", "--json"])
            .output()
            .unwrap(),
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["printer"]["media"].is_null()
    );
    let output = sandbox
        .command()
        .args(["print", "/nonexistent/rongta.png"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Cannot open"));
    let output = sandbox
        .command()
        .env("RONGTA_ADDRESS", "11:22:33:44:55:66")
        .args(["print", "/nonexistent/rongta.png", "--address", "invalid"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid printer Bluetooth address"));
}

#[test]
fn legacy_state_loads_and_is_rewritten_as_nested_structured_state_on_save() {
    let sandbox = Sandbox::new("migration");
    let path = sandbox.state_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let legacy = "version = 1\n[selection]\naddress = 'AA:BB:CC:DD:EE:FF'\nmodel = 'r22'\nmedia = 'gap:50x30'\n";
    std::fs::write(&path, legacy).unwrap();
    let status = success(
        sandbox
            .command()
            .args(["status", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&status).unwrap()["printer"]["media"],
        serde_json::json!({"type":"gap","width_mm":50.0,"height_mm":30.0})
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), legacy);
    success(
        sandbox
            .command()
            .args(["settings", "--model", "r22"])
            .output()
            .unwrap(),
    );
    let state: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(state["version"].as_integer(), Some(2));
    assert!(state.get("selection").is_none());
    assert_eq!(
        state["printer"]["media"]["height_mm"].as_float(),
        Some(30.0)
    );
}

#[test]
fn structured_media_requires_complete_valid_variant_even_with_saved_fallback() {
    let sandbox = Sandbox::new("structured-validation");
    success(
        sandbox
            .command()
            .args(["settings", "--media", "gap:50x30"])
            .output()
            .unwrap(),
    );
    let config = sandbox.0.join("bad.toml");
    for media in [
        "type='gap'\nwidth_mm=40", // Cannot inherit the height from another media layer.
        "type='gap'\nwidth_mm=0\nheight_mm=30",
        "type='gap'\nwidth_mm=50\nheight_mm=nan",
        "type='circle'\ndiameter_mm=51",
        "type='circle'\ndiameter_mm=2",
        "type='continuous'\nwidth_mm=inf",
        "type='continuous'\nwidth_mm=30\nheight_mm=40", // Wrong variant field.
    ] {
        std::fs::write(&config, format!("[printer.media]\n{media}\n")).unwrap();
        let output = sandbox
            .command()
            .arg("--config")
            .arg(&config)
            .arg("status")
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted invalid media: {media}");
    }
}

#[test]
fn every_media_variant_round_trips_as_an_object_in_state_and_json() {
    let sandbox = Sandbox::new("variants");
    for (shorthand, expected) in [
        (
            "gap:40x30",
            serde_json::json!({"type":"gap","width_mm":40.0,"height_mm":30.0}),
        ),
        (
            "circle:30",
            serde_json::json!({"type":"circle","diameter_mm":30.0}),
        ),
        (
            "continuous:50",
            serde_json::json!({"type":"continuous","width_mm":50.0}),
        ),
    ] {
        success(
            sandbox
                .command()
                .args(["settings", "--media", shorthand])
                .output()
                .unwrap(),
        );
        let state: toml::Value =
            toml::from_str(&std::fs::read_to_string(sandbox.state_path()).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(&state["printer"]["media"]).unwrap(),
            expected
        );
        let status = success(
            sandbox
                .command()
                .args(["status", "--json"])
                .output()
                .unwrap(),
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&status).unwrap()["printer"]["media"],
            expected
        );
    }
}
