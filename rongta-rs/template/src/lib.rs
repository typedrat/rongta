//! Fill-in label templates: typed fields, lines of text on the left and a QR
//! code on the right, drawn at the printer's dot pitch.

mod interpolate;

/// Atkinson Hyperlegible Next (SIL Open Font License 1.1; see `fonts/OFL.txt`).
/// Its distinct letterforms keep codes like `O0Il1` readable at thermal
/// resolution, and bundling it makes labels identical on every host.
#[allow(dead_code)]
mod fonts {
    pub static REGULAR: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Regular.ttf");
    pub static BOLD: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Bold.ttf");
}
