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

use anyhow::{Context, Result, bail, ensure};
use interpolate::Piece;
use regex::Regex;
use rongta::media::Media;
use serde::Deserialize;
use std::{collections::BTreeMap, fmt, path::Path};

/// Field values by field id.
pub type Values = BTreeMap<String, String>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Case {
    #[default]
    Preserve,
    Upper,
    Lower,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum ErrorCorrection {
    L,
    #[default]
    M,
    Q,
    H,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
    pub case: Case,
    pattern: Option<Regex>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}
impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct TextLine {
    content: Vec<Piece>,
    size_mm: f64,
    bold: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Template {
    pub name: String,
    pub media: Media,
    pub fields: Vec<Field>,
    qr: Vec<Piece>,
    error_correction: ErrorCorrection,
    lines: Vec<TextLine>,
    width_mm: f64,
    height_mm: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    version: u32,
    name: String,
    media: String,
    #[serde(default, rename = "field")]
    fields: Vec<RawField>,
    qr: RawQr,
    #[serde(default, rename = "text")]
    lines: Vec<RawText>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    id: String,
    label: String,
    hint: Option<String>,
    pattern: Option<String>,
    #[serde(default)]
    case: Case,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQr {
    content: String,
    #[serde(default)]
    error_correction: ErrorCorrection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawText {
    content: String,
    size_mm: f64,
    #[serde(default)]
    bold: bool,
}

impl Template {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read template {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("Invalid template {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let raw: RawTemplate = toml::from_str(text)?;
        ensure!(
            raw.version == 1,
            "Unsupported template version {}; this app reads version 1",
            raw.version
        );
        let media: Media = raw.media.parse()?;
        let Media::Gap {
            width_mm,
            height_mm,
        } = media
        else {
            bail!("Templates support gap labels only, not {media}");
        };

        let mut fields: Vec<Field> = Vec::new();
        for field in raw.fields {
            ensure!(
                !field.id.is_empty()
                    && field
                        .id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "Field id {:?} must use only letters, digits, - and _",
                field.id
            );
            ensure!(
                fields.iter().all(|f| f.id != field.id),
                "Field {:?} is declared twice",
                field.id
            );
            let pattern = field
                .pattern
                .map(|p| Regex::new(&format!("^(?:{p})$")))
                .transpose()
                .with_context(|| format!("Field {:?} has an invalid pattern", field.id))?;
            fields.push(Field {
                id: field.id,
                label: field.label,
                hint: field.hint,
                case: field.case,
                pattern,
            });
        }

        let declared = |pieces: &[Piece]| -> Result<()> {
            for piece in pieces {
                if let Piece::Field(name) = piece {
                    ensure!(
                        fields.iter().any(|f| &f.id == name),
                        "{{{name}}} does not name a declared field"
                    );
                }
            }
            Ok(())
        };
        let qr = interpolate::parse(&raw.qr.content).context("In [qr] content")?;
        declared(&qr)?;
        let mut lines = Vec::new();
        for line in raw.lines {
            ensure!(
                line.size_mm.is_finite() && (1.0..=20.0).contains(&line.size_mm),
                "Text size_mm must be between 1 and 20, not {}",
                line.size_mm
            );
            let content = interpolate::parse(&line.content).context("In [[text]] content")?;
            declared(&content)?;
            lines.push(TextLine {
                content,
                size_mm: line.size_mm,
                bold: line.bold,
            });
        }

        Ok(Self {
            name: raw.name,
            media,
            fields,
            qr,
            error_correction: raw.qr.error_correction,
            lines,
            width_mm,
            height_mm,
        })
    }

    /// Trims surrounding whitespace and applies each field's case rule.
    /// Values for undeclared ids are dropped.
    pub fn normalize(&self, values: &Values) -> Values {
        self.fields
            .iter()
            .filter_map(|field| {
                let value = values.get(&field.id)?.trim();
                let value = match field.case {
                    Case::Preserve => value.to_owned(),
                    Case::Upper => value.to_uppercase(),
                    Case::Lower => value.to_lowercase(),
                };
                Some((field.id.clone(), value))
            })
            .collect()
    }

    /// One error per invalid field; empty when every field may be rendered.
    pub fn check(&self, values: &Values) -> Vec<FieldError> {
        self.fields
            .iter()
            .filter_map(|field| {
                let value = values.get(&field.id).map(String::as_str).unwrap_or("");
                let message = if value.is_empty() {
                    "Required".to_owned()
                } else if value.chars().any(char::is_control) {
                    "Contains control characters".to_owned()
                } else if let Some(pattern) = &field.pattern
                    && !pattern.is_match(value)
                {
                    field
                        .hint
                        .clone()
                        .unwrap_or_else(|| "Doesn't match the expected format".to_owned())
                } else {
                    return None;
                };
                Some(FieldError {
                    field: field.id.clone(),
                    message,
                })
            })
            .collect()
    }

    /// Size in dots of the area `render` draws.
    pub fn printable_size(&self) -> (u32, u32) {
        rongta::image_processing::printable_size(self.width_mm, self.height_mm)
    }
}
