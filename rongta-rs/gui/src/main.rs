#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod errors;
mod image_tab;
mod preview;
mod printer;
mod template_tab;
mod theme;
mod worker;

use eframe::egui;
use errors::{Problem, problem_ui};
use printer::Printer;
use rongta_host::job::Stage;
use std::path::PathBuf;
use worker::{Event, Job, PrintRequest, Request, Worker};

/// The central panel's margin on each side.
const PANEL_MARGIN: f32 = 8.0;
/// The theme's horizontal item spacing, which also separates the columns.
const COLUMN_GAP: f32 = 10.0;
/// The form's widest row that can't wrap, the print card's Darkness steps,
/// needs this much.
const FORM_MIN_WIDTH: f32 = 460.0;
/// The narrowest window that still shows the whole print head at one
/// physical pixel per dot on a display at 1× scale.
const MIN_WIDTH: f32 =
    2.0 * PANEL_MARGIN + FORM_MIN_WIDTH + COLUMN_GAP + preview::CHROME + preview::HEAD_DOTS as f32;
/// Gives the form room to spare beyond its minimum.
const DEFAULT_WIDTH: f32 = MIN_WIDTH + 100.0;
/// Fits the image tab, the tallest, once its label size is known; anything
/// more scrolls.
const MIN_HEIGHT: f32 = 630.0;
/// Also fits the image tab's two-line label-size hint.
const DEFAULT_HEIGHT: f32 = 690.0;

fn main() -> eframe::Result {
    // Packaging smoke tests run this on machines without a display.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--version")
    {
        println!("rongta-gui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rongta Labels")
            .with_app_id("rongta-labels")
            .with_inner_size([DEFAULT_WIDTH, DEFAULT_HEIGHT])
            .with_min_inner_size([MIN_WIDTH, MIN_HEIGHT]),
        ..Default::default()
    };
    if let Some(icon) = window_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    let template_path = std::env::args_os().nth(1).map(PathBuf::from);
    eframe::run_native(
        "Rongta Labels",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = App::with(Worker::spawn(cc.egui_ctx.clone()), Printer::load());
            let remembered = cc
                .storage
                .and_then(|storage| eframe::get_value::<Option<PathBuf>>(storage, LAST_TEMPLATE))
                .flatten();
            app.template = template_tab::TemplateTab::new(template_copies(), remembered.clone());
            match startup(template_path, remembered) {
                Startup::Open(path) => app.template.open(path),
                Startup::Reopen(path) => app.template.reopen(path),
                Startup::Empty => {}
            }
            Ok(Box::new(app))
        }),
    )
}

/// The icon the build embedded, if it had one. A bad image only costs the
/// window its icon.
fn window_icon() -> Option<egui::IconData> {
    #[cfg(rongta_icon)]
    {
        eframe::icon_data::from_png_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/icon.png")))
            .ok()
    }
    #[cfg(not(rongta_icon))]
    {
        None
    }
}

/// The eframe storage key for the template to reopen at start. Only the path
/// is kept, never what was typed: access codes differ per label, and one left
/// filled in could print a duplicate.
const LAST_TEMPLATE: &str = "last_template";

/// Where the app keeps its own copy of each template opened, under the same
/// project folders as the CLI's settings.
fn template_copies() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "rongta").map(|dirs| dirs.data_dir().join("templates"))
}

/// Which template, if any, the window starts with.
#[derive(Debug, PartialEq, Eq)]
enum Startup {
    Open(PathBuf),
    Reopen(PathBuf),
    Empty,
}

