//! Choosing the printer. The choice is saved where the CLI keeps it, so both
//! tools agree on which printer to use.
use crate::{
    errors::{Problem, problem_ui},
    theme,
    worker::{Request, Worker},
};
use eframe::egui;
use rongta::{bluetooth::ScanDevice, media::Media};
use rongta_host::{bluetooth::Address, configuration::Storage};

const NO_PRINTERS: &str =
    "No printers found. Make sure the printer is switched on and close by, then try again.";

pub struct Printer {
    storage: Option<Storage>,
    pub selected: Option<Address>,
    pub media: Option<Media>,
    devices: Vec<ScanDevice<Address>>,
    scanning: bool,
    /// Whether a search has finished, so an empty list means "none found"
    /// rather than "not looked yet".
    searched: bool,
    problem: Option<Problem>,
}

impl Printer {
    pub fn load() -> Self {
        let mut printer = Self::empty();
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

    fn empty() -> Self {
        Self {
            storage: None,
            selected: None,
            media: None,
            devices: Vec::new(),
            scanning: false,
            searched: false,
            problem: None,
        }
    }

    #[cfg(test)]
    pub fn for_test(
        devices: Vec<ScanDevice<Address>>,
        selected: Option<Address>,
        problem: Option<Problem>,
    ) -> Self {
        Self {
            selected,
            devices,
            problem,
            ..Self::empty()
        }
    }

    pub fn label(device: &ScanDevice<Address>) -> String {
        format!(
            "{} ({})",
            device.name.as_deref().unwrap_or("Unnamed printer"),
            device.address
        )
    }

    fn current_label(&self) -> String {
        let Some(address) = self.selected else {
            return "None chosen".to_owned();
        };
        self.devices
            .iter()
            .find(|d| d.address == address)
            .map(Self::label)
            .unwrap_or_else(|| format!("Saved printer ({address})"))
    }

    pub fn scanning(&self) -> bool {
        self.scanning
    }

    /// Explains an empty list after a search that found nothing.
    fn notice(&self) -> Option<&'static str> {
        (self.searched && !self.scanning && self.devices.is_empty() && self.problem.is_none())
            .then_some(NO_PRINTERS)
    }

    pub fn scanned(&mut self, result: Result<Vec<ScanDevice<Address>>, Problem>) {
        self.scanning = false;
        self.searched = true;
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
                Err(error) => {
                    // The old printer's media may not match the new one.
                    self.media = None;
                    self.problem = Some(Problem::from(&error));
                }
            }
        }
    }

    /// `busy` is true while a print runs; searching then would compete with it
    /// for the Bluetooth adapter.
    pub fn ui(&mut self, ui: &mut egui::Ui, worker: &Worker, busy: bool) {
        let p = theme::palette(ui);
        ui.horizontal(|ui| {
            let heading = ui.label(
                egui::RichText::new("Printer").font(egui::FontId::new(15.0, theme::semibold())),
            );
            let mut choice = self.selected;
            let combo = egui::ComboBox::from_id_salt("printer")
                .width(280.0)
                .selected_text(self.current_label())
                .show_ui(ui, |ui| {
                    if self.devices.is_empty() {
                        let empty = if self.searched {
                            "No printers found"
                        } else {
                            "Press “Find printers” first"
                        };
                        ui.label(egui::RichText::new(empty).color(p.muted));
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
                .add_enabled(!self.scanning && !busy, egui::Button::new("Find printers"))
                .clicked()
            {
                match worker.send(Request::Scan) {
                    Ok(()) => self.scanning = true,
                    Err(problem) => self.problem = Some(problem),
                }
            }
            if self.scanning {
                ui.spinner();
                ui.label(egui::RichText::new("Searching…").color(p.muted));
            }
        });
        if let Some(problem) = &self.problem {
            problem_ui(ui, problem);
        } else if let Some(notice) = self.notice() {
            ui.label(egui::RichText::new(notice).color(p.muted));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_scan_says_so_until_the_next_search() {
        let mut printer = Printer::for_test(Vec::new(), None, None);
        assert_eq!(printer.notice(), None, "nothing searched yet");
        printer.scanned(Ok(Vec::new()));
        assert!(printer.notice().unwrap().starts_with("No printers found."));
        printer.scanning = true;
        assert_eq!(printer.notice(), None, "searching again");
        printer.scanned(Err(Problem {
            summary: "Bluetooth isn't available.".into(),
            details: String::new(),
        }));
        assert_eq!(printer.notice(), None, "the problem explains it instead");
    }

    #[test]
    fn a_saved_printer_missing_from_the_scan_is_named_as_saved() {
        let address = "AA:BB:CC:DD:EE:FF".parse::<Address>().unwrap();
        let printer = Printer::for_test(Vec::new(), Some(address), None);
        assert_eq!(printer.current_label(), "Saved printer (AA:BB:CC:DD:EE:FF)");
    }

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
