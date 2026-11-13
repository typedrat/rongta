//! Fill-in label templates: typed fields, lines of text on the left and a QR
//! code on the right, drawn at the printer's dot pitch.

mod interpolate;
mod qr;
mod render;
mod text;

/// Atkinson Hyperlegible Next (SIL Open Font License 1.1; see `fonts/OFL.txt`).
/// Its distinct letterforms keep codes like `O0Il1` readable at thermal
/// resolution, and bundling it makes labels identical on every host.
mod fonts {
    pub static REGULAR: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Regular.ttf");
    pub static BOLD: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Bold.ttf");
}

use anyhow::{Context, Result, ensure};
use interpolate::{Piece, Placeholder};
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
struct TextLine {
    content: Vec<Piece>,
    size_mm: f64,
    bold: bool,
    align: text::Align,
}

#[derive(Clone, Debug)]
pub struct Template {
    pub name: String,
    media: Media,
    pub fields: Vec<Field>,
    qr: Vec<Piece>,
    error_correction: ErrorCorrection,
    lines: Vec<TextLine>,
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
    #[serde(default)]
    align: text::Align,
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
        ensure!(
            matches!(media, Media::Gap { .. }),
            "Templates support gap labels only, not {media}"
        );

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
                .map(|p| {
                    // Compiled bare first: an unbalanced pattern such as
                    // `a)|(b` would otherwise close the anchoring group early
                    // and match prefixes and suffixes.
                    Regex::new(&p)?;
                    Regex::new(&format!("^(?:{p})$"))
                })
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
                if let Piece::Field(placeholder) = piece {
                    let field = fields
                        .iter()
                        .find(|f| f.id == placeholder.field)
                        .with_context(|| format!("{placeholder} does not name a declared field"))?;
                    if let Some(group) = &placeholder.group {
                        ensure!(
                            field
                                .pattern
                                .as_ref()
                                .is_some_and(|p| p.capture_names().flatten().any(|n| n == group)),
                            "{placeholder}: field {:?} has no group {group:?} in its pattern",
                            field.id
                        );
                    }
                }
            }
            Ok(())
        };
        let qr = interpolate::parse(&raw.qr.content).context("In [qr] content")?;
        declared(&qr)?;
        let mut lines = Vec::new();
        for line in raw.lines {
            ensure!(
                line.size_mm.is_finite() && (2.0..=20.0).contains(&line.size_mm),
                "Text size_mm must be between 2 and 20, not {}",
                line.size_mm
            );
            let content = interpolate::parse(&line.content).context("In [[text]] content")?;
            declared(&content)?;
            lines.push(TextLine {
                content,
                size_mm: line.size_mm,
                bold: line.bold,
                align: line.align,
            });
        }

        Ok(Self {
            name: raw.name,
            media,
            fields,
            qr,
            error_correction: raw.qr.error_correction,
            lines,
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

    /// The text a placeholder stands for in checked `values`: a group the
    /// value's match didn't take part in is empty.
    fn resolve(&self, values: &Values, placeholder: &Placeholder) -> String {
        let value = &values[&placeholder.field];
        let Some(group) = &placeholder.group else {
            return value.clone();
        };
        self.fields
            .iter()
            .find(|f| f.id == placeholder.field)
            .and_then(|f| f.pattern.as_ref())
            .and_then(|p| p.captures(value))
            .and_then(|c| c.name(group))
            .map_or_else(String::new, |m| m.as_str().to_owned())
    }

    /// The gap labels this template is drawn for.
    pub fn media(&self) -> &Media {
        &self.media
    }

    /// Label size in millimetres; `parse` admits only gap media.
    fn size_mm(&self) -> (f64, f64) {
        match self.media {
            Media::Gap {
                width_mm,
                height_mm,
            } => (width_mm, height_mm),
            _ => unreachable!("templates are parsed with gap media only"),
        }
    }

    /// Size in dots of the area `render` draws.
    pub fn printable_size(&self) -> (u32, u32) {
        let (width_mm, height_mm) = self.size_mm();
        rongta::image_processing::printable_size(width_mm, height_mm)
    }
}
