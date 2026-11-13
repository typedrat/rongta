#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod theme;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rongta Labels")
            .with_inner_size([860.0, 620.0])
            .with_min_inner_size([700.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rongta Labels",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(App::default()))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Template,
    Image,
}

#[derive(Default)]
struct App {
    tab: Tab,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Template, "Template");
                ui.selectable_value(&mut self.tab, Tab::Image, "Image");
            });
            ui.add_space(8.0);
            theme_demo(ui);
        });
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

#[cfg(all(test, target_os = "linux"))]
mod snapshots;
