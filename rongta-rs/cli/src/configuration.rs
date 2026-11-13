//! Persisted printer selection and file/environment overrides.
use anyhow::{Context, Result, bail, ensure};
use confique::{Config, Layer};
use directories::ProjectDirs;
use rongta::media::Media;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
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
    pub address: Option<crate::bluetooth::Address>,
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
    address: Option<crate::bluetooth::Address>,
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
