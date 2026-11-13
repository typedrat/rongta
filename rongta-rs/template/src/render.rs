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
        let column = qr::draw(&mut canvas, &url, self.error_correction)?;
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

    pub fn raster(&self, values: &Values, head_width: u32) -> Result<Raster> {
        let image = self.render(values)?;
        // The image is already pure black and white, so any mid threshold is exact.
        let content = Raster::from_rgba(&image, 127)?;
        image_processing::place(
            &content,
            image.dimensions(),
            &self.media,
            head_width,
            (self.height_mm * 8.0) as u32,
        )
    }
}
