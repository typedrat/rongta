//! Draws lines of text at the printer's dot pitch.
use crate::fonts;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use anyhow::{Result, ensure};
use image::{GrayImage, Luma};

/// 2 mm: smaller text loses strokes on a 203 dpi head.
const MIN_SIZE_DOTS: f32 = 16.0;

pub struct Line {
    pub text: String,
    pub size_dots: f32,
    pub bold: bool,
}

fn font(bold: bool) -> FontRef<'static> {
    FontRef::try_from_slice(if bold { fonts::BOLD } else { fonts::REGULAR })
        .expect("bundled font is valid")
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

/// Stacks `lines` left-aligned and vertically centred within the first
/// `column` dots of `canvas`, shrinking any line too wide to fit.
pub fn draw(canvas: &mut GrayImage, lines: &[Line], column: u32) -> Result<()> {
    let mut fitted = Vec::with_capacity(lines.len());
    for line in lines {
        let font = font(line.bold);
        let natural = advance(&font, PxScale::from(line.size_dots), &line.text);
        let size = if natural > column as f32 {
            (line.size_dots * column as f32 / natural).floor()
        } else {
            line.size_dots
        };
        ensure!(
            size >= MIN_SIZE_DOTS,
            "{:?} is too long to fit beside the QR code",
            line.text
        );
        fitted.push((font, PxScale::from(size), line.text.as_str()));
    }
    let heights: Vec<f32> = fitted
        .iter()
        .map(|(font, scale, _)| font.as_scaled(*scale).height())
        .collect();
    let total: f32 = heights.iter().sum();
    ensure!(
        total <= canvas.height() as f32,
        "The text is too tall for this label"
    );

    let mut top = (canvas.height() as f32 - total) / 2.0;
    for ((font, scale, text), height) in fitted.iter().zip(&heights) {
        let scaled = font.as_scaled(*scale);
        let baseline = top + scaled.ascent();
        let mut x = 0.0;
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
