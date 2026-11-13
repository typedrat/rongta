//! Headless image snapshots, rendered on Mesa's lavapipe software Vulkan
//! driver. A GPU rasterizes edges and samples textures slightly differently,
//! so the harness refuses to run anywhere else rather than report pixel
//! diffs. Gated to Linux because other platforms have no lavapipe to match.

use std::{path::PathBuf, sync::Arc};

use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use egui_kittest::{Harness, SnapshotResults, kittest::Queryable as _};
use rongta::bluetooth::ScanDevice;
use rongta_host::{bluetooth::Address, job::Stage};

use crate::{
    App, Closing, DEFAULT_HEIGHT, DEFAULT_WIDTH, Darkness, MIN_HEIGHT, MIN_WIDTH, Status,
    errors::Problem, image_tab::ImageTab, print_controls, printer::Printer, status_ui,
    template_tab::TemplateTab, theme, worker::Worker,
};

/// A wgpu setup that only accepts a CPU Vulkan adapter, which in practice is
/// lavapipe.
fn lavapipe() -> egui_wgpu::WgpuSetup {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
        if let Some(adapter) = adapters
            .iter()
            .find(|adapter| adapter.get_info().device_type == wgpu::DeviceType::Cpu)
        {
            return Ok(adapter.clone());
        }
        let found: Vec<String> = adapters
            .iter()
            .map(|adapter| adapter.get_info().name)
            .collect();
        panic!(
            "GUI snapshots are recorded on lavapipe, but Vulkan offers only {found:?}. \
             Run them with `gui-snapshots` from the dev shell."
        );
    }));
    egui_wgpu::WgpuSetup::CreateNew(setup)
}

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
        .wgpu_setup(lavapipe())
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

#[test]
fn primary_button_hovered() {
    let mut results = SnapshotResults::new();
    for (theme, suffix) in THEMES {
        let mut harness = harness(egui::vec2(220.0, 90.0), theme, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let _ = theme::primary_button(ui, "Print", true);
            });
        });
        harness.get_by_label("Print").hover();
        harness.run_steps(4);
        results.add(harness.try_snapshot(format!("primary_button_hovered_{suffix}")));
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

/// Builds a snapshot case's starting state.
type Make<T> = fn() -> T;

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

/// Snapshots in both themes, building fresh state for each: a preview's
/// texture belongs to the context that uploaded it, so sharing one across
/// harnesses would draw it blank.
fn snapshot<T>(
    results: &mut SnapshotResults,
    name: &str,
    size: egui::Vec2,
    mut make: impl FnMut() -> T,
    draw: impl Fn(&mut T, &mut egui::Ui),
) {
    for (theme, suffix) in THEMES {
        let mut state = make();
        let mut harness = harness(size, theme, |u| {
            egui::CentralPanel::default().show(u, |ui| draw(&mut state, ui));
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
    snapshot(
        &mut results,
        "printer_bar",
        egui::vec2(560.0, 60.0),
        || {
            (
                Printer::for_test(devices.clone(), Some(selected), None),
                Worker::detached(),
            )
        },
        |(printer, worker), ui| printer.ui(ui, worker, false),
    );
    snapshot(
        &mut results,
        "printer_bar_problem",
        egui::vec2(560.0, 140.0),
        || {
            let error =
                anyhow::anyhow!("No BlueZ adapter; stop bt-bridge and restore host Bluetooth");
            (
                Printer::for_test(Vec::new(), None, Some(Problem::from(&error))),
                Worker::detached(),
            )
        },
        |(printer, worker), ui| printer.ui(ui, worker, false),
    );
    // The saved printer wasn't found, and nothing else was either.
    snapshot(
        &mut results,
        "printer_bar_none_found",
        egui::vec2(640.0, 90.0),
        || {
            let mut printer = Printer::for_test(Vec::new(), Some(selected), None);
            printer.scanned(Ok(Vec::new()));
            (printer, Worker::detached())
        },
        |(printer, worker), ui| printer.ui(ui, worker, false),
    );
}

#[test]
fn status_bar() {
    let mut results = SnapshotResults::new();
    let failed = || {
        Status::Failed(Problem::from(&anyhow::anyhow!(
            "Timed out after 60s discovering/pairing/connecting AA:BB:CC:DD:EE:FF"
        )))
    };
    // A failure adds its folded "Details" below the summary.
    let cases: [(&str, &dyn Fn() -> Status, f32); 4] = [
        ("status_ready", &|| Status::Ready, 76.0),
        ("status_working", &|| Status::Working(Stage::Printing), 76.0),
        ("status_printed", &|| Status::Printed, 76.0),
        ("status_failed", &failed, 110.0),
    ];
    for (name, status, height) in cases {
        snapshot(
            &mut results,
            name,
            egui::vec2(560.0, height),
            status,
            |status, ui| status_ui(ui, status),
        );
    }
}

#[test]
fn print_card() {
    let mut results = SnapshotResults::new();
    for (name, ready, hint) in [
        ("print_card_ready", true, None),
        (
            "print_card_no_printer",
            false,
            Some("Choose a printer first"),
        ),
        (
            "print_card_not_ready",
            false,
            Some("Fill in the form to print"),
        ),
    ] {
        snapshot(
            &mut results,
            name,
            egui::vec2(500.0, 200.0),
            || (2, Darkness::default()),
            |(copies, darkness), ui| {
                theme::card(ui, |ui| {
                    print_controls(ui, copies, darkness, ready, hint);
                });
            },
        );
    }
}

#[test]
fn darkness_is_one_named_choice() {
    let mut darkness = Darkness::default();
    let mut harness = harness(egui::vec2(480.0, 200.0), egui::Theme::Light, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            print_controls(ui, &mut 1, &mut darkness, true, None);
        });
    });
    harness.run();
    harness.get_by_role_and_label(egui::accesskit::Role::RadioGroup, "Darkness");
    harness.get_by_label("Darkest").click();
    harness.run();
    drop(harness);
    assert_eq!(darkness, Darkness::Darkest);
}