/// A file named on the command line, which is also how "Open with" arrives,
/// is what the person asked for just now, so it beats last time's template.
fn startup(argument: Option<PathBuf>, remembered: Option<PathBuf>) -> Startup {
    match (argument, remembered) {
        (Some(path), _) => Startup::Open(path),
        (None, Some(path)) => Startup::Reopen(path),
        (None, None) => Startup::Empty,
    }
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

impl Status {
    /// "Printed" is about the label just printed, so it no longer applies once
    /// the person changes what is on screen. A failure stays until the next
    /// print, in case they still need its details.
    fn edited(&mut self) {
        if matches!(self, Status::Printed) {
            *self = Status::Ready;
        }
    }
}

/// How dark to print, in the steps the person chooses between.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Darkness {
    Lightest,
    Light,
    #[default]
    Normal,
    Dark,
    Darkest,
}

impl Darkness {
    const ALL: [Darkness; 5] = [
        Darkness::Lightest,
        Darkness::Light,
        Darkness::Normal,
        Darkness::Dark,
        Darkness::Darkest,
    ];

    fn label(self) -> &'static str {
        match self {
            Darkness::Lightest => "Lightest",
            Darkness::Light => "Light",
            Darkness::Normal => "Normal",
            Darkness::Dark => "Dark",
            Darkness::Darkest => "Darkest",
        }
    }

    /// The printer's density setting, which runs from 0 to 15.
    fn density(self) -> u8 {
        match self {
            Darkness::Lightest => 0,
            Darkness::Light => 4,
            Darkness::Normal => 8,
            Darkness::Dark => 12,
            Darkness::Darkest => 15,
        }
    }
}

/// Closing mid-print cuts the job off partway, so a close then waits for the
/// person to confirm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Closing {
    #[default]
    Open,
    /// A close came in mid-print and the modal is asking about it.
    Asking,
    /// The next close request goes through, even mid-print.
    Confirmed,
}

/// What to tell the window this frame.
#[derive(Debug, PartialEq, Eq)]
enum WindowCommand {
    None,
    CancelClose,
    Close,
}

impl Closing {
    /// `requested` is whether the window was asked to close this frame.
    /// A print that ends while the modal is up settles the question: success
    /// carries out the close the person asked for, while a failure keeps the
    /// window open so they can see what went wrong.
    fn frame(&mut self, requested: bool, status: &Status) -> WindowCommand {
        let busy = matches!(status, Status::Working(_));
        if requested {
            if busy && *self != Closing::Confirmed {
                *self = Closing::Asking;
                WindowCommand::CancelClose
            } else {
                WindowCommand::None
            }
        } else if *self == Closing::Asking && !busy {
            if matches!(status, Status::Printed) {
                *self = Closing::Confirmed;
                WindowCommand::Close
            } else {
                *self = Closing::Open;
                WindowCommand::None
            }
        } else {
            WindowCommand::None
        }
    }

    fn keep_printing(&mut self) {
        *self = Closing::Open;
    }

    fn close_anyway(&mut self) -> WindowCommand {
        *self = Closing::Confirmed;
        WindowCommand::Close
    }
}

struct App {
    tab: Tab,
    worker: Worker,
    printer: Printer,
    copies: u16,
    darkness: Darkness,
    status: Status,
    closing: Closing,
    template: template_tab::TemplateTab,
    image: image_tab::ImageTab,
}

impl App {
    fn with(worker: Worker, printer: Printer) -> Self {
        Self {
            tab: Tab::default(),
            worker,
            printer,
            copies: 1,
            darkness: Darkness::default(),
            status: Status::Ready,
            closing: Closing::default(),
            template: Default::default(),
            image: Default::default(),
        }
    }

    /// A dropped .toml opens as a template; anything else is tried as an
    /// image. Returns whether anything was dropped.
    fn handle_drops(&mut self, ctx: &egui::Context) -> bool {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in &dropped {
            let path = file.path().to_path_buf();
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            {
                self.template.open(path);
                self.tab = Tab::Template;
            } else {
                self.image.open(ctx, path);
                self.tab = Tab::Image;
            }
        }
        !dropped.is_empty()
    }

    fn busy(&self) -> bool {
        matches!(self.status, Status::Working(_))
    }

