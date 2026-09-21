//! Persisted printer selection and file/environment overrides.
use anyhow::{Context, Result, bail, ensure};
use confique::{Config, Layer};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    #[default]
    R22,
}
impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("r22")
    }
}

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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Config)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[config(nested)]
    pub printer: PrinterSettings,
}

/// All selections belonging to one printer, reusable for future named printers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Config)]
#[serde(deny_unknown_fields)]
pub struct PrinterSettings {
    /// Selected Bluetooth printer address.
    #[config(env = "RONGTA_ADDRESS")]
    pub address: Option<bluer::Address>,
    /// Printer model (currently r22).
    #[config(env = "RONGTA_MODEL", default = "r22")]
    pub model: Model,
    /// Structured loaded media. CLI/environment syntax: gap:50x30, circle:40, continuous:50.
    #[config(env = "RONGTA_MEDIA", parse_env = parse_media)]
    pub media: Option<Media>,
}
fn parse_media(s: &str) -> std::io::Result<Media> {
    s.parse().map_err(|e: anyhow::Error| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string())
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    printer: PrinterSettings,
}

// Compatibility is restricted to old saved state; config files use the new schema.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyState {
    version: u32,
    selection: LegacyPrinterSettings,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPrinterSettings {
    address: Option<bluer::Address>,
    model: Model,
    media: Option<String>,
}

fn read_state(text: &str) -> Result<Settings> {
    #[derive(Deserialize)]
    struct Version {
        version: u32,
    }
    match toml::from_str::<Version>(text)?.version {
        1 => {
            let legacy: LegacyState = toml::from_str(text)?;
            ensure!(legacy.version == 1, "Invalid legacy state version");
            Ok(Settings {
                printer: PrinterSettings {
                    address: legacy.selection.address,
                    model: legacy.selection.model,
                    media: legacy
                        .selection
                        .media
                        .as_deref()
                        .map(str::parse)
                        .transpose()?,
                },
            })
        }
        2 => {
            let state: State = toml::from_str(text)?;
            Ok(Settings {
                printer: state.printer,
            })
        }
        version => bail!("Unsupported state version {version}"),
    }
}

pub struct Storage {
    pub state_path: PathBuf,
    pub config_path: PathBuf,
    explicit_config: bool,
}
impl Storage {
    pub fn discover(config: Option<PathBuf>) -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "rongta")
            .context("Cannot locate user storage directories")?;
        let config = config.or_else(|| std::env::var_os("RONGTA_CONFIG").map(PathBuf::from));
        Ok(Self {
            state_path: dirs
                .state_dir()
                .unwrap_or_else(|| dirs.data_local_dir())
                .join("state.toml"),
            explicit_config: config.is_some(),
            config_path: config.unwrap_or_else(|| dirs.config_dir().join("config.toml")),
        })
    }
    pub fn saved(&self) -> Result<Settings> {
        match fs::read_to_string(&self.state_path) {
            Ok(text) => read_state(&text)
                .with_context(|| format!("Invalid state file {}", self.state_path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e).with_context(|| format!("Cannot read {}", self.state_path.display())),
        }
    }
    pub fn effective(&self) -> Result<Settings> {
        let saved = self.saved()?;
        let mut file = confique::File::new(&self.config_path)?;
        if self.explicit_config {
            file = file.required();
        }
        let overrides = <Settings as Config>::Layer::from_env()?.with_fallback(file.load()?);
        let mut base = <Settings as Config>::Layer::empty();
        base.printer.address = saved.printer.address;
        base.printer.model = Some(saved.printer.model);
        base.printer.media = saved.printer.media;
        // A different printer must not inherit the old one's media.
        if overrides.printer.address.is_some() && overrides.printer.address != saved.printer.address
        {
            base.printer.media = None;
        }
        Settings::builder()
            .preloaded(overrides)
            .preloaded(base)
            .load()
            .context("Cannot load printer settings")
    }
    pub fn update(&self, change: impl FnOnce(&mut Settings) -> Result<()>) -> Result<Settings> {
        let parent = self
            .state_path
            .parent()
            .context("State path has no parent")?;
        fs::create_dir_all(parent)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(parent.join("state.lock"))?;
        lock.lock()?;
        let mut saved = self.saved()?;
        change(&mut saved)?;
        let text = toml::to_string_pretty(&State {
            version: 2,
            printer: saved.printer.clone(),
        })?;
        atomic_write(&self.state_path, text.as_bytes())?;
        Ok(saved)
    }
    pub fn save_menu(&self, original: &Settings, edited: &Settings) -> Result<()> {
        self.update(|current| {
            ensure!(
                current == original,
                "Settings changed in another process; reopen the menu before saving"
            );
            *current = edited.clone();
            Ok(())
        })?;
        Ok(())
    }
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("State path has no parent")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_updates_preserve_fields_and_stale_menu_cannot_overwrite_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = Storage {
            state_path: dir.path().join("state.toml"),
            config_path: dir.path().join("config.toml"),
            explicit_config: false,
        };
        let original = store.saved().unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                store
                    .update(|s| {
                        s.printer.address = Some("AA:BB:CC:DD:EE:FF".parse().unwrap());
                        Ok(())
                    })
                    .unwrap();
            });
            scope.spawn(|| {
                barrier.wait();
                store
                    .update(|s| {
                        s.printer.media = Some("gap:50x30".parse().unwrap());
                        Ok(())
                    })
                    .unwrap();
            });
        });
        let saved = store.saved().unwrap();
        assert!(saved.printer.address.is_some());
        assert!(saved.printer.media.is_some());
        assert!(store.save_menu(&original, &original).is_err());
        assert_eq!(store.saved().unwrap(), saved);
    }
}
