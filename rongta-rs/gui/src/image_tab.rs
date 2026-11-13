//! Printing an ordinary picture: choose it, choose how it fills the label and
//! how greys become dots, then preview.
use crate::{
    errors::{Problem, problem_ui},
    preview::{HEAD_DOTS, Preview},
    theme,
    worker::Job,
};
use eframe::egui;
use image::{DynamicImage, ImageError, ImageReader, RgbaImage, imageops::FilterType};
use rongta::{
    image_processing::Layout,
    media::Media,
    monochrome::{Conversion, Dither},
};
use std::{
    io::{BufRead, Seek},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
};

/// (label, dither name); `None` is the SDK's sharp black-and-white threshold.
const STYLES: [(&str, Option<&str>); 4] = [
    ("Sharp — text and logos", None),
    ("Photo — smooth", Some("floyd-steinberg")),
    ("Photo — high contrast", Some("atkinson")),
    ("Pattern", Some("bayer")),
];

/// The longest edge kept after loading. A label is at most 384 dots across,
/// so this still leaves room to crop for "Fill the label", and it keeps
/// re-rendering the preview on each edit cheap.
const MAX_EDGE: u32 = 2048;

/// Enough for a 24-megapixel phone photo; past this a file is more likely
/// hostile than a picture someone wants on a label.
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DECODE_SIDE: u32 = 16_384;

const UNREADABLE: &str = "Couldn't open this picture.";
const TOO_LARGE: &str = "This picture is too large to open.";
const OPENING: &str = "Opening the picture…";

/// A decoded picture, or why it couldn't be, tagged with the open it answers.
struct Loaded {
    generation: u64,
    path: PathBuf,
    result: Result<RgbaImage, Problem>,
}

/// Which open is current. Every open bumps the generation, so a slow decode
/// that finishes after a newer open is recognised as stale and dropped.
#[derive(Default)]
struct Opening {
    generation: u64,
    /// The file being opened, until its result arrives.
    pending: Option<PathBuf>,
}

impl Opening {
    /// Starts an open that supersedes any still running, returning its
    /// generation.
    fn begin(&mut self, path: PathBuf) -> u64 {
        self.generation += 1;
        self.pending = Some(path);
        self.generation
    }

    /// The latest open's result, or `None` if `loaded` was superseded.
    fn finish(&mut self, loaded: Loaded) -> Option<(PathBuf, Result<RgbaImage, Problem>)> {
        if loaded.generation != self.generation || self.pending.is_none() {
            return None;
        }
        self.pending = None;
        Some((loaded.path, loaded.result))
    }
}

/// Decodes run on their own short-lived threads and report back here.
struct Loader {
    opening: Opening,
    sender: Sender<Loaded>,
    receiver: Receiver<Loaded>,
}

impl Default for Loader {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            opening: Opening::default(),
            sender,
            receiver,
        }
    }
}

#[derive(Default)]
pub struct ImageTab {
    loader: Loader,
    path: Option<PathBuf>,
    source: Option<Arc<RgbaImage>>,
    /// Empty means the printer's saved media.
    media_text: String,
    fit: bool,
    style: usize,
    problem: Option<Problem>,
    /// Keyed by media, fit and style; opening a picture clears it.
    preview: Preview<(Media, bool, usize)>,
}

fn load(path: &Path) -> Result<RgbaImage, Problem> {
    let reader = ImageReader::open(path).map_err(|error| {
        Problem::new(
            UNREADABLE,
            &anyhow::Error::new(error).context(format!("Cannot open {}", path.display())),
        )
    })?;
    decode(reader)
}

/// Decodes within memory limits, then shrinks to `MAX_EDGE` before the
/// RGBA conversion so that copy is small too.
fn decode(reader: ImageReader<impl BufRead + Seek>) -> Result<RgbaImage, Problem> {
    let mut reader = reader
        .with_guessed_format()
        .map_err(|error| Problem::new(UNREADABLE, &error.into()))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    limits.max_image_width = Some(MAX_DECODE_SIDE);
    limits.max_image_height = Some(MAX_DECODE_SIDE);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| {
        let summary = match error {
            ImageError::Limits(_) => TOO_LARGE,
            _ => UNREADABLE,
        };
        Problem::new(summary, &error.into())
    })?;
    Ok(shrink(image).into_rgba8())
}

