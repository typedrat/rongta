//! Headless image snapshots. Gated to Linux because other platforms render
//! through Metal or DX12, whose pixels differ from the lavapipe-rendered
//! reference images.

use eframe::egui;
use egui_kittest::{Harness, SnapshotResults};

use crate::{theme, theme_demo};

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
