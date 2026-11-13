#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod errors;
mod image_tab;
mod preview;
mod printer;
mod template_tab;
mod theme;
mod worker;

use eframe::egui;
use errors::Problem;
use printer::Printer;
use rongta_host::job::Stage;
use std::path::PathBuf;
use worker::{Event, Job, PrintRequest, Request, Worker};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rongta Labels")
            .with_inner_size([860.0, 620.0])
            .with_min_inner_size([700.0, 540.0]),
        ..Default::default()
    };
    let template_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    eframe::run_native(
        "Rongta Labels",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(App::new(cc.egui_ctx.clone(), template_path)))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Template,
    Image,
}

enum Status {
    Ready,
    Working(Stage),
    Printed,
    Failed(Problem),
}

struct App {
    tab: Tab,
    worker: Worker,
    printer: Printer,
    copies: u16,
    density: u8,
    status: Status,
    template: template_tab::TemplateTab,
    image: image_tab::ImageTab,
}

impl App {
    fn new(ctx: egui::Context, template_path: Option<PathBuf>) -> Self {
        let mut template = template_tab::TemplateTab::default();
        if let Some(path) = template_path {
            template.open(path);
        }
        Self {
            tab: Tab::default(),
            worker: Worker::spawn(ctx),
            printer: Printer::load(),
            copies: 1,
            density: 8,
            status: Status::Ready,
            template,
            image: Default::default(),
        }
    }

    /// A dropped .toml opens as a template; anything else is tried as an image.
    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            let path = file.path().to_path_buf();
            if path.extension().is_some_and(|ext| ext == "toml") {
                self.template.open(path);
                self.tab = Tab::Template;
            } else {
                self.image.open(path);
                self.tab = Tab::Image;
            }
        }
    }

    fn busy(&self) -> bool {
        matches!(self.status, Status::Working(_))
    }

    /// Copies, darkness and the Print button for whichever tab is showing.
    /// `job` is `None` while the form is incomplete.
    fn print_card(&mut self, ui: &mut egui::Ui, job: Option<Job>) {
        let ready = job.is_some() && self.printer.selected.is_some() && !self.busy();
        let has_printer = self.printer.selected.is_some();
        let print = theme::card(ui, |ui| {
            print_controls(ui, &mut self.copies, &mut self.density, ready, has_printer)
        });
        if print && let (Some(address), Some(job)) = (self.printer.selected, job) {
            self.worker.send(Request::Print(PrintRequest {
                address,
                copies: self.copies,
                density: self.density,
                job,
            }));
            self.status = Status::Working(Stage::Connecting);
        }
    }
}

/// Draws the print card's controls and reports whether Print was clicked. Kept
/// apart from the worker so it can be drawn without one.
fn print_controls(
    ui: &mut egui::Ui,
    copies: &mut u16,
    density: &mut u8,
    ready: bool,
    has_printer: bool,
) -> bool {
    ui.horizontal(|ui| {
        let label = ui.label("Copies");
        ui.add(egui::DragValue::new(copies).range(1..=100))
            .labelled_by(label.id);
        ui.add_space(16.0);
        ui.add(egui::Slider::new(density, 0..=15).text("Darkness"));
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let clicked = theme::primary_button(ui, "Print", ready).clicked();
        if !has_printer {
            ui.label(egui::RichText::new("Choose a printer first").color(theme::palette(ui).muted));
        }
        clicked
    })
    .inner
}

fn status_ui(ui: &mut egui::Ui, status: &Status) {
    let p = theme::palette(ui);
    ui.horizontal(|ui| match status {
        Status::Ready => {
            ui.label(egui::RichText::new("Ready").color(p.muted));
        }
        Status::Working(stage) => {
            ui.spinner();
            ui.label(match stage {
                Stage::Connecting => "Connecting to the printer…",
                Stage::Handshaking => "Talking to the printer…",
                Stage::Printing => "Printing…",
            });
        }
        Status::Printed => {
            ui.label(egui::RichText::new("✓ Printed.").color(p.success));
        }
        Status::Failed(problem) => {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&problem.summary).color(p.error));
                ui.collapsing("Details", |ui| {
                    ui.label(egui::RichText::new(&problem.details).monospace());
                });
            });
        }
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        for event in self.worker.poll() {
            match event {
                Event::Scanned(result) => self.printer.scanned(result),
                Event::Stage(stage) => self.status = Status::Working(stage),
                Event::Printed(Ok(())) => self.status = Status::Printed,
                Event::Printed(Err(problem)) => self.status = Status::Failed(problem),
            }
        }

        egui::Panel::top("printer").show(ui, |ui| {
            ui.add_space(10.0);
            self.printer.ui(ui, &self.worker);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(6.0);
            status_ui(ui, &self.status);
            ui.add_space(6.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Template, "Template");
                ui.selectable_value(&mut self.tab, Tab::Image, "Image");
            });
            ui.add_space(8.0);
            let ctx = ui.ctx().clone();
            self.handle_drops(&ctx);
            ui.columns(2, |columns| match self.tab {
                Tab::Template => {
                    self.template.form_ui(&mut columns[0]);
                    columns[0].add_space(8.0);
                    let job = self.template.job();
                    self.print_card(&mut columns[0], job);
                    self.template.preview_ui(&mut columns[1]);
                }
                Tab::Image => {
                    let media = self.printer.media.clone();
                    self.image.form_ui(&mut columns[0], media.as_ref());
                    columns[0].add_space(8.0);
                    let job = self.image.job(media.as_ref());
                    self.print_card(&mut columns[0], job);
                    self.image.preview_ui(&mut columns[1], media.as_ref());
                }
            });
        });
    }
}

#[cfg(all(test, target_os = "linux"))]
mod snapshots;
