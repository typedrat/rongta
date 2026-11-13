//! The label exactly as it will print: the same raster the printer gets,
//! shown as white paper with black dots whatever the theme.
use crate::theme;
use eframe::egui::{self, Color32, CornerRadius, Margin, Stroke, TextureOptions, vec2};
use rongta::{image_processing, raster::Raster};

#[derive(Default)]
pub struct Preview {
    key: Option<String>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
}

impl Preview {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Renders again only when `key` changes, because rasterizing every frame
    /// would keep a CPU core busy while the window is open.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        key: String,
        render: impl FnOnce() -> anyhow::Result<Raster>,
    ) {
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        match render() {
            Ok(raster) => {
                let gray = image_processing::preview(&raster);
                let image = egui::ColorImage::from_gray(
                    [gray.width() as usize, gray.height() as usize],
                    gray.as_raw(),
                );
                // Nearest-neighbour keeps each printer dot a crisp square.
                self.texture = Some(ctx.load_texture("preview", image, TextureOptions::NEAREST));
                self.error = None;
            }
            Err(error) => {
                self.texture = None;
                self.error = Some(format!("{error:#}"));
            }
        }
    }

    pub fn show(&self, ui: &mut egui::Ui, caption: &str) {
        let p = theme::palette(ui);
        theme::card(ui, |ui| {
            ui.label(egui::RichText::new(caption).color(p.muted));
            ui.add_space(4.0);
            match (&self.texture, &self.error) {
                (Some(texture), _) => {
                    let [w, h] = texture.size().map(|v| v as f32);
                    let available = ui.available_width() - 24.0;
                    // Whole-number zoom where it fits, so dots stay even.
                    let scale = if available >= w {
                        (available / w).floor().min(3.0)
                    } else {
                        available / w
                    };
                    ui.vertical_centered(|ui| {
                        egui::Frame::new()
                            .fill(Color32::WHITE)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(Margin::same(0))
                            .shadow(egui::epaint::Shadow {
                                offset: [0, 2],
                                blur: 8,
                                spread: 0,
                                color: Color32::from_black_alpha(40),
                            })
                            .show(ui, |ui| {
                                ui.add(egui::Image::new(egui::load::SizedTexture::new(
                                    texture.id(),
                                    vec2(w * scale, h * scale),
                                )));
                            });
                    });
                }
                (None, Some(error)) => {
                    ui.label(egui::RichText::new(error).color(p.error));
                }
                (None, None) => {
                    ui.label(egui::RichText::new("Nothing to preview yet").color(p.muted));
                }
            }
        });
    }
}