    /// Copies, darkness and the Print button for whichever tab is showing.
    /// `job` is the reason Print is unavailable while the form is incomplete.
    fn print_card(&mut self, ui: &mut egui::Ui, job: Result<Job, &'static str>) {
        let hint = print_hint(
            self.printer.selected.is_some(),
            self.printer.scanning(),
            job.as_ref().err().copied(),
        );
        let ready = hint.is_none() && !self.busy();
        let print = theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            print_controls(ui, &mut self.copies, &mut self.darkness, ready, hint)
        });
        if print && let (Some(address), Ok(job)) = (self.printer.selected, job) {
            let request = Request::Print(PrintRequest {
                address,
                copies: self.copies,
                density: self.darkness.density(),
                job,
            });
            self.status = match self.worker.send(request) {
                Ok(()) => Status::Working(Stage::Connecting),
                Err(problem) => Status::Failed(problem),
            };
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAST_TEMPLATE, &self.template.remembered());
    }

    // eframe calls this, but not `ui`, while the window is minimized, and a
    // close request that isn't cancelled here goes through. The print's
    // progress has to be current then, too, to know whether it is busy.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for event in self.worker.poll() {
            match event {
                Event::Scanned(result) => self.printer.scanned(result),
                Event::Stage(stage) => self.status = Status::Working(stage),
                Event::Printed(Ok(())) => self.status = Status::Printed,
                Event::Printed(Err(problem)) => self.status = Status::Failed(problem),
            }
        }
        if self.image.poll() {
            self.status.edited();
        }
        let requested = ctx.input(|input| input.viewport().close_requested());
        send(ctx, self.closing.frame(requested, &self.status));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.closing == Closing::Asking {
            let ctx = ui.ctx().clone();
            let command = close_modal(&ctx, &mut self.closing);
            send(&ctx, command);
        }

        let busy = self.busy();
        egui::Panel::top("printer").show(ui, |ui| {
            ui.add_space(10.0);
            self.printer.ui(ui, &self.worker, busy);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(6.0);
            status_ui(ui, &self.status);
            ui.add_space(6.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let mut edited = false;
            ui.horizontal(|ui| {
                edited |= ui
                    .selectable_value(&mut self.tab, Tab::Template, "Template")
                    .changed();
                edited |= ui
                    .selectable_value(&mut self.tab, Tab::Image, "Image")
                    .changed();
            });
            ui.add_space(8.0);
            let ctx = ui.ctx().clone();
            edited |= self.handle_drops(&ctx);
            let (mut form, mut preview) = columns(ui);
            // Problems and hints add lines to the form; scrolling keeps Print
            // reachable in a short window.
            let form_scroll = egui::ScrollArea::vertical()
                .id_salt("form")
                .auto_shrink([false, true]);
            match self.tab {
                Tab::Template => {
                    edited |= form_scroll
                        .show(&mut form, |form| {
                            let edited = self.template.form_ui(form);
                            form.add_space(8.0);
                            let job = self.template.job();
                            self.print_card(form, job);
                            edited
                        })
                        .inner;
                    self.template.preview_ui(&mut preview);
                }
                Tab::Image => {
                    let media = self.printer.media.clone();
                    edited |= form_scroll
                        .show(&mut form, |form| {
                            let edited = self.image.form_ui(form, media.as_ref());
                            form.add_space(8.0);
                            let job = self.image.job(media.as_ref());
                            self.print_card(form, job);
                            edited
                        })
                        .inner;
                    self.image.preview_ui(&mut preview, media.as_ref());
                }
            }
            if edited {
                self.status.edited();
            }
        });
    }
}

fn send(ctx: &egui::Context, command: WindowCommand) {
    match command {
        WindowCommand::None => {}
        WindowCommand::CancelClose => ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose),
        WindowCommand::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
    }
}

