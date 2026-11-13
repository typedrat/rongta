//! R22 physical-label geometry, separate from wire raster packing.
use crate::{
    media::Media,
    monochrome::{self, Conversion},
    raster::Raster,
};
use anyhow::{Result, ensure};
use image::{RgbaImage, imageops};

#[derive(Clone, Copy, Debug)]
pub enum Layout {
    Label,
    Fit,
}
impl std::str::FromStr for Layout {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "label" => Ok(Self::Label),
            "fit" => Ok(Self::Fit),
            _ => anyhow::bail!("Layout must be label or fit"),
        }
    }
}

/// Dots inside the area the R22 can mark on a label of this size: the head is
/// 48 mm wide, and 1 mm at the top and bottom stays blank.
pub fn printable_size(width_mm: f64, height_mm: f64) -> (u32, u32) {
    (
        (width_mm.min(48.0) * 8.0) as u32,
        ((height_mm - 2.0) * 8.0) as u32,
    )
}

/// Centre the top-left `size` dots of `content` on a head-width canvas
/// `canvas_height` dots tall. Dots outside a circular label stay white.
pub fn place(
    content: &Raster,
    size: (u32, u32),
    media: &Media,
    head_width: u32,
    canvas_height: u32,
) -> Result<Raster> {
    let (content_width, content_height) = size;
    ensure!(
        content_width <= content.width()
            && content_height <= content.height()
            && content_width <= head_width
            && content_height <= canvas_height,
        "Content does not fit the label canvas"
    );
    let offset_x = (head_width - content_width) / 2;
    let offset_y = (canvas_height - content_height) / 2;
    let stride = head_width as usize / 8;
    let mut bytes = vec![0; stride * canvas_height as usize];
    for y in 0..content_height {
        for x in 0..content_width {
            let dst_x = x + offset_x;
            let dst_y = y + offset_y;
            if let Media::Circle { diameter_mm } = media {
                let radius = diameter_mm * 4.0;
                let dx = f64::from(dst_x) + 0.5 - f64::from(head_width) / 2.0;
                let dy = f64::from(dst_y) + 0.5 - f64::from(canvas_height) / 2.0;
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
            }
            if content.bytes()[y as usize * (content.width() as usize / 8) + x as usize / 8]
                & (0x80 >> (x % 8))
                != 0
            {
                bytes[dst_y as usize * stride + dst_x as usize / 8] |= 0x80 >> (dst_x % 8);
            }
        }
    }
    Raster::new(head_width, canvas_height, bytes)
}

/// Crop a full-label design (Label), or fit an entire image (Fit), using 8 dots/mm.
/// White margins and circular masks are applied after monochrome conversion.
pub fn prepare(
    source: &RgbaImage,
    media: &Media,
    head_width: u32,
    layout: Layout,
    length_mm: Option<f64>,
    conversion: impl Into<Conversion>,
) -> Result<Raster> {
    ensure!(
        source.width() > 0 && source.height() > 0,
        "Source image is empty"
    );
    ensure!(
        head_width > 0 && head_width.is_multiple_of(8),
        "Invalid printer head width"
    );
    ensure!(
        length_mm.is_none() || matches!(media, Media::Continuous { .. }),
        "--length-mm only applies to continuous media"
    );
    let width_mm = media.width_mm();
    let height_mm = media
        .height_mm()
        .or(length_mm)
        .unwrap_or(width_mm * f64::from(source.height()) / f64::from(source.width()));
    ensure!(
        width_mm.is_finite() && width_mm > 0.0 && width_mm <= 50.0,
        "Invalid R22 media width"
    );
    ensure!(
        height_mm.is_finite() && height_mm > 2.0 && height_mm <= 1000.0,
        "Label length must be greater than 2 mm and at most 1000 mm"
    );
    let content_width_mm = width_mm.min(48.0);
    let content_height_mm = height_mm - 2.0;
    let (width, height) = printable_size(width_mm, height_mm);
    let canvas_height = (height_mm * 8.0) as u32;
    ensure!(
        width > 0 && height > 0 && width <= head_width,
        "Media printable area does not fit the printer head"
    );
    ensure!(
        u64::from(head_width) * u64::from(canvas_height) * 4 <= 64 * 1024 * 1024,
        "Prepared image exceeds 64 MiB limit"
    );
    let conversion = conversion.into();
    let (raster, content_width, content_height) = match layout {
        Layout::Label => {
            let crop_width = ((f64::from(source.width()) * content_width_mm / width_mm) as u32)
                .clamp(1, source.width());
            let crop_height = ((f64::from(source.height()) * content_height_mm / height_mm) as u32)
                .clamp(1, source.height());
            let crop = imageops::crop_imm(
                source,
                (source.width() - crop_width) / 2,
                (source.height() - crop_height) / 2,
                crop_width,
                crop_height,
            )
            .to_image();
            (
                monochrome::convert(&crop, (width, height), conversion)?,
                width,
                height,
            )
        }
        Layout::Fit => {
            let scale = (f64::from(width) / f64::from(source.width()))
                .min(f64::from(height) / f64::from(source.height()));
            let width = ((f64::from(source.width()) * scale) as u32).max(1);
            let height = ((f64::from(source.height()) * scale) as u32).max(1);
            (
                monochrome::convert(source, (width, height), conversion)?,
                width,
                height,
            )
        }
    };
    place(
        &raster,
        (content_width, content_height),
        media,
        head_width,
        canvas_height,
    )
}

/// An exact black/white preview of the packed pixels that will be encoded.
pub fn preview(raster: &Raster) -> image::GrayImage {
    image::GrayImage::from_fn(raster.width(), raster.height(), |x, y| {
        let black = raster.bytes()[y as usize * (raster.width() as usize / 8) + x as usize / 8]
            & (0x80 >> (x % 8))
            != 0;
        image::Luma([if black { 0 } else { 255 }])
    })
}
