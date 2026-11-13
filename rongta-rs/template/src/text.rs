//! Draws lines of text at the printer's dot pitch.
use crate::fonts;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use anyhow::{Result, bail, ensure};
use image::{GrayImage, Luma};

/// 2 mm em: smaller text loses strokes on a 203 dpi head.
const MIN_SIZE_DOTS: f32 = 16.0;
/// 1 mm. Labels narrower than the head are printable to their physical edge,
/// so text keeps clear of it in case the label sits slightly off-centre.
const INSET_DOTS: u32 = 8;

pub struct Line {
    pub text: String,
    /// Em size.
    pub size_dots: f32,
    pub bold: bool,
}

fn font(bold: bool) -> FontRef<'static> {
    FontRef::try_from_slice(if bold { fonts::BOLD } else { fonts::REGULAR })
        .expect("bundled font is valid")
}

/// ab_glyph scales so that ascent − descent spans the given pixels; sizes
/// here are ems, as in any other typesetting.
fn scale(font: &FontRef<'_>, em_dots: f32) -> PxScale {
    let units_per_em = font.units_per_em().expect("bundled font has units per em");
    PxScale::from(em_dots * font.height_unscaled() / units_per_em)
}

fn advance(font: &FontRef<'_>, scale: PxScale, text: &str) -> f32 {
    let scaled = font.as_scaled(scale);
    let mut width = 0.0;
    let mut previous = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(previous) = previous {
            width += scaled.kern(previous, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

/// Stacks `lines` left-aligned and vertically centred in the first `column`
/// dots of `canvas`, 1 mm in from its left edge, shrinking any line too wide
/// to fit.
pub fn draw(canvas: &mut GrayImage, lines: &[Line], column: u32) -> Result<()> {
    let room = column.saturating_sub(INSET_DOTS) as f32;
    let mut fitted = Vec::with_capacity(lines.len());
    for line in lines {
        let font = font(line.bold);
        // Without this check a missing glyph prints as an empty box.
        if let Some(c) = line.text.chars().find(|&c| font.glyph_id(c).0 == 0) {
            bail!(
                "{:?} contains {c:?}, which the label font cannot print",
                line.text
            );
        }
        let natural = advance(&font, scale(&font, line.size_dots), &line.text);
        let size = if natural > room {
            (line.size_dots * room / natural).floor()
        } else {
            line.size_dots
        };
        ensure!(
            size >= MIN_SIZE_DOTS,
            "{:?} is too long to fit beside the QR code",
            line.text
        );
        fitted.push((scale(&font, size), font, line.text.as_str()));
    }
    let heights: Vec<f32> = fitted
        .iter()
        .map(|(scale, font, _)| font.as_scaled(*scale).height())
        .collect();
    let total: f32 = heights.iter().sum();
    ensure!(
        total <= canvas.height() as f32,
        "The text is too tall for this label"
    );

    let mut top = (canvas.height() as f32 - total) / 2.0;
    for ((scale, font, text), height) in fitted.iter().zip(&heights) {
        let scaled = font.as_scaled(*scale);
        let baseline = top + scaled.ascent();
        let mut x = INSET_DOTS as f32;
        let mut previous = None;
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            if let Some(previous) = previous {
                x += scaled.kern(previous, id);
            }
            let glyph = id.with_scale_and_position(*scale, point(x, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    // Dots are on or off; a half-coverage cut keeps strokes at
                    // their designed weight.
                    if coverage >= 0.5
                        && px >= 0
                        && py >= 0
                        && (px as u32) < column
                        && (py as u32) < canvas.height()
                    {
                        canvas.put_pixel(px as u32, py as u32, Luma([0]));
                    }
                });
            }
            x += scaled.h_advance(id);
            previous = Some(id);
        }
        top += height;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(canvas: &GrayImage) -> Option<(u32, u32, u32, u32)> {
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for (x, y, pixel) in canvas.enumerate_pixels() {
            if pixel.0[0] == 0 {
                let (x0, y0, x1, y1) = bounds.unwrap_or((x, y, x, y));
                bounds = Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)));
            }
        }
        bounds
    }

    fn line(text: &str, size_dots: f32) -> Line {
        Line {
            text: text.to_owned(),
            size_dots,
            bold: false,
        }
    }

    fn blank() -> GrayImage {
        GrayImage::from_pixel(400, 120, Luma([255]))
    }

    #[test]
    fn size_is_the_em_so_capitals_are_cap_height_tall() {
        let font = font(false);
        let cap_height = font
            .outline(font.glyph_id('H'))
            .map(|outline| (outline.bounds.max.y - outline.bounds.min.y).abs())
            .unwrap();
        let em = 40.0;
        let expected = em * cap_height / font.units_per_em().unwrap();

        let mut canvas = blank();
        draw(&mut canvas, &[line("H", em)], 400).unwrap();
        let (_, top, _, bottom) = ink(&canvas).unwrap();
        let height = (bottom - top + 1) as f32;
        assert!((height - expected).abs() <= 1.0, "{height} vs {expected}");
    }

    #[test]
    fn text_starts_a_millimetre_in() {
        let mut canvas = blank();
        draw(&mut canvas, &[line("H", 40.0)], 400).unwrap();
        let (left, ..) = ink(&canvas).unwrap();
        assert!(left >= INSET_DOTS, "{left}");
    }

    #[test]
    fn long_lines_shrink_to_fit_their_column() {
        let font = font(false);
        let text = "Scan to redeem";
        let natural = advance(&font, scale(&font, 40.0), text);
        let column = 160;
        assert!(natural > (column - INSET_DOTS) as f32, "{natural}");

        let mut canvas = blank();
        draw(&mut canvas, &[line(text, 40.0)], column).unwrap();
        let (left, _, right, _) = ink(&canvas).unwrap();
        assert!(left >= INSET_DOTS && right < column, "{left}..{right}");
        // Shrunk, not clipped: the same text set at the fitted size with
        // unlimited room draws exactly the same dots.
        let fitted = (40.0 * (column - INSET_DOTS) as f32 / natural).floor();
        let mut unclipped = blank();
        draw(&mut unclipped, &[line(text, fitted)], 400).unwrap();
        assert_eq!(canvas, unclipped);
    }

    #[test]
    fn lines_that_would_need_less_than_2_mm_are_refused() {
        let mut canvas = blank();
        let error = draw(&mut canvas, &[line(&"W".repeat(40), 40.0)], 160)
            .unwrap_err()
            .to_string();
        assert!(error.contains("too long"), "{error}");
    }

    #[test]
    fn characters_without_a_glyph_are_refused() {
        let mut canvas = blank();
        let error = draw(&mut canvas, &[line("Code 日本", 24.0)], 400)
            .unwrap_err()
            .to_string();
        assert!(error.contains('日'), "{error}");
    }
}
