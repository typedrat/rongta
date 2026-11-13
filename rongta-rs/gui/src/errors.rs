//! Errors phrased for the person at the printer, with the technical chain
//! kept for anyone helping them.
use crate::theme;
use eframe::egui;

/// The summary for failures nobody anticipated.
pub const UNEXPECTED: &str =
    "Something went wrong. The details below may help whoever set this up.";

#[derive(Clone, Debug)]
pub struct Problem {
    pub summary: String,
    pub details: String,
}

impl Problem {
    /// A problem whose summary the caller already knows, keeping `error`'s
    /// whole chain as the details.
    pub fn new(summary: impl Into<String>, error: &anyhow::Error) -> Self {
        Self {
            summary: summary.into(),
            details: format!("{error:#}"),
        }
    }
}

/// The summary in the error colour, with the technical chain folded away
/// under "Details" so it doesn't alarm the person printing.
pub fn problem_ui(ui: &mut egui::Ui, problem: &Problem) {
    let p = theme::palette(ui);
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(&problem.summary).color(p.error));
        ui.collapsing("Details", |ui| {
            ui.label(egui::RichText::new(&problem.details).monospace());
        });
    });
}

impl From<&anyhow::Error> for Problem {
    fn from(error: &anyhow::Error) -> Self {
        let details = format!("{error:#}");
        let summary = if details.starts_with("Print job failed") {
            "The print may not have finished. Check the printer and the label before printing again."
        } else if details.contains("Timed out")
            && (details.contains("connecting")
                || details.contains("pairing")
                || details.contains("SDP"))
        {
            "Couldn't reach the printer. Make sure it's switched on and close by, then try again."
        } else if details.contains("Timed out") {
            "The printer stopped responding. Switch it off and on again, then try again."
        } else if details.contains("Pairing failed") {
            "Couldn't pair with the printer. Pair it in your computer's Bluetooth settings, then try again."
        } else if details.contains("BlueZ")
            || details.contains("adapter")
            || details.contains("Winsock")
        {
            "Bluetooth isn't available. Check that it's turned on in your computer's settings."
        } else {
            UNEXPECTED
        };
        Self {
            summary: summary.to_owned(),
            details,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn known_failures_get_plain_summaries_and_keep_details() {
        for (error, needle) in [
            (
                anyhow!("Timed out after 60s discovering/pairing/connecting AA:BB:CC:DD:EE:FF"),
                "Couldn't reach the printer",
            ),
            (
                anyhow!("Timed out after 60s pairing/connecting AA:BB:CC:DD:EE:FF"),
                "Couldn't reach the printer",
            ),
            (
                anyhow!("status timeout").context(
                    "Print job failed; any bytes already sent cannot be recalled; check the printer before retrying",
                ),
                "may not have finished",
            ),
            (
                anyhow!("No BlueZ adapter; stop bt-bridge and restore host Bluetooth"),
                "Bluetooth",
            ),
            (
                anyhow!("no adapter bonded").context("Pairing failed; check the PIN"),
                "Couldn't pair",
            ),
            (
                anyhow!("Timed out pairing with the printer"),
                "Couldn't reach the printer",
            ),
            (
                anyhow!("Timed out waiting for the printer's SDP records"),
                "Couldn't reach the printer",
            ),
            (
                anyhow!("Timed out after 60s waiting for status"),
                "stopped responding",
            ),
            (anyhow!("something unexpected"), "Something went wrong"),
        ] {
            let problem = Problem::from(&error);
            assert!(problem.summary.contains(needle), "{:?}", problem.summary);
            assert_eq!(problem.details, format!("{error:#}"));
        }
    }
}
