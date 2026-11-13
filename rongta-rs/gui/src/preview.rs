//! The label exactly as it will print: the same raster the printer gets,
//! shown as white paper with black dots whatever the theme.
use crate::{
    errors::{Problem, problem_ui},
    theme,
};
use eframe::egui::{
    self, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, TextureFilter, TextureOptions,
    Vec2, emath::GuiRounding as _, pos2,
};
use rongta::{image_processing, raster::Raster};

/// Dots across the print head: the widest raster there is to show.
pub const HEAD_DOTS: u32 = 384;

const PAPER_STROKE: f32 = 1.0;

/// Width the preview card adds around the raster: its own padding and
/// border, plus the paper's border.
pub const CHROME: f32 = theme::CARD_CHROME + 2.0 * PAPER_STROKE;

/// Whole zoom only ever magnifies, so nearest keeps each dot a crisp square;
/// a shrunk preview averages neighbouring dots instead of dropping some.
const DOTS: TextureOptions = TextureOptions {
    magnification: TextureFilter::Nearest,
    minification: TextureFilter::Linear,
    ..TextureOptions::NEAREST
};

/// How many physical pixels each printer dot covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// The same whole number of pixels for every dot, so the preview shows
    /// every dot at one size.
    Whole(u32),
    /// Under a pixel per dot; only for windows too narrow for the head.
    Shrunk(f32),
}

impl Zoom {
    /// The zoom for `dots` across `available` points. Zoom is counted in
    /// physical pixels so it stays whole at fractional display scales. It
    /// aims for about one point per dot: larger would crowd out the form
    /// without showing anything more.
    pub fn fit(available: f32, pixels_per_point: f32, dots: u32) -> Option<Self> {
        if !(available > 0.0 && pixels_per_point > 0.0 && dots > 0) {
            return None;
        }
        // The tolerance absorbs float error when `available` was itself
        // computed from a whole zoom, as `column_width` does.
        let room = (available * pixels_per_point + 1e-3) / dots as f32;
        let target = pixels_per_point.round().max(1.0);
        Some(if room >= 1.0 {
            Self::Whole(room.floor().min(target) as u32)
        } else {
            Self::Shrunk(available * pixels_per_point / dots as f32)
        })
    }

    /// The length in points of `dots` drawn at this zoom.
    pub fn points(self, dots: u32, pixels_per_point: f32) -> f32 {
        let pixels_per_dot = match self {
            Self::Whole(k) => k as f32,
            Self::Shrunk(scale) => scale,
        };
        dots as f32 * pixels_per_dot / pixels_per_point
    }
}

/// The width of a preview column offered `available` points: enough for the
/// whole head at whole zoom, or all of it when even one pixel per dot won't
/// fit.
pub fn column_width(available: f32, pixels_per_point: f32) -> f32 {
    match Zoom::fit(available - CHROME, pixels_per_point, HEAD_DOTS) {
        Some(zoom @ Zoom::Whole(_)) => CHROME + zoom.points(HEAD_DOTS, pixels_per_point),
        _ => available.max(0.0),
    }
}

/// `K` identifies what was rendered, so a frame that changes nothing costs
/// one comparison.
pub struct Preview<K> {
    key: Option<K>,
    texture: Option<egui::TextureHandle>,
    problem: Option<Problem>,
}

impl<K> Default for Preview<K> {
    fn default() -> Self {
        Self {
            key: None,
            texture: None,
            problem: None,
        }
    }
}

impl<K: PartialEq + Clone> Preview<K> {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Renders again only when `key` changes, because rasterizing every frame
    /// would keep a CPU core busy while the window is open.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        key: &K,
        render: impl FnOnce() -> Result<Raster, Problem>,
    ) {
        if self.key.as_ref() == Some(key) {
            return;
        }
        self.key = Some(key.clone());
        match render() {
            Ok(raster) => {
                let gray = image_processing::preview(&raster);
                let image = egui::ColorImage::from_gray(
                    [gray.width() as usize, gray.height() as usize],
                    gray.as_raw(),
                );
                self.texture = Some(ctx.load_texture("preview", image, DOTS));
                self.problem = None;
            }
            Err(problem) => {
                self.texture = None;
                self.problem = Some(problem);
            }
        }
    }

    pub fn show(&self, ui: &mut egui::Ui, caption: &str) {
        let p = theme::palette(ui);
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(caption).color(p.muted));
            ui.add_space(4.0);
            match (&self.texture, &self.problem) {
                (Some(texture), _) => paper(ui, texture),
                (None, Some(problem)) => problem_ui(ui, problem),
                (None, None) => {
                    ui.label(egui::RichText::new("Nothing to preview yet").color(p.muted));
                }
            }
        });
    }
}