/// Asks whether to close mid-print. Escape or a click outside counts as
/// "Keep printing", the choice that loses nothing.
fn close_modal(ctx: &egui::Context, closing: &mut Closing) -> WindowCommand {
    let mut command = WindowCommand::None;
    // A popup's margins are sized for menus; this is a dialog.
    let frame = egui::Frame::window(&ctx.global_style());
    let modal = egui::Modal::new(egui::Id::unique("close while printing"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("A label is still printing");
            ui.label("Closing now may leave a half-printed label.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let keep = theme::primary_button(ui, "Keep printing", true);
                // Focused, Enter or Space picks the safe choice.
                if ui.memory(|memory| memory.focused().is_none()) {
                    keep.request_focus();
                }
                if keep.clicked() {
                    closing.keep_printing();
                }
                if ui.button("Close anyway").clicked() {
                    command = closing.close_anyway();
                }
            });
        });
    if *closing == Closing::Asking && modal.should_close() {
        closing.keep_printing();
    }
    command
}

/// Splits the rest of `ui` into the form and, on the right, a preview column
/// just wide enough to show the label at whole zoom. The form gets the rest.
fn columns(ui: &mut egui::Ui) -> (egui::Ui, egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    let preview_width = preview::column_width(
        rect.width() - FORM_MIN_WIDTH - COLUMN_GAP,
        ui.pixels_per_point(),
    );
    let split = rect.max.x - preview_width;
    let column = |ui: &mut egui::Ui, salt: &str, rect: egui::Rect| {
        ui.new_child(
            egui::UiBuilder::new()
                .id_salt(salt)
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        )
    };
    let form = column(
        ui,
        "form",
        egui::Rect::from_min_max(rect.min, egui::pos2(split - COLUMN_GAP, rect.max.y)),
    );
    let preview = column(
        ui,
        "preview",
        egui::Rect::from_min_max(egui::pos2(split, rect.min.y), rect.max),
    );
    (form, preview)
}

/// Why Print can't be pressed yet, in the order the person should fix things.
/// `form` is the open tab's reason, if it has one.
fn print_hint(
    has_printer: bool,
    scanning: bool,
    form: Option<&'static str>,
) -> Option<&'static str> {
    if !has_printer {
        Some("Choose a printer first")
    } else if form.is_some() {
        form
    } else if scanning {
        Some("Wait for the printer search to finish")
    } else {
        None
    }
}