fn example_tab(code: Option<&str>) -> TemplateTab {
    let mut tab = TemplateTab::default();
    tab.open(PathBuf::from(concat!(
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
    let cases: [(&str, Make<TemplateTab>, egui::Vec2); 5] = [
        (
            "template_form_empty_state",
            TemplateTab::default,
            egui::vec2(400.0, 150.0),
        ),
        (
            "template_form_blank",
            || example_tab(None),
            egui::vec2(400.0, 210.0),
        ),
        (
            "template_form_invalid",
            || example_tab(Some("ab1")),
            egui::vec2(400.0, 210.0),
        ),
        (
            "template_form_valid",
            || example_tab(Some("ab12cd")),
            egui::vec2(400.0, 210.0),
        ),
        (
            "template_form_bad_file",
            || {
                let mut tab = TemplateTab::default();
                tab.open(PathBuf::from("/nonexistent/access-code.rongta.toml"));
                tab
            },
            egui::vec2(400.0, 230.0),
        ),
    ];
    for (name, make, size) in cases {
        snapshot(&mut results, name, size, make, |tab, ui| {
            tab.form_ui(ui);
        });
    }
}

#[test]
fn template_preview() {
    let mut results = SnapshotResults::new();
    snapshot(
        &mut results,
        "template_preview",
        egui::vec2(460.0, 300.0),
        || example_tab(Some("AB12CD")),
        TemplateTab::preview_ui,
    );
    snapshot(
        &mut results,
        "template_preview_empty",
        egui::vec2(460.0, 110.0),
        || example_tab(None),
        TemplateTab::preview_ui,
    );
    snapshot(
        &mut results,
        "template_preview_too_long",
        egui::vec2(460.0, 190.0),
        || {
            let mut tab = TemplateTab::default();
            tab.open(PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/name-badge.rongta.toml"
            )));
            tab.set_value("name", "Bartholomew Montgomery-Fitzwilliam");
            tab
        },
        TemplateTab::preview_ui,
    );
}

fn image_tab(media: Option<&str>) -> ImageTab {
    let mut tab = ImageTab::default();
    tab.open_now(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../examples/first-label.png"
    )));
    if let Some(media) = media {
        tab.set_media_text(media);
    }
    tab
}

