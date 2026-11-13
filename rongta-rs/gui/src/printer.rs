//! Choosing the printer. The choice is saved where the CLI keeps it, so both
//! tools agree on which printer to use.
use crate::{
    errors::Problem,
    theme,
    worker::{Request, Worker},
};
use eframe::egui;
use rongta::{bluetooth::ScanDevice, media::Media};
use rongta_host::{bluetooth::Address, configuration::Storage};

pub struct Printer {
    storage: Option<Storage>,
    pub selected: Option<Address>,
    pub media: Option<Media>,
    devices: Vec<ScanDevice<Address>>,
    scanning: bool,
    problem: Option<Problem>,
}

impl Printer {
    pub fn load() -> Self {
        let mut printer = Self {
            storage: None,
            selected: None,
            media: None,
            devices: Vec::new(),
            scanning: false,
            problem: None,
        };
        match Storage::discover(None).and_then(|storage| Ok((storage.effective()?, storage))) {
            Ok((settings, storage)) => {
                printer.selected = settings.printer.address;
                printer.media = settings.printer.media;
                printer.storage = Some(storage);
            }
            Err(error) => printer.problem = Some(Problem::from(&error)),
        }
        printer
    }

    #[cfg(test)]
    pub fn for_test(
        devices: Vec<ScanDevice<Address>>,
        selected: Option<Address>,
        problem: Option<Problem>,
    ) -> Self {
        Self {
            storage: None,
            selected,
            media: None,
            devices,
            scanning: false,
            problem,
        }
    }

    pub fn label(device: &ScanDevice<Address>) -> String {
        format!(
            "{} ({})",
            device.name.as_deref().unwrap_or("Unnamed printer"),
            device.address
        )
    }

    pub fn scanned(&mut self, result: Result<Vec<ScanDevice<Address>>, Problem>) {
        self.scanning = false;
        match result {
            Ok(devices) => {
                if self.selected.is_none() && devices.len() == 1 {
                    self.select(devices[0].address);
                }
                self.devices = devices;
                self.problem = None;
            }
            Err(problem) => self.problem = Some(problem),
        }
    }

    fn select(&mut self, address: Address) {
        self.selected = Some(address);
        if let Some(storage) = &self.storage {
            let saved = storage.update(|settings| {
                if settings.printer.address != Some(address) {
                    // Loaded labels belong to the previous printer.
                    settings.printer.media = None;
                }
                settings.printer.address = Some(address);
                Ok(())
            });
            match saved {
                Ok(settings) => self.media = settings.printer.media,
                Err(error) => self.problem = Some(Problem::from(&error)),
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, worker: &Worker) {
        let p = theme::palette(ui);
        ui.horizontal(|ui| {
            let heading = ui.label(
                egui::RichText::new("Printer").font(egui::FontId::new(15.0, theme::semibold())),
            );
            let current = match self.selected {
                Some(address) => self
                    .devices
                    .iter()
                    .find(|d| d.address == address)
                    .map(Self::label)
                    .unwrap_or_else(|| address.to_string()),
                None => "None chosen".to_owned(),
            };
            let mut choice = self.selected;
            let combo = egui::ComboBox::from_id_salt("printer")
                .width(280.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if self.devices.is_empty() {
                        ui.label(egui::RichText::new("Press “Find printers” first").color(p.muted));
                    }
                    for device in &self.devices {
                        ui.selectable_value(&mut choice, Some(device.address), Self::label(device));
                    }
                });
            combo.response.labelled_by(heading.id);
            if let Some(address) = choice
                && choice != self.selected
            {
                self.select(address);
            }
            if ui
                .add_enabled(!self.scanning, egui::Button::new("Find printers"))
                .clicked()
            {
                self.scanning = true;
                worker.send(Request::Scan);
            }
            if self.scanning {
                ui.spinner();
                ui.label(egui::RichText::new("Searching…").color(p.muted));
            }
        });
        if let Some(problem) = &self.problem {
            ui.label(egui::RichText::new(&problem.summary).color(p.error));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_labelled_by_name_and_address() {
        let device = ScanDevice {
            address: "AA:BB:CC:DD:EE:FF".parse::<Address>().unwrap(),
            name: Some("R22_1A2B".into()),
            rssi: None,
            class: None,
            paired: true,
            connected: false,
            spp: Some(true),
            known_before_scan: true,
        };
        assert_eq!(Printer::label(&device), "R22_1A2B (AA:BB:CC:DD:EE:FF)");
        let unnamed = ScanDevice {
            name: None,
            ..device
        };
        assert_eq!(
            Printer::label(&unnamed),
            "Unnamed printer (AA:BB:CC:DD:EE:FF)"
        );
    }
}
