use crate::{Template, Values, interpolate, qr, text};
use anyhow::{Result, ensure};
use image::{DynamicImage, GrayImage, Luma, RgbaImage};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rongta::{image_processing, raster::Raster};

/// RFC 3986 unreserved characters pass through; everything else in a value is
/// encoded so it cannot change the URL's structure.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

impl Template {
    /// Draws the printable area alone, one pixel per dot, in pure black and
    /// white. `values` must already have been through `normalize`. Fails on
    /// the errors `check` reports and on layout problems it cannot see: text
    /// too long to fit, QR content too long, or characters the font lacks, so
    /// only a successful render shows that a label can print.
    pub fn render(&self, values: &Values) -> Result<RgbaImage> {
        let errors = self.check(values);
        ensure!(
            errors.is_empty(),
            "{}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
        let (width, height) = self.printable_size();
        let mut canvas = GrayImage::from_pixel(width, height, Luma([255]));
        let url = interpolate::fill(&self.qr, |name| {
            utf8_percent_encode(&values[name], COMPONENT).to_string()
        });
        // Text needs at least 40% of the width to stay legible beside the code.
        let max_side = if self.lines.is_empty() {
            width.min(height)
        } else {
            height.min(width - (width * 2).div_ceil(5))
        };
        let column = qr::draw(&mut canvas, &url, self.error_correction, max_side)?;
        let lines: Vec<text::Line> = self
            .lines
            .iter()
            .map(|line| text::Line {
                text: interpolate::fill(&line.content, |name| values[name].clone()),
                size_dots: (line.size_mm * 8.0) as f32,
                bold: line.bold,
            })
            .collect();
        text::draw(&mut canvas, &lines, column)?;
        Ok(DynamicImage::ImageLuma8(canvas).into_rgba8())
    }

    /// The whole label canvas at `head_width` dots, as it should be previewed
    /// and printed.
    pub fn raster(&self, values: &Values, head_width: u32) -> Result<Raster> {
        let image = self.render(values)?;
        // The image is already pure black and white, so any mid threshold is exact.
        let content = Raster::from_rgba(&image, 127)?;
        image_processing::place(
            &content,
            image.dimensions(),
            &self.media,
            head_width,
            (self.size_mm().1 * 8.0) as u32,
        )
    }
}
