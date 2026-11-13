//! Printing an ordinary picture: choose it, choose how it fills the label and
//! how greys become dots, then preview.
use crate::{preview::Preview, theme, worker::Job};
use anyhow::{Context, Result};
use eframe::egui;
use image::RgbaImage;
use rongta::{
    image_processing::Layout,
    media::Media,
    monochrome::{Conversion, Dither},
};
use std::{path::PathBuf, sync::Arc};

/// (label, dither name); `None` is the SDK's sharp black-and-white threshold.
const STYLES: [(&str, Option<&str>); 4] = [
    ("Sharp — text and logos", None),
    ("Photo — smooth", Some("floyd-steinberg")),
    ("Photo — high contrast", Some("atkinson")),
    ("Pattern", Some("bayer")),
];

#[derive(Default)]
pub struct ImageTab {
    path: Option<PathBuf>,
    source: Option<Arc<RgbaImage>>,
    /// Empty means the printer's saved media.
    media_text: String,
    fit: bool,
    style: usize,
    error: Option<String>,
    preview: Preview,
}

/// Decoding is capped so a hostile or huge file can't exhaust memory.
fn load(path: &std::path::Path) -> Result<RgbaImage> {
    let mut reader = image::ImageReader::open(path)
        .with_context(|| format!("Cannot open {}", path.display()))?
        .with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader
        .decode()
        .context("This file isn't a picture this app can read")?
        .into_rgba8())
}

impl ImageTab {
    pub fn open(&mut self, path: PathBuf) {
        match load(&path) {
            Ok(image) => {
                self.source = Some(Arc::new(image));
                self.path = Some(path);
                self.error = None;
                self.preview.clear();
            }
            Err(error) => self.error = Some(format!("{error:#}")),
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

    pub fn job(&self, saved_media: Option<&Media>) -> Option<Job> {
        Some(Job::Image {
            source: self.source.clone()?,
            media: self.media(saved_media)?,
            layout: if self.fit { Layout::Fit } else { Layout::Label },
            conversion: self.conversion(),
        })
    }

    pub fn form_ui(&mut self, ui: &mut egui::Ui, saved_media: Option<&Media>) {
        let p = theme::palette(ui);
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
                        self.open(path);
                    }
                });
            });
            if let Some(error) = &self.error {
                ui.label(egui::RichText::new(error).color(p.error));
            }

            let labels = ui.label(
                egui::RichText::new("Labels").font(egui::FontId::new(14.0, theme::semibold())),
            );
            let placeholder = saved_media
                .map(|media| format!("{media} (saved)"))
                .unwrap_or_else(|| "e.g. gap:50x30".into());
            ui.add(
                egui::TextEdit::singleline(&mut self.media_text)
                    .hint_text(placeholder)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(labels.id);
            if self.media(saved_media).is_none() {
                ui.label(
                    egui::RichText::new(
                        "Enter the label size: gap:WIDTHxHEIGHT, circle:DIAMETER, or continuous:WIDTH, in mm",
                    )
                    .small()
                    .color(p.error),
                );
            }

            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.fit, false, "Fill the label");
                ui.selectable_value(&mut self.fit, true, "Show the whole picture");
            });
            let style = ui.label(
                egui::RichText::new("Style").font(egui::FontId::new(14.0, theme::semibold())),
            );
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
        });
    }

    pub fn preview_ui(&mut self, ui: &mut egui::Ui, saved_media: Option<&Media>) {
        match self.job(saved_media) {
            Some(job) => {
                let key = format!(
                    "{:?}|{}|{}|{}|{saved_media:?}",
                    self.path, self.media_text, self.fit, self.style
                );
                self.preview.update(ui.ctx(), key, move || job.raster(384));
            }
            None => self.preview.clear(),
        }
        let caption = match self.media(saved_media) {
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
        assert!(tab.job(Some(&gap)).is_none());
        tab.open(png());
        assert!(tab.job(None).is_none(), "no saved media and none typed");
        assert!(matches!(tab.job(Some(&gap)), Some(Job::Image { .. })));
        tab.media_text = "gap:40x30".into();
        let Some(Job::Image { media, .. }) = tab.job(Some(&gap)) else {
            panic!()
        };
        assert_eq!(media.to_string(), "gap:40x30");
        tab.media_text = "nonsense".into();
        assert!(tab.job(Some(&gap)).is_none());
    }

    #[test]
    fn unreadable_files_report_an_error() {
        let mut tab = ImageTab::default();
        tab.open(PathBuf::from("/nonexistent.png"));
        assert!(tab.error.is_some());
        assert!(tab.source.is_none());
    }
}