#[test]
fn image_form() {
    let mut results = SnapshotResults::new();
    let gap: rongta::media::Media = "gap:50x30".parse().unwrap();
    // The cases without saved media show the label-size hint, two lines tall.
    let cases: [(&str, Make<ImageTab>, bool, f32); 6] = [
        ("image_form_empty", ImageTab::default, true, 290.0),
        ("image_form_loaded", || image_tab(None), true, 290.0),
        // The picture already open stays until the next one is decoded.
        (
            "image_form_opening",
            || {
                let mut tab = image_tab(None);
                tab.start_opening(PathBuf::from("/photos/holiday.jpg"));
                tab
            },
            true,
            320.0,
        ),
        ("image_form_no_size", || image_tab(None), false, 340.0),
        (
            "image_form_invalid",
            || image_tab(Some("gap:")),
            false,
            340.0,
        ),
        (
            "image_form_bad_file",
            || {
                let mut tab = ImageTab::default();
                tab.open_now(PathBuf::from("/nonexistent/photo.jpg"));
                tab
            },
            true,
            360.0,
        ),
    ];
    for (name, make, saved, height) in cases {
        let saved = saved.then_some(&gap);
        snapshot(
            &mut results,
            name,
            egui::vec2(460.0, height),
            make,
            |tab, ui| {
                tab.form_ui(ui, saved);
            },
        );
    }
}

#[test]
fn image_preview() {
    let mut results = SnapshotResults::new();
    let gap: rongta::media::Media = "gap:50x30".parse().unwrap();
    for (name, style) in [("image_preview", 0), ("image_preview_smooth", 1)] {
        snapshot(
            &mut results,
            name,
            egui::vec2(460.0, 340.0),
            || {
                let mut tab = image_tab(None);
                tab.set_style(style);
                tab
            },
            |tab, ui| tab.preview_ui(ui, Some(&gap)),
        );
    }
}

/// The whole window on the template tab, filled in and ready to print.
fn ready_app() -> App {
    let printer = device("AA:BB:CC:DD:EE:FF", "R22_1A2B");
    let selected = printer.address;
    let mut app = App::with(
        Worker::detached(),
        Printer::for_test(vec![printer], Some(selected), None),
    );
    app.template = example_tab(Some("AB12CD"));
    app
}

#[test]
fn whole_app() {
    let mut results = SnapshotResults::new();
    // The image form is the tallest, so it is the one to check at the minimum.
    for (theme, suffix) in THEMES {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(MIN_WIDTH, MIN_HEIGHT))
            .with_theme(theme)
            .with_context_setup(theme::install)
            .wgpu_setup(lavapipe())
            .build_eframe(|_| {
                let mut app = ready_app();
                app.tab = crate::Tab::Image;
                app.image = image_tab(Some("gap:50x30"));
                app
            });
        harness.run_steps(4);
        results.add(harness.try_snapshot(format!("app_image_minimum_{suffix}")));
    }
    // Closing was requested mid-print.
    for (theme, suffix) in THEMES {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(DEFAULT_WIDTH, DEFAULT_HEIGHT))
            .with_theme(theme)
            .with_context_setup(theme::install)
            .wgpu_setup(lavapipe())
            .build_eframe(|_| {
                let mut app = ready_app();
                app.status = Status::Working(Stage::Printing);
                app.closing = Closing::Asking;
                app
            });
        harness.run_steps(4);
        results.add(harness.try_snapshot(format!("app_closing_while_printing_{suffix}")));
    }
    for (name, size, pixels_per_point) in [
        (
            "app_default",
            egui::vec2(DEFAULT_WIDTH, DEFAULT_HEIGHT),
            1.0,
        ),
        ("app_minimum", egui::vec2(MIN_WIDTH, MIN_HEIGHT), 1.0),
        // A fractional display scale, where whole zoom has to be counted in
        // physical pixels to keep the dots even.
        (
            "app_default_scaled",
            egui::vec2(DEFAULT_WIDTH, DEFAULT_HEIGHT),
            1.25,
        ),
        // 1.5 is where a too-tight zoom tolerance would drop the preview to
        // the shrunk path, since egui rounds rects to 1/32 of a point.
        (
            "app_default_scaled_1_5",
            egui::vec2(DEFAULT_WIDTH, DEFAULT_HEIGHT),
            1.5,
        ),
    ] {
        for (theme, suffix) in THEMES {
            let mut harness = Harness::builder()
                .with_size(size)
                .with_pixels_per_point(pixels_per_point)
                .with_theme(theme)
                .with_context_setup(theme::install)
                .wgpu_setup(lavapipe())
                .build_eframe(|_| ready_app());
            harness.run_steps(4);
            results.add(harness.try_snapshot(format!("{name}_{suffix}")));
        }
    }
}
