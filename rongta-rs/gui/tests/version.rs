#[test]
fn version_prints_without_opening_a_window() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rongta-gui"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("rongta-gui {}\n", env!("CARGO_PKG_VERSION"))
    );
}
