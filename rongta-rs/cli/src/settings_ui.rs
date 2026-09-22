use crate::{
    bluetooth,
    configuration::{Model, Settings, Storage},
};
use anyhow::{Context, Result, ensure};
use dialoguer::{Input, Select, theme::ColorfulTheme};
use rongta::bluetooth::DeviceFilter;
use rongta::media::Media;
use std::{
    io::{self, IsTerminal},
    time::Duration,
};

pub async fn menu(storage: &Storage) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stderr().is_terminal(),
        "Settings menu requires a terminal; use settings --address MAC --model r22 --media gap:50x30, or status --json"
    );
    let original = storage.saved()?;
    let effective = storage.effective()?;
    let mut edited = original.clone();
    let theme = ColorfulTheme::default();
    eprintln!("Saved selection: {}", storage.state_path.display());
    eprintln!("Config overrides: {}", storage.config_path.display());
    if effective != original {
        eprintln!(
            "File/environment overrides are active. This menu edits saved selections; overrides remain in effect."
        );
        eprintln!(
            "Effective: {} / {} / {}",
            address_text(&effective),
            effective.printer.model,
            media_text(&effective)
        );
    }
    loop {
        let items = [
            format!("Printer: {}", address_text(&edited)),
            format!("Model: {}", edited.printer.model),
            format!("Media: {}", media_text(&edited)),
            "Save and exit".into(),
            "Cancel".into(),
        ];
        let choice = Select::with_theme(&theme)
            .with_prompt("Rongta settings")
            .items(&items)
            .default(0)
            .interact_opt()?;
        match choice {
            Some(0) => {
                let method = Select::with_theme(&theme)
                    .with_prompt("Select printer")
                    .items(["Scan for printers", "Enter MAC address", "Back"])
                    .default(0)
                    .interact_opt()?;
                let address = match method {
                    Some(0) => {
                        eprintln!("Scanning for printers (10 seconds)…");
                        let mut discovery = bluetooth::BlueZDiscovery { adapter_name: None };
                        match rongta::bluetooth::scan(
                            &mut discovery,
                            Duration::from_secs(10),
                            DeviceFilter::Printers,
                        )
                        .await
                        {
                            Ok(devices) => {
                                if devices.is_empty() {
                                    eprintln!(
                                        "No printer candidates found. You can enter a MAC address manually."
                                    );
                                    continue;
                                }
                                let labels: Vec<_> = devices
                                    .iter()
                                    .map(|device| {
                                        format!(
                                            "{}  {}",
                                            device.address,
                                            device
                                                .name
                                                .as_deref()
                                                .unwrap_or("Unnamed")
                                                .chars()
                                                .map(|c| if c.is_control() { ' ' } else { c })
                                                .collect::<String>()
                                        )
                                    })
                                    .collect();
                                Select::with_theme(&theme)
                                    .with_prompt("Printer (cached devices may be offline)")
                                    .items(&labels)
                                    .default(0)
                                    .interact_opt()?
                                    .map(|i| devices[i].address)
                            }
                            Err(error) => {
                                eprintln!("Scan failed: {error:#}");
                                continue;
                            }
                        }
                    }
                    Some(1) => {
                        let value: String = Input::with_theme(&theme)
                            .with_prompt("Bluetooth MAC address")
                            .with_initial_text(
                                edited
                                    .printer
                                    .address
                                    .map(|a| a.to_string())
                                    .unwrap_or_default(),
                            )
                            .validate_with(|s: &String| {
                                s.parse::<bluer::Address>()
                                    .map(|_| ())
                                    .map_err(|_| "Use AA:BB:CC:DD:EE:FF")
                            })
                            .interact_text()?;
                        Some(value.parse().context("Invalid printer Bluetooth address")?)
                    }
                    _ => None,
                };
                if let Some(address) = address {
                    if edited.printer.address != Some(address) {
                        edited.printer.media = None;
                    }
                    edited.printer.address = Some(address);
                }
            }
            Some(1) => {
                if Select::with_theme(&theme)
                    .with_prompt("Printer model")
                    .items(["R22"])
                    .default(0)
                    .interact_opt()?
                    .is_some()
                {
                    edited.printer.model = Model::R22;
                }
            }
            Some(2) => {
                let presets = [
                    "gap:50x80",
                    "gap:50x50",
                    "gap:50x30",
                    "gap:40x60",
                    "gap:40x40",
                    "gap:40x30",
                    "gap:30x30",
                    "circle:50",
                    "circle:40",
                    "circle:30",
                    "continuous:50",
                    "continuous:40",
                    "continuous:30",
                    "Custom media",
                    "Clear media",
                    "Back",
                ];
                let default = edited
                    .printer
                    .media
                    .as_ref()
                    .and_then(|media| presets.iter().position(|p| *p == media.to_string()))
                    .unwrap_or(2);
                let labels: Vec<_> = presets
                    .iter()
                    .map(|preset| {
                        preset
                            .parse::<Media>()
                            .map(|media| describe_media(&media))
                            .unwrap_or_else(|_| preset.to_string())
                    })
                    .collect();
                match Select::with_theme(&theme)
                    .with_prompt("Loaded media (dimensions in mm)")
                    .items(&labels)
                    .default(default)
                    .interact_opt()?
                {
                    Some(i @ 0..=12) => edited.printer.media = Some(presets[i].parse()?),
                    Some(13) => {
                        let value: String = Input::with_theme(&theme)
                            .with_prompt("Media: gap:WxH, circle:D, continuous:W")
                            .validate_with(|s: &String| {
                                s.parse::<Media>().map(|_| ()).map_err(|e| e.to_string())
                            })
                            .interact_text()?;
                        edited.printer.media = Some(value.parse()?);
                    }
                    Some(14) => edited.printer.media = None,
                    _ => {}
                }
            }
            Some(3) => {
                storage.save_menu(&original, &edited)?;
                eprintln!("Saved settings to {}", storage.state_path.display());
                return Ok(());
            }
            _ => {
                eprintln!("Settings unchanged.");
                return Ok(());
            }
        }
    }
}
fn address_text(settings: &Settings) -> String {
    settings
        .printer
        .address
        .map(|a| a.to_string())
        .unwrap_or_else(|| "not selected".into())
}
fn media_text(settings: &Settings) -> String {
    settings
        .printer
        .media
        .as_ref()
        .map(describe_media)
        .unwrap_or_else(|| "not selected".into())
}

fn describe_media(media: &Media) -> String {
    match media {
        Media::Gap {
            width_mm,
            height_mm,
        } => format!("{width_mm} × {height_mm} mm gap labels"),
        Media::Circle { diameter_mm } => format!("{diameter_mm} mm circular labels"),
        Media::Continuous { width_mm } => format!("{width_mm} mm continuous stock"),
    }
}
