//! One-bit image conversion adapted from synapdeck-togo's img2idx.py.
//! Dithering operates in linear light; SDK threshold conversion is unchanged.
use crate::raster::Raster;
use anyhow::{Result, ensure};
use image::{
    Rgb, Rgb32FImage, Rgba, RgbaImage,
    imageops::{self, FilterType},
};

#[derive(Clone, Copy, Debug)]
pub enum Dither {
    FloydSteinberg,
    Atkinson,
    Jarvis,
    Stucki,
    Sierra,
    SierraLite,
    Bayer,
}

impl std::str::FromStr for Dither {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "floyd-steinberg" => Self::FloydSteinberg,
            "atkinson" => Self::Atkinson,
            "jarvis" => Self::Jarvis,
            "stucki" => Self::Stucki,
            "sierra" => Self::Sierra,
            "sierra-lite" => Self::SierraLite,
            "bayer" => Self::Bayer,
            _ => anyhow::bail!(
                "Dither must be none, floyd-steinberg, atkinson, jarvis, stucki, sierra, sierra-lite, or bayer"
            ),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Conversion {
    Threshold(u8),
    Dither(Dither),
}

impl From<u8> for Conversion {
    fn from(threshold: u8) -> Self {
        Self::Threshold(threshold)
    }
}

/// Composite on white, resize, then reduce to one bit. Rows are MSB-first,
/// 1=black, and padded with white to a multiple of eight pixels.
/// Dithered images use linear RGB Lanczos3 and Rec.709 luminance. Threshold
/// images retain the SDK-compatible sRGB/triangle preparation path.
pub fn convert(source: &RgbaImage, size: (u32, u32), conversion: Conversion) -> Result<Raster> {
    let (width, height) = size;
    ensure!(
        source.width() > 0 && source.height() > 0 && width > 0 && height > 0,
        "Image dimensions must be nonzero"
    );
    ensure!(
        width.div_ceil(8) <= u32::from(u16::MAX),
        "Raster byte width exceeds the wire field"
    );
    // Bound the working images, including image::resize's vertical-first float
    // intermediate. A narrow, very tall target can otherwise expand enormously.
    let working_bytes = u64::from(source.width()) * u64::from(source.height()) * 12
        + u64::from(source.width()) * u64::from(height) * 16
        + u64::from(width) * u64::from(height) * 20;
    ensure!(
        working_bytes <= 256 * 1024 * 1024,
        "Image conversion exceeds 256 MiB working limit"
    );
    let method = match conversion {
        Conversion::Threshold(threshold) => {
            let mut opaque = RgbaImage::from_pixel(source.width(), source.height(), Rgba([255; 4]));
            imageops::overlay(&mut opaque, source, 0, 0);
            let resized = imageops::resize(&opaque, width, height, FilterType::Triangle);
            return Raster::from_rgba(&resized, threshold);
        }
        Conversion::Dither(method) => method,
    };
    let linear = Rgb32FImage::from_fn(source.width(), source.height(), |x, y| {
        let [r, g, b, a] = source.get_pixel(x, y).0;
        let alpha = f64::from(a) / 255.0;
        Rgb([r, g, b].map(|v| {
            let srgb = f64::from(v) / 255.0;
            let light = if srgb <= 0.04045 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            };
            (light * alpha + 1.0 - alpha) as f32
        }))
    });
    let resized = imageops::resize(&linear, width, height, FilterType::Lanczos3);
    drop(linear);
    let mut work: Vec<f64> = resized
        .pixels()
        .map(|pixel| {
            let [r, g, b] = pixel.0.map(|v| f64::from(v.clamp(0.0, 1.0)));
            r * 0.2126 + g * 0.7152 + b * 0.0722
        })
        .collect();
    drop(resized);
    let (kernel, divisor): (&[(i32, u32, f64)], f64) = match method {
        Dither::FloydSteinberg => (&[(1, 0, 7.), (-1, 1, 3.), (0, 1, 5.), (1, 1, 1.)], 16.),
        Dither::Atkinson => (
            &[
                (1, 0, 1.),
                (2, 0, 1.),
                (-1, 1, 1.),
                (0, 1, 1.),
                (1, 1, 1.),
                (0, 2, 1.),
            ],
            8.,
        ),
        Dither::Jarvis => (
            &[
                (1, 0, 7.),
                (2, 0, 5.),
                (-2, 1, 3.),
                (-1, 1, 5.),
                (0, 1, 7.),
                (1, 1, 5.),
                (2, 1, 3.),
                (-2, 2, 1.),
                (-1, 2, 3.),
                (0, 2, 5.),
                (1, 2, 3.),
                (2, 2, 1.),
            ],
            48.,
        ),
        Dither::Stucki => (
            &[
                (1, 0, 8.),
                (2, 0, 4.),
                (-2, 1, 2.),
                (-1, 1, 4.),
                (0, 1, 8.),
                (1, 1, 4.),
                (2, 1, 2.),
                (-2, 2, 1.),
                (-1, 2, 2.),
                (0, 2, 4.),
                (1, 2, 2.),
                (2, 2, 1.),
            ],
            42.,
        ),
        Dither::Sierra => (
            &[
                (1, 0, 5.),
                (2, 0, 3.),
                (-2, 1, 2.),
                (-1, 1, 4.),
                (0, 1, 5.),
                (1, 1, 4.),
                (2, 1, 2.),
                (-1, 2, 2.),
                (0, 2, 3.),
                (1, 2, 2.),
            ],
            32.,
        ),
        Dither::SierraLite => (&[(1, 0, 2.), (-1, 1, 1.), (0, 1, 1.)], 4.),
        Dither::Bayer => (&[], 1.),
    };
    let stride = width.div_ceil(8) as usize;
    let mut bytes = vec![0; stride * height as usize];
    for y in 0..height {
        for step in 0..width {
            let reverse = y % 2 != 0;
            let x = if reverse { width - 1 - step } else { step };
            let old = work[(y * width + x) as usize];
            // Diffusion rounds midpoint ties up; ordered quantization uses a
            // strict comparison, matching the reference at threshold ties.
            let white = if matches!(method, Dither::Bayer) {
                old > (f64::from(bayer_rank(x, y)) + 0.5) / 64.0
            } else {
                old >= 0.5
            };
            if !white {
                bytes[y as usize * stride + x as usize / 8] |= 0x80 >> (x % 8);
            }
            let error = (old - if white { 1.0 } else { 0.0 }) / divisor;
            for &(dx, dy, weight) in kernel {
                let nx = i64::from(x) + i64::from(if reverse { -dx } else { dx });
                if (0..i64::from(width)).contains(&nx) && y + dy < height {
                    work[((y + dy) * width) as usize + nx as usize] += error * weight;
                }
            }
        }
    }
    Raster::new(width.div_ceil(8) * 8, height, bytes)
}

fn bayer_rank(x: u32, y: u32) -> u32 {
    // Recursive [[4M,4M+2],[4M+3,4M+1]] expansion to an 8x8 matrix.
    let mut rank = 0;
    for bit in 0..3 {
        rank = rank * 4 + [[0, 2], [3, 1]][((y >> bit) & 1) as usize][((x >> bit) & 1) as usize];
    }
    rank
}
