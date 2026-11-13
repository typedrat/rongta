//! Bluetooth work on its own thread, so the window keeps drawing while a scan
//! or print takes seconds. Discovery futures aren't `Send`, so the thread runs
//! a current-thread runtime and blocks on one request at a time.
#![allow(dead_code)]

use crate::errors::Problem;
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
                            let result = runtime.block_on(scan());
                            send(Event::Scanned(result.map_err(|e| Problem::from(&e))));
                        }
                        Request::Print(request) => {
                            let result =
                                runtime.block_on(print(request, |stage| send(Event::Stage(stage))));
                            send(Event::Printed(result.map_err(|e| Problem::from(&e))));
                        }
                    }
                }
            })
            .expect("starting the Bluetooth thread");
        Self { requests, events }
    }

    pub fn send(&self, request: Request) {
        // The thread only stops when this sender is dropped.
        let _ = self.requests.send(request);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
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
