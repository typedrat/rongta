//! Portable media geometry and serialization.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// A single atomic value so overrides cannot mix dimensions from different media.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Media {
    Gap {
        #[serde(deserialize_with = "deserialize_width")]
        width_mm: f64,
        #[serde(deserialize_with = "deserialize_height")]
        height_mm: f64,
    },
    Circle {
        #[serde(deserialize_with = "deserialize_diameter")]
        diameter_mm: f64,
    },
    Continuous {
        #[serde(deserialize_with = "deserialize_width")]
        width_mm: f64,
    },
}
impl FromStr for Media {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        fn dimension(s: &str) -> Result<f64> {
            let value: f64 = s
                .parse()
                .context("Media dimensions must be numbers in mm")?;
            ensure!(
                value.is_finite() && value > 0.0,
                "Media dimensions must be finite and positive"
            );
            Ok(value)
        }
        let (kind, size) = s
            .split_once(':')
            .context("Use gap:50x30, circle:40, or continuous:50")?;
        let media = match kind {
            "gap" => {
                let (w, h) = size
                    .split_once('x')
                    .context("Gap media requires WIDTHxHEIGHT in mm")?;
                Self::Gap {
                    width_mm: dimension(w)?,
                    height_mm: dimension(h)?,
                }
            }
            "circle" => Self::Circle {
                diameter_mm: dimension(size)?,
            },
            "continuous" => Self::Continuous {
                width_mm: dimension(size)?,
            },
            _ => bail!("Unknown media type {kind}; use gap, circle, or continuous"),
        };
        validate_width(media.width_mm())?;
        if let Some(height) = media.height_mm() {
            validate_height(height)?;
        }
        Ok(media)
    }
}
fn validate_width(value: f64) -> Result<()> {
    ensure!(
        value.is_finite() && value > 0.0 && value <= 50.0,
        "R22 media width must be finite, positive, and at most 50 mm"
    );
    Ok(())
}
fn validate_height(value: f64) -> Result<()> {
    ensure!(
        value.is_finite() && value > 2.0 && value <= 1000.0,
        "R22 label height must be finite, greater than 2 mm and at most 1000 mm"
    );
    Ok(())
}
fn deserialize_width<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<f64, D::Error> {
    let value = f64::deserialize(deserializer)?;
    validate_width(value).map_err(serde::de::Error::custom)?;
    Ok(value)
}
fn deserialize_height<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<f64, D::Error> {
    let value = f64::deserialize(deserializer)?;
    validate_height(value).map_err(serde::de::Error::custom)?;
    Ok(value)
}
fn deserialize_diameter<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<f64, D::Error> {
    let value = deserialize_width(deserializer)?;
    validate_height(value).map_err(serde::de::Error::custom)?;
    Ok(value)
}
impl fmt::Display for Media {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gap {
                width_mm,
                height_mm,
            } => write!(f, "gap:{width_mm}x{height_mm}"),
            Self::Circle { diameter_mm } => write!(f, "circle:{diameter_mm}"),
            Self::Continuous { width_mm } => write!(f, "continuous:{width_mm}"),
        }
    }
}
impl Media {
    pub fn width_mm(&self) -> f64 {
        match *self {
            Self::Gap { width_mm, .. } | Self::Continuous { width_mm } => width_mm,
            Self::Circle { diameter_mm } => diameter_mm,
        }
    }
    pub fn height_mm(&self) -> Option<f64> {
        match *self {
            Self::Gap { height_mm, .. } => Some(height_mm),
            Self::Circle { diameter_mm } => Some(diameter_mm),
            Self::Continuous { .. } => None,
        }
    }
}
