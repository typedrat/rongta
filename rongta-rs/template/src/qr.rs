//! QR codes drawn with whole-dot modules; resampled modules print as grey
//! smears that phone cameras misread.
use crate::ErrorCorrection;
use anyhow::{Context, Result, ensure};
use image::{GrayImage, Luma};
use qrcode::{Color, EcLevel, QrCode};

/// The QR specification's minimum quiet zone, in modules.
const QUIET_MODULES: u32 = 4;
/// Smallest module, in 0.125 mm dots, that scans reliably off thermal paper.
const MIN_MODULE_DOTS: u32 = 3;

impl From<ErrorCorrection> for EcLevel {
    fn from(level: ErrorCorrection) -> Self {
        match level {
            ErrorCorrection::L => EcLevel::L,
            ErrorCorrection::M => EcLevel::M,
            ErrorCorrection::Q => EcLevel::Q,
            ErrorCorrection::H => EcLevel::H,
        }
    }
}

/// Draws `data` as the largest whole-dot code that fits a `max_side` square,
/// quiet zone included, flush right and centred vertically. Returns the x
/// where its quiet zone begins.
pub fn draw(
    canvas: &mut GrayImage,
    data: &str,
    level: ErrorCorrection,
    max_side: u32,
) -> Result<u32> {
    let code = QrCode::with_error_correction_level(data, level.into())
        .context("The QR content is too long to encode")?;
    let modules = code.width() as u32;
    let max_side = max_side.min(canvas.width()).min(canvas.height());
    let module = max_side / (modules + 2 * QUIET_MODULES);
    ensure!(
        module >= MIN_MODULE_DOTS,
        "The QR code needs {modules} modules per side, too many to scan on this label; \
         shorten the link or use a taller label"
    );
    let side = (modules + 2 * QUIET_MODULES) * module;
    let left = canvas.width() - side;
    let top = (canvas.height() - side) / 2;
    for (index, color) in code.to_colors().into_iter().enumerate() {
        if color != Color::Dark {
            continue;
        }
        let x0 = left + (QUIET_MODULES + index as u32 % modules) * module;
        let y0 = top + (QUIET_MODULES + index as u32 / modules) * module;
        for y in y0..y0 + module {
            for x in x0..x0 + module {
                canvas.put_pixel(x, y, Luma([0]));
            }
        }
    }
    Ok(left)
}
