//! Errors phrased for the person at the printer, with the technical chain
//! kept for anyone helping them.
#![allow(dead_code)]

#[derive(Clone, Debug)]
pub struct Problem {
    pub summary: String,
    pub details: String,
}

impl From<&anyhow::Error> for Problem {
    fn from(error: &anyhow::Error) -> Self {
        let details = format!("{error:#}");
        let summary = if details.starts_with("Print job failed") {
            "The print may not have finished. Check the printer and the label before printing again."
        } else if details.contains("Timed out") && details.contains("connecting") {
            "Couldn't reach the printer. Make sure it's switched on and close by, then try again."
        } else if details.contains("Timed out") {
            "The printer stopped responding. Switch it off and on again, then try again."
        } else if details.contains("BlueZ")
            || details.contains("adapter")
            || details.contains("Winsock")
        {
            "Bluetooth isn't available. Check that it's turned on in your computer's settings."
        } else if details.contains("Pairing failed") {
            "Couldn't pair with the printer. Pair it in your computer's Bluetooth settings, then try again."
        } else {
            "Something went wrong. The details below may help whoever set this up."
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
            (anyhow!("something unexpected"), "Something went wrong"),
        ] {
            let problem = Problem::from(&error);
            assert!(problem.summary.contains(needle), "{:?}", problem.summary);
            assert_eq!(problem.details, format!("{error:#}"));
        }
    }
}
