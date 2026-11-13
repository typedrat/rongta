//! Bluetooth work on its own thread, so the window keeps drawing while a scan
//! or print takes seconds. Discovery futures aren't `Send`, so the thread runs
//! a current-thread runtime and blocks on one request at a time.

use crate::errors::{self, Problem};
use anyhow::Result;
use eframe::egui;
use image::RgbaImage;
use rongta::{
    bluetooth::{DeviceFilter, ScanDevice},
    image_processing::{self, Layout},
    media::Media,
    monochrome::Conversion,
    printing::PrintOptions,
    raster::Raster,
};
use rongta_host::{
    bluetooth::{Address, SystemDiscovery},
    job::{self, Stage, Target},
};
use rongta_template::{Template, Values};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

const SCAN_TIME: Duration = Duration::from_secs(8);
const PRINT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub enum Job {
    Template {
        template: Template,
        values: Values,
    },
    Image {
        source: Arc<RgbaImage>,
        media: Media,
        layout: Layout,
        conversion: Conversion,
    },
}

impl Job {
    pub fn raster(&self, head_width: u32) -> Result<Raster> {
        match self {
            Job::Template { template, values } => template.raster(values, head_width),
            Job::Image {
                source,
                media,
                layout,
                conversion,
            } => image_processing::prepare(source, media, head_width, *layout, None, *conversion),
        }
    }
}

pub struct PrintRequest {
    pub address: Address,
    pub copies: u16,
    pub density: u8,
    pub job: Job,
}

pub enum Request {
    Scan,
    Print(PrintRequest),
}

pub enum Event {
    Scanned(Result<Vec<ScanDevice<Address>>, Problem>),
    Stage(Stage),
    Printed(Result<(), Problem>),
}

pub struct Worker {
    requests: mpsc::Sender<Request>,
    events: mpsc::Receiver<Event>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (requests, incoming) = mpsc::channel();
        let (outgoing, events) = mpsc::channel();
        std::thread::Builder::new()
            .name("bluetooth".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("building the Bluetooth runtime");
                let send = |event| {
                    let _ = outgoing.send(event);
                    ctx.request_repaint();
                };
                for request in incoming {
                    match request {
                        Request::Scan => {
                            let result = contain(|| runtime.block_on(scan()));
                            send(Event::Scanned(result));
                        }
                        Request::Print(request) => {
                            let result = contain(|| {
                                runtime.block_on(print(request, |stage| send(Event::Stage(stage))))
                            });
                            send(Event::Printed(result));
                        }
                    }
                }
            })
            .expect("starting the Bluetooth thread");
        Self { requests, events }
    }

    /// A worker with no thread behind it, for drawing UI without Bluetooth.
    #[cfg(test)]
    pub fn detached() -> Self {
        let (requests, _) = mpsc::channel();
        let (_, events) = mpsc::channel();
        Self { requests, events }
    }

    /// Fails only if the thread has died, which `contain` should prevent; the
    /// caller shows the problem rather than waiting for an event that will
    /// never come.
    pub fn send(&self, request: Request) -> Result<(), Problem> {
        self.requests.send(request).map_err(|_| Problem {
            summary: "The app lost its connection to Bluetooth. Close the app and open it again."
                .into(),
            details: "The Bluetooth thread has stopped".into(),
        })
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

/// Runs one request's work, turning a panic into a `Problem` so the thread
/// keeps serving requests and the UI is never left waiting for an event.
fn contain<T>(work: impl FnOnce() -> Result<T>) -> Result<T, Problem> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(result) => result.map_err(|e| Problem::from(&e)),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_owned());
            // Panic text is a programmer's message; matching it against known
            // printer failures would give a misleading summary.
            Err(Problem {
                summary: errors::UNEXPECTED.into(),
                details: message,
            })
        }
    }
}

async fn scan() -> Result<Vec<ScanDevice<Address>>> {
    let mut discovery = SystemDiscovery { adapter_name: None };
    rongta::bluetooth::scan(&mut discovery, SCAN_TIME, DeviceFilter::Printers).await
}

async fn print(request: PrintRequest, progress: impl FnMut(Stage)) -> Result<()> {
    job::print(
        Target {
            address: request.address,
            adapter: None,
            pin: None,
            timeout: PRINT_TIMEOUT,
        },
        |info| request.job.raster(info.model.width_dots),
        PrintOptions {
            copies: request.copies,
            density: request.density,
            timeout: PRINT_TIMEOUT,
            ..Default::default()
        },
        progress,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_a_problem_with_its_message() {
        // The default hook still prints the panic to stderr; that noise is
        // harmless and swapping the process-wide hook would race other tests.
        let problem = contain::<()>(|| panic!("raster blew up")).unwrap_err();
        assert!(problem.summary.starts_with("Something went wrong."));
        assert_eq!(problem.details, "raster blew up");

        let problem = contain::<()>(|| panic!("{}", String::from("owned message"))).unwrap_err();
        assert_eq!(problem.details, "owned message");

        let problem = contain::<()>(|| std::panic::panic_any(7u8)).unwrap_err();
        assert_eq!(problem.details, "unknown panic");
    }

    #[test]
    fn panic_text_is_not_mistaken_for_a_known_failure() {
        let problem = contain::<()>(|| panic!("Timed out connecting")).unwrap_err();
        assert!(problem.summary.starts_with("Something went wrong."));
        assert_eq!(problem.details, "Timed out connecting");
    }

    #[test]
    fn sending_to_a_stopped_thread_is_a_problem() {
        let problem = Worker::detached().send(Request::Scan).unwrap_err();
        assert!(
            problem.summary.contains("Close the app"),
            "{}",
            problem.summary
        );
    }

    #[test]
    fn results_pass_through_contain() {
        assert_eq!(contain(|| Ok(3)).unwrap(), 3);
        let problem = contain::<()>(|| Err(anyhow::anyhow!("something unexpected"))).unwrap_err();
        assert_eq!(problem.details, "something unexpected");
    }
}
