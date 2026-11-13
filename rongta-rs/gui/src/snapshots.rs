//! Headless image snapshots. Gated to Linux because other platforms render
//! through Metal or DX12, whose pixels differ from the lavapipe-rendered
//! reference images.

use eframe::egui;
use egui_kittest::{Harness, SnapshotResults};
use rongta::bluetooth::ScanDevice;
use rongta_host::{bluetooth::Address, job::Stage};

use crate::{Status, errors::Problem, print_controls, printer::Printer, status_ui, theme};

/// Builds a harness that renders `ui` offscreen with the app theme installed.
pub fn harness<'a>(
    size: egui::Vec2,
    theme: egui::Theme,
    ui: impl FnMut(&mut egui::Ui) + 'a,
) -> Harness<'a> {
    Harness::builder()
        .with_size(size)
        .with_theme(theme)
        .with_context_setup(theme::install)
        .wgpu()
        .build_ui(ui)
}

#[test]
fn theme_demo_in_both_themes() {
    let mut results = SnapshotResults::new();
    for (theme, name) in [
        (egui::Theme::Light, "theme_light"),
        (egui::Theme::Dark, "theme_dark"),
    ] {
        let mut harness = harness(egui::vec2(480.0, 280.0), theme, |ui| {
            egui::CentralPanel::default().show(ui, theme_demo);
        });
        harness.run();
        results.add(harness.try_snapshot(name));
    }
}

/// A card exercising the theme's main surfaces, text styles and button.
fn theme_demo(ui: &mut egui::Ui) {
    theme::card(ui, |ui| {
        ui.heading("Rongta Labels");
        ui.label("Theme check");
        ui.label(egui::RichText::new("Secondary text").color(theme::palette(ui).muted));
        let _ = theme::primary_button(ui, "Print", true);
    });
}

const THEMES: [(egui::Theme, &str); 2] =
    [(egui::Theme::Light, "light"), (egui::Theme::Dark, "dark")];

fn device(address: &str, name: &str) -> ScanDevice<Address> {
    ScanDevice {
        address: address.parse().unwrap(),
        name: Some(name.into()),
        rssi: None,
        class: None,
        paired: true,
        connected: false,
        spp: Some(true),
        known_before_scan: true,
    }
}

fn snapshot(
    results: &mut SnapshotResults,
    name: &str,
    size: egui::Vec2,
    mut ui: impl FnMut(&mut egui::Ui),
) {
    for (theme, suffix) in THEMES {
        let mut harness = harness(size, theme, |u| {
            egui::CentralPanel::default().show(u, &mut ui);
        });
        // A fixed step count rather than `run`, which never settles while the
        // working status animates a spinner.
        harness.run_steps(4);
        results.add(harness.try_snapshot(format!("{name}_{suffix}")));
    }
}

#[test]
fn printer_bar() {
    let mut results = SnapshotResults::new();
    let devices = vec![
        device("AA:BB:CC:DD:EE:FF", "R22_1A2B"),
        device("11:22:33:44:55:66", "Other printer"),
    ];
    let selected = devices[0].address;
    let mut chosen = Printer::for_test(devices.clone(), Some(selected), None);
    let worker = crate::worker::Worker::detached();
    snapshot(&mut results, "printer_bar", egui::vec2(560.0, 60.0), |ui| {
        chosen.ui(ui, &worker);
    });
    let problem = Problem {
        summary: "Couldn't find any printers. Check that Bluetooth is on.".into(),
        details: "adapter not found".into(),
    };
    let mut failed = Printer::for_test(Vec::new(), None, Some(problem));
    snapshot(
        &mut results,
        "printer_bar_problem",
        egui::vec2(560.0, 80.0),
        |ui| {
            failed.ui(ui, &worker);
        },
    );
}

#[test]
fn status_bar() {
    let mut results = SnapshotResults::new();
    let failed = Status::Failed(Problem {
        summary: "Couldn't reach the printer. Check that it is on and nearby.".into(),
        details: "Timed out connecting".into(),
    });
    let cases = [
        ("status_ready", Status::Ready),
        ("status_working", Status::Working(Stage::Printing)),
        ("status_printed", Status::Printed),
        ("status_failed", failed),
    ];
    for (name, status) in cases {
        snapshot(&mut results, name, egui::vec2(480.0, 76.0), |ui| {
            status_ui(ui, &status);
        });
    }
}

#[test]
fn print_card() {
    let mut results = SnapshotResults::new();
    for (name, ready, has_printer) in [
        ("print_card_ready", true, true),
        ("print_card_no_printer", false, false),
    ] {
        let (mut copies, mut density) = (2, 8);
        snapshot(&mut results, name, egui::vec2(480.0, 160.0), |ui| {
            theme::card(ui, |ui| {
                print_controls(ui, &mut copies, &mut density, ready, has_printer);
            });
        });
    }
}

fn example_tab(code: Option<&str>) -> crate::template_tab::TemplateTab {
    let mut tab = crate::template_tab::TemplateTab::default();
    tab.open(std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../template/examples/access-code.rongta.toml"
    )));
    if let Some(code) = code {
        tab.set_value("code", code);
    }
    tab
}

#[test]
fn template_form() {
    let mut results = SnapshotResults::new();
    let cases = [
        ("template_form_empty_state", None, egui::vec2(400.0, 140.0)),
        ("template_form_blank", Some(""), egui::vec2(400.0, 200.0)),
        (
            "template_form_invalid",
            Some("ab1"),
            egui::vec2(400.0, 200.0),
        ),
        (
            "template_form_valid",
            Some("ab12cd"),
            egui::vec2(400.0, 200.0),
        ),
    ];
    for (name, code, size) in cases {
        let mut tab = match code {
            None => crate::template_tab::TemplateTab::default(),
            Some("") => example_tab(None),
            Some(code) => example_tab(Some(code)),
        };
        snapshot(&mut results, name, size, |ui| tab.form_ui(ui));
    }
}

/// Each theme gets its own tab: a preview's texture belongs to the context
/// that uploaded it, so sharing one across harnesses would draw it blank.
#[test]
fn template_preview() {
    let mut results = SnapshotResults::new();
    for (name, code, size) in [
        ("template_preview", Some("AB12CD"), egui::vec2(460.0, 340.0)),
        ("template_preview_empty", None, egui::vec2(460.0, 110.0)),
    ] {
        for (theme, suffix) in THEMES {
            let mut tab = example_tab(code);
            let mut harness = harness(size, theme, |u| {
                egui::CentralPanel::default().show(u, |ui| tab.preview_ui(ui));
            });
            harness.run_steps(4);
            results.add(harness.try_snapshot(format!("{name}_{suffix}")));
        }
    }
}