fn shrink(image: DynamicImage) -> DynamicImage {
    if image.width().max(image.height()) > MAX_EDGE {
        // `resize` keeps the aspect ratio within the bound.
        image.resize(MAX_EDGE, MAX_EDGE, FilterType::Lanczos3)
    } else {
        image
    }
}

/// `load`, with a panicking decoder reported as an unreadable picture rather
/// than leaving the open pending forever.
fn load_guarded(path: &Path) -> Result<RgbaImage, Problem> {
    std::panic::catch_unwind(|| load(path)).unwrap_or_else(|_| {
        Err(Problem::new(
            UNREADABLE,
            &anyhow::anyhow!("The decoder crashed reading {}", path.display()),
        ))
    })
}

impl ImageTab {
    /// Starts decoding `path` on a background thread; `poll` picks up the
    /// result. The current picture stays until then.
    pub fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        let generation = self.begin(path.clone());
        let sender = self.loader.sender.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("open-picture".into())
            .spawn(move || {
                let result = load_guarded(&path);
                // The tab is gone if the receiver is, and then so is the
                // window to repaint.
                if sender
                    .send(Loaded {
                        generation,
                        path,
                        result,
                    })
                    .is_ok()
                {
                    ctx.request_repaint();
                }
            });
        if let Err(error) = spawned {
            let path = self.loader.opening.pending.clone().unwrap_or_default();
            self.finish(Loaded {
                generation,
                path,
                result: Err(Problem::new(UNREADABLE, &error.into())),
            });
        }
    }

    /// Decodes on the calling thread, for tests that need the picture now.
    #[cfg(test)]
    pub fn open_now(&mut self, path: PathBuf) {
        let generation = self.begin(path.clone());
        let result = load_guarded(&path);
        self.finish(Loaded {
            generation,
            path,
            result,
        });
    }

    /// Shows `path` as opening without decoding it.
    #[cfg(all(test, target_os = "linux"))]
    pub fn start_opening(&mut self, path: PathBuf) {
        self.begin(path);
    }

    fn begin(&mut self, path: PathBuf) -> u64 {
        self.problem = None;
        self.loader.opening.begin(path)
    }

    /// Applies any finished open. Returns whether the picture changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(loaded) = self.loader.receiver.try_recv() {
            changed |= self.finish(loaded);
        }
        changed
    }

    /// Returns whether the picture changed.
    fn finish(&mut self, loaded: Loaded) -> bool {
        match self.loader.opening.finish(loaded) {
            Some((path, Ok(image))) => {
                self.source = Some(Arc::new(image));
                self.path = Some(path);
                self.problem = None;
                self.preview.clear();
                true
            }
            Some((_, Err(problem))) => {
                self.problem = Some(problem);
                false
            }
            None => false,
        }
    }

    fn media(&self, saved: Option<&Media>) -> Option<Media> {
        if self.media_text.trim().is_empty() {
            saved.cloned()
        } else {
            self.media_text.trim().parse().ok()
        }
    }

    fn conversion(&self) -> Conversion {
        match STYLES[self.style].1 {
            None => Conversion::Threshold(208),
            Some(name) => Conversion::Dither(name.parse::<Dither>().expect("listed dithers parse")),
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    pub fn set_media_text(&mut self, text: &str) {
        self.media_text = text.into();
    }

    #[cfg(all(test, target_os = "linux"))]
    pub fn set_style(&mut self, style: usize) {
        self.style = style;
    }

    /// The print job, or a short reason Print is unavailable.
    pub fn job(&self, saved_media: Option<&Media>) -> Result<Job, &'static str> {
        if self.loader.opening.pending.is_some() {
            return Err(OPENING);
        }
        self.shown_job(saved_media)
    }

    /// The job for the picture on screen, which may be about to be replaced.
    fn shown_job(&self, saved_media: Option<&Media>) -> Result<Job, &'static str> {
        Ok(Job::Image {
            source: self.source.clone().ok_or("Choose a picture to print")?,
            media: self
                .media(saved_media)
                .ok_or("Enter the label size to print")?,
            layout: if self.fit { Layout::Fit } else { Layout::Label },
            conversion: self.conversion(),
        })
    }

    /// Returns whether the person changed anything.
    pub fn form_ui(&mut self, ui: &mut egui::Ui, saved_media: Option<&Media>) -> bool {
        let p = theme::palette(ui);
        let mut edited = false;
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                let title = self
                    .path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "No picture chosen".into());
                ui.heading(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Choose…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter(
                                "Picture",
                                &["png", "jpg", "jpeg", "pnm", "pbm", "pgm", "ppm"],
                            )
                            .pick_file()
                    {
                        self.open(ui.ctx(), path);
                        edited = true;
                    }
                });
            });
            if let Some(name) = self
                .loader
                .opening
                .pending
                .as_ref()
                .and_then(|path| path.file_name())
            {
                ui.horizontal(|ui| {
                    // The default spinner is a whole control tall, which
                    // dwarfs a one-line note.
                    ui.add(egui::Spinner::new().size(16.0));
                    ui.label(
                        egui::RichText::new(format!("Opening {}…", name.to_string_lossy()))
                            .color(p.muted),
                    );
                });
            }
            if let Some(problem) = &self.problem {
                problem_ui(ui, problem);
            }

            let labels = ui.label(
                egui::RichText::new("Labels").font(egui::FontId::new(14.0, theme::semibold())),
            );
            let placeholder = saved_media
                .map(|media| format!("{media} (saved)"))
                .unwrap_or_else(|| "e.g. gap:50x30".into());
            edited |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.media_text)
                        .hint_text(placeholder)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(labels.id)
                .changed();
            if self.media(saved_media).is_none() {
                // Blank isn't a mistake yet, only something still to fill in.
                let colour = if self.media_text.trim().is_empty() {
                    p.muted
                } else {
                    p.error
                };
                ui.label(
                    egui::RichText::new(
                        "Enter the label size: gap:WIDTHxHEIGHT, circle:DIAMETER, or continuous:WIDTH, in mm",
                    )
                    .small()
                    .color(colour),
                );
            }

            ui.horizontal(|ui| {
                edited |= ui
                    .selectable_value(&mut self.fit, false, "Fill the label")
                    .changed();
                edited |= ui
                    .selectable_value(&mut self.fit, true, "Show the whole picture")
                    .changed();
            });
            let style = ui.label(
                egui::RichText::new("Style").font(egui::FontId::new(14.0, theme::semibold())),
            );
            let before = self.style;
            egui::ComboBox::from_id_salt("style")
                .width(260.0)
                .selected_text(STYLES[self.style].0)
                .show_ui(ui, |ui| {
                    for (index, (label, _)) in STYLES.iter().enumerate() {
                        ui.selectable_value(&mut self.style, index, *label);
                    }
                })
                .response
                .labelled_by(style.id);
            edited |= self.style != before;
        });
        edited
    }

    pub fn preview_ui(&mut self, ui: &mut egui::Ui, saved_media: Option<&Media>) {
        let media = self.media(saved_media);
        match (self.shown_job(saved_media), &media) {
            (Ok(job), Some(media)) => {
                let key = (media.clone(), self.fit, self.style);
                self.preview.update(ui.ctx(), &key, || {
                    job.raster(HEAD_DOTS).map_err(|error| {
                        Problem::new("Couldn't turn this picture into a label.", &error)
                    })
                });
            }
            _ => self.preview.clear(),
        }
        let caption = match media {
            Some(media) => format!("Preview · {media} label"),
            None => "Preview".into(),
        };
        self.preview.show(ui, &caption);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../examples/first-label.png"
        ))
    }

    #[test]
    fn needs_an_image_and_media() {
        let gap: Media = "gap:50x30".parse().unwrap();
        let mut tab = ImageTab::default();
        assert_eq!(tab.job(Some(&gap)).err(), Some("Choose a picture to print"));
        tab.open_now(png());
        assert_eq!(
            tab.job(None).err(),
            Some("Enter the label size to print"),
            "no saved media and none typed"
        );
        assert!(matches!(tab.job(Some(&gap)), Ok(Job::Image { .. })));
        tab.media_text = "gap:40x30".into();
        let Ok(Job::Image { media, .. }) = tab.job(Some(&gap)) else {
            panic!()
        };
        assert_eq!(media.to_string(), "gap:40x30");
        tab.media_text = "nonsense".into();
        assert!(tab.job(Some(&gap)).is_err());
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::GrayImage::new(width, height)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn a_picture_past_the_limits_is_too_large_not_unreadable() {
        let bytes = png_bytes(20_000, 1);
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes));
        let problem = decode(reader).unwrap_err();
        assert_eq!(problem.summary, "This picture is too large to open.");
        assert!(problem.details.contains("limit"), "{}", problem.details);

        let reader = image::ImageReader::new(std::io::Cursor::new(b"not a picture".to_vec()));
        assert_eq!(
            decode(reader).unwrap_err().summary,
            "Couldn't open this picture."
        );
    }

    #[test]
    fn large_pictures_are_shrunk_once_at_load() {
        let reader = image::ImageReader::new(std::io::Cursor::new(png_bytes(3000, 600)));
        let image = decode(reader).unwrap();
        assert_eq!(image.dimensions(), (2048, 410));

        let reader = image::ImageReader::new(std::io::Cursor::new(png_bytes(640, 480)));
        assert_eq!(decode(reader).unwrap().dimensions(), (640, 480));
    }

    #[test]
    fn unreadable_files_report_an_error() {
        let mut tab = ImageTab::default();
        tab.open_now(PathBuf::from("/nonexistent.png"));
        let problem = tab.problem.as_ref().unwrap();
        assert_eq!(problem.summary, "Couldn't open this picture.");
        assert!(problem.details.contains("/nonexistent.png"));
        assert!(tab.source.is_none());
    }

    fn loaded(generation: u64, path: &str) -> Loaded {
        Loaded {
            generation,
            path: path.into(),
            result: Ok(RgbaImage::new(1, 1)),
        }
    }

    #[test]
    fn an_open_is_pending_until_its_result_arrives() {
        let mut opening = Opening::default();
        let generation = opening.begin("a.png".into());
        assert_eq!(opening.pending, Some("a.png".into()));
        let (path, result) = opening.finish(loaded(generation, "a.png")).unwrap();
        assert_eq!(path, PathBuf::from("a.png"));
        assert!(result.is_ok());
        assert_eq!(opening.pending, None);
    }

    #[test]
    fn only_the_latest_open_is_applied() {
        let mut opening = Opening::default();
        let first = opening.begin("a.png".into());
        let second = opening.begin("b.png".into());
        assert!(opening.finish(loaded(first, "a.png")).is_none());
        assert_eq!(opening.pending, Some("b.png".into()), "still waiting");
        assert!(opening.finish(loaded(second, "b.png")).is_some());
        // A result arriving after its open was applied is never applied twice.
        assert!(opening.finish(loaded(second, "b.png")).is_none());
    }

    #[test]
    fn the_previous_picture_stays_while_the_next_opens() {
        let gap: Media = "gap:50x30".parse().unwrap();
        let mut tab = ImageTab::default();
        tab.open_now(png());
        let generation = tab.begin("/elsewhere/next.png".into());
        assert_eq!(tab.job(Some(&gap)).err(), Some("Opening the picture…"));
        assert!(tab.shown_job(Some(&gap)).is_ok(), "the preview keeps it");
        assert_eq!(tab.path, Some(png()));

        // A failed open reports why and goes back to the picture it kept.
        assert!(!tab.finish(Loaded {
            generation,
            path: "/elsewhere/next.png".into(),
            result: Err(Problem::new(TOO_LARGE, &anyhow::anyhow!("limits"))),
        }));
        assert_eq!(tab.problem.as_ref().unwrap().summary, TOO_LARGE);
        assert!(tab.job(Some(&gap)).is_ok());
        assert_eq!(tab.path, Some(png()));
    }

    #[test]
    fn opening_decodes_off_the_calling_thread() {
        let mut tab = ImageTab::default();
        tab.open(&egui::Context::default(), png());
        assert!(tab.loader.opening.pending.is_some());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !tab.poll() {
            assert!(std::time::Instant::now() < deadline, "never loaded");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(tab.path, Some(png()));
        assert!(tab.source.is_some());
        assert!(tab.loader.opening.pending.is_none());
    }
}
