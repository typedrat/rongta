use std::process::Command;

fn example() -> String {
    format!(
        "{}/../template/examples/access-code.rongta.toml",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn rongta() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rongta"));
    for (key, _) in std::env::vars().filter(|(key, _)| key.starts_with("RONGTA_")) {
        command.env_remove(key);
    }
    command
}

#[test]
fn preview_saves_the_full_label_canvas() {
    let dir = std::env::temp_dir().join(format!("rongta-template-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("label.png");
    let output = rongta()
        .args(["template", &example(), "code=ab12cd", "--output"])
        .arg(&png)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let image = image::open(&png).unwrap();
    assert_eq!((image.width(), image.height()), (384, 200));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_values_fail_before_connecting() {
    for (args, needle) in [
        (vec!["code=no"], "6 to 12 letters or digits"),
        (vec![], "code: Required"),
        (vec!["colour=red"], "no field \"colour\""),
        (vec!["code"], "NAME=VALUE"),
    ] {
        let output = rongta()
            .args(["template", &example()])
            .args(args)
            .args(["--address", "00:00:00:00:00:00", "--timeout", "1"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{needle:?} not in {stderr}");
    }
}

#[test]
fn unrenderable_values_fail_before_connecting() {
    // Without a pattern, `check` accepts any value, so only rendering finds
    // that this one is too long to print.
    let source = std::fs::read_to_string(example())
        .unwrap()
        .replace("pattern = \"[A-Z0-9]{6,12}\"\n", "");
    let dir = std::env::temp_dir().join(format!("rongta-unrenderable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("open.rongta.toml");
    std::fs::write(&path, source).unwrap();
    let output = rongta()
        .arg("template")
        .arg(&path)
        .arg(format!("code={}", "W".repeat(40)))
        .args(["--address", "00:00:00:00:00:00", "--timeout", "1"])
        .output()
        .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("too long to fit"), "{stderr}");
}