/// Draws the print card's controls and reports whether Print was clicked. Kept
/// apart from the worker so it can be drawn without one.
fn print_controls(
    ui: &mut egui::Ui,
    copies: &mut u16,
    darkness: &mut Darkness,
    ready: bool,
    hint: Option<&str>,
) -> bool {
    egui::Grid::new("print settings")
        .num_columns(2)
        .spacing(egui::vec2(10.0, 10.0))
        .show(ui, |ui| {
            let label = ui.label("Copies");
            ui.add(egui::DragValue::new(copies).range(1..=100))
                .labelled_by(label.id);
            ui.end_row();
            let label = ui.label("Darkness");
            let steps = Darkness::ALL.map(|step| (step, step.label()));
            theme::segmented(ui, darkness, &steps, label.id);
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let clicked = theme::primary_button(ui, "Print", ready).clicked();
        if let Some(hint) = hint {
            ui.add(
                egui::Label::new(egui::RichText::new(hint).color(theme::palette(ui).muted)).wrap(),
            );
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
        Status::Failed(problem) => problem_ui(ui, problem),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_clears_printed_but_not_failures() {
        let mut status = Status::Printed;
        status.edited();
        assert!(matches!(status, Status::Ready));

        let mut status = Status::Working(Stage::Printing);
        status.edited();
        assert!(matches!(status, Status::Working(Stage::Printing)));

        let mut status = Status::Failed(Problem {
            summary: "x".into(),
            details: "y".into(),
        });
        status.edited();
        assert!(matches!(status, Status::Failed(_)));
    }

    #[test]
    fn a_file_argument_wins_over_the_remembered_template() {
        let argument = PathBuf::from("/downloads/badge.rongta.toml");
        let remembered = PathBuf::from("/data/templates/access-code.rongta.toml");
        assert_eq!(
            startup(Some(argument.clone()), Some(remembered.clone())),
            Startup::Open(argument.clone())
        );
        assert_eq!(
            startup(Some(argument.clone()), None),
            Startup::Open(argument)
        );
        assert_eq!(
            startup(None, Some(remembered.clone())),
            Startup::Reopen(remembered)
        );
        assert_eq!(startup(None, None), Startup::Empty);
    }

    #[test]
    fn darkness_steps_span_the_printer_density_range() {
        let steps: Vec<_> = Darkness::ALL
            .iter()
            .map(|step| (step.label(), step.density()))
            .collect();
        assert_eq!(
            steps,
            [
                ("Lightest", 0),
                ("Light", 4),
                ("Normal", 8),
                ("Dark", 12),
                ("Darkest", 15)
            ]
        );
        assert_eq!(Darkness::default().density(), 8);
    }

    #[test]
    fn closing_when_idle_needs_no_confirmation() {
        for status in [Status::Ready, Status::Printed] {
            let mut closing = Closing::default();
            assert_eq!(closing.frame(true, &status), WindowCommand::None);
            assert_eq!(closing, Closing::Open);
        }
    }

    #[test]
    fn closing_mid_print_asks_first() {
        let printing = Status::Working(Stage::Printing);
        let mut closing = Closing::default();
        assert_eq!(closing.frame(true, &printing), WindowCommand::CancelClose);
        assert_eq!(closing, Closing::Asking);
        // Asking stays up while the print runs, even if asked again.
        assert_eq!(closing.frame(false, &printing), WindowCommand::None);
        assert_eq!(closing.frame(true, &printing), WindowCommand::CancelClose);
        assert_eq!(closing, Closing::Asking);

        closing.keep_printing();
        assert_eq!(closing, Closing::Open);
        assert_eq!(closing.frame(false, &printing), WindowCommand::None);
    }

    #[test]
    fn close_anyway_is_not_asked_about_again() {
        let printing = Status::Working(Stage::Printing);
        let mut closing = Closing::default();
        closing.frame(true, &printing);
        assert_eq!(closing.close_anyway(), WindowCommand::Close);
        // The close that command causes arrives as a fresh request.
        assert_eq!(closing.frame(true, &printing), WindowCommand::None);
    }

    #[test]
    fn a_print_finishing_while_asking_closes_only_if_it_succeeded() {
        let printing = Status::Working(Stage::Printing);
        let mut closing = Closing::default();
        closing.frame(true, &printing);
        assert_eq!(closing.frame(false, &Status::Printed), WindowCommand::Close);
        assert_eq!(closing.frame(true, &Status::Printed), WindowCommand::None);

        let mut closing = Closing::default();
        closing.frame(true, &printing);
        let failed = Status::Failed(Problem {
            summary: "x".into(),
            details: "y".into(),
        });
        assert_eq!(closing.frame(false, &failed), WindowCommand::None);
        assert_eq!(closing, Closing::Open, "the window stays to show why");
    }

    #[test]
    fn print_hints_come_in_the_order_to_fix_them() {
        let form = Some("Fill in the form to print");
        assert_eq!(
            print_hint(false, true, form),
            Some("Choose a printer first")
        );
        assert_eq!(print_hint(true, true, form), form);
        assert_eq!(
            print_hint(true, true, None),
            Some("Wait for the printer search to finish")
        );
        assert_eq!(print_hint(true, false, None), None);
    }

    #[test]
    fn the_minimum_window_fits_the_head_at_one_pixel_per_dot() {
        let available = MIN_WIDTH - 2.0 * PANEL_MARGIN - FORM_MIN_WIDTH - COLUMN_GAP;
        assert_eq!(
            preview::Zoom::fit(available - preview::CHROME, 1.0, preview::HEAD_DOTS),
            Some(preview::Zoom::Whole(1))
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod snapshots;