fn paper(ui: &mut egui::Ui, texture: &egui::TextureHandle) {
    let p = theme::palette(ui);
    let ppp = ui.pixels_per_point();
    let [width, height] = texture.size().map(|v| v as u32);
    let Some(zoom) = Zoom::fit(ui.available_width() - 2.0 * PAPER_STROKE, ppp, width) else {
        return;
    };
    let size = egui::vec2(zoom.points(width, ppp), zoom.points(height, ppp));
    let (outer, response) = ui
        .vertical_centered(|ui| {
            ui.allocate_exact_size(size + Vec2::splat(2.0 * PAPER_STROKE), Sense::hover())
        })
        .inner;
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::new(egui::Role::Image);
        info.label = Some("Label preview".into());
        info
    });
    // Centring can land on a fraction of a pixel. With the origin on a pixel
    // boundary as well as a whole-pixel size, each dot covers exactly its own
    // pixels instead of being resampled across their edges.
    let image = Rect::from_min_size(
        (outer.min + Vec2::splat(PAPER_STROKE)).round_to_pixels(ppp),
        size,
    );
    let painter = ui.painter();
    let shadow = egui::epaint::Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: Color32::from_black_alpha(40),
    };
    painter.add(shadow.as_shape(image, CornerRadius::ZERO));
    painter.image(
        texture.id(),
        image,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    painter.rect_stroke(
        image,
        CornerRadius::ZERO,
        Stroke::new(PAPER_STROKE, p.border),
        StrokeKind::Outside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_is_whole_physical_pixels_per_dot() {
        assert_eq!(Zoom::fit(384.0, 1.0, 384), Some(Zoom::Whole(1)));
        // More room doesn't mean more zoom: about a point per dot is enough.
        assert_eq!(Zoom::fit(1000.0, 1.0, 384), Some(Zoom::Whole(1)));
        assert_eq!(Zoom::fit(1000.0, 2.0, 384), Some(Zoom::Whole(2)));
        assert_eq!(Zoom::fit(300.0, 2.0, 384), Some(Zoom::Whole(1)));
        assert_eq!(Zoom::fit(600.0, 1.5, 384), Some(Zoom::Whole(2)));
        assert_eq!(Zoom::fit(400.0, 1.5, 384), Some(Zoom::Whole(1)));
        assert_eq!(Zoom::fit(1000.0, 1.25, 384), Some(Zoom::Whole(1)));
    }

    #[test]
    fn zoomed_size_is_whole_pixels_at_fractional_scale() {
        for ppp in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let zoom = Zoom::fit(1000.0, ppp, 384).unwrap();
            let pixels = zoom.points(384, ppp) * ppp;
            assert!((pixels - pixels.round()).abs() < 1e-3, "{ppp}: {pixels}");
        }
        assert_eq!(Zoom::Whole(1).points(384, 1.25), 307.2);
    }

    #[test]
    fn too_little_room_shrinks_instead() {
        assert_eq!(Zoom::fit(192.0, 1.0, 384), Some(Zoom::Shrunk(0.5)));
        assert_eq!(Zoom::Shrunk(0.5).points(384, 1.0), 192.0);
        assert_eq!(Zoom::fit(0.0, 1.0, 384), None);
        assert_eq!(Zoom::fit(-5.0, 1.0, 384), None);
        assert_eq!(Zoom::fit(100.0, 1.0, 0), None);
    }

    #[test]
    fn the_column_holds_the_head_at_whole_zoom() {
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let column = column_width(2000.0, ppp);
            assert!(column < 2000.0);
            // What the card leaves for the paper must give the same zoom back.
            let inside = Zoom::fit(column - CHROME, ppp, HEAD_DOTS);
            assert_eq!(inside, Zoom::fit(2000.0, ppp, HEAD_DOTS), "{ppp}");
        }
        assert_eq!(column_width(2000.0, 1.0), CHROME + 384.0);
        // Too narrow for one pixel per dot: take what there is.
        assert_eq!(column_width(300.0, 1.0), 300.0);
    }
}
