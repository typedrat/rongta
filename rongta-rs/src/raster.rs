//! Protocol-28 monochrome raster: rows left to right, high bit first, 1 = black.
use anyhow::{Result, ensure};
use image::RgbaImage;

#[derive(Clone, Debug)]
pub struct Raster {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

impl Raster {
    /// Construct an already packed raster. Width is in dots and must be byte-aligned.
    pub fn new(width: u32, height: u32, bytes: Vec<u8>) -> Result<Self> {
        ensure!(width > 0 && height > 0, "Raster dimensions must be nonzero");
        ensure!(
            width.is_multiple_of(8),
            "Packed raster width must be a multiple of 8"
        );
        ensure!(
            width / 8 <= u16::MAX as u32,
            "Raster byte width exceeds the wire field"
        );
        let length = (width as usize / 8).checked_mul(height as usize);
        ensure!(
            length == Some(bytes.len()),
            "Packed raster length does not match its dimensions"
        );
        Ok(Self {
            width,
            height,
            bytes,
        })
    }

    /// Reproduce SDK algorithm 0 (black/white), including its floating-point
    /// truncation and inclusive threshold. Alpha is ignored, as in the SDK.
    /// Composite transparent source images onto white before calling if desired.
    /// Partial rows are padded with white dots; the SDK's unaligned-width bug
    /// is deliberately not reproduced.
    pub fn from_rgba(image: &RgbaImage, threshold: u8) -> Result<Self> {
        let (width, height) = image.dimensions();
        ensure!(width > 0 && height > 0, "Raster dimensions must be nonzero");
        let byte_width = width.div_ceil(8);
        ensure!(
            byte_width <= u16::MAX as u32,
            "Raster byte width exceeds the wire field"
        );
        let length = (byte_width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| anyhow::anyhow!("Raster dimensions overflow"))?;
        let mut bytes = vec![0; length];
        for (x, y, pixel) in image.enumerate_pixels() {
            let [r, g, b, _] = pixel.0;
            // Operation order matters: this is not image::to_luma's formula.
            let gray = (f64::from(g) * 0.59 + f64::from(r) * 0.3 + f64::from(b) * 0.11) as u8;
            if gray <= threshold {
                bytes[y as usize * byte_width as usize + x as usize / 8] |= 0x80 >> (x % 8);
            }
        }
        Self::new(byte_width * 8, height, bytes)
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
