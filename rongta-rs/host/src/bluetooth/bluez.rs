//! BlueZ-managed Classic Bluetooth SPP connection, with SDP channel resolution.
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use bluer::{
    AdapterEvent, Address, DiscoveryFilter, DiscoveryTransport, Session, Uuid,
    agent::{Agent, AgentHandle},
    rfcomm::{Profile, ProfileHandle, Role, Stream},
};
use futures_util::StreamExt;

use rongta::bluetooth::{Discovery, ScanDevice};

const SPP_UUID: Uuid = Uuid::from_u128(rongta::bluetooth::SPP_UUID);

/// Keep the profile and agent registered until the stream is closed.
/// Dropping the profile immediately after connecting would unregister SPP.
pub struct Connection {
    pub stream: Stream,
    _profile: ProfileHandle,
    _agent: Option<AgentHandle>,
    _session: Session,
}

impl Connection {
    /// Discover only as needed, pair through BlueZ, and connect the selected MAC.
    /// No adapter power or system service settings are changed.
    pub async fn connect(
        address: super::Address,
        adapter: Option<&str>,
        pin: Option<String>,
        timeout: Duration,
    ) -> Result<Self> {
        let address = Address(address.0);
        tokio::time::timeout(timeout, Self::connect_inner(address, adapter, pin))
            .await
            .with_context(|| {
                format!("Timed out after {timeout:?} discovering/pairing/connecting {address}")
            })?
    }

    async fn connect_inner(
        address: Address,
        adapter: Option<&str>,
        pin: Option<String>,
    ) -> Result<Self> {
        let session = Session::new()
            .await
            .context("Connecting to BlueZ; stop bt-bridge and start bluetooth.service first")?;
        let adapter = match adapter {
            Some(name) => session.adapter(name)?,
            None => session
                .default_adapter()
                .await
                .context("No BlueZ adapter; stop bt-bridge and restore host Bluetooth")?,
        };
        ensure!(
            adapter.is_powered().await?,
            "Adapter {} is powered off; power it on with bluetoothctl",
            adapter.name()
        );
        if !adapter.device_addresses().await?.contains(&address) {
            tracing::info!(%address, adapter = adapter.name(), "Discovering printer");
            adapter
                .set_discovery_filter(DiscoveryFilter {
                    transport: DiscoveryTransport::BrEdr,
                    ..Default::default()
                })
                .await?;
            let events = adapter
                .discover_devices()
                .await
                .context("Starting Classic Bluetooth discovery")?;
            tokio::pin!(events);
            loop {
                match events.next().await {
                    Some(AdapterEvent::DeviceAdded(found)) if found == address => break,
                    Some(_) => {}
                    None => bail!("Bluetooth discovery stopped before finding {address}"),
                }
            }
        }
        let device = adapter.device(address)?;
        // With no explicit PIN, BlueZ uses the user's existing default agent.
        let agent = if let Some(pin) = pin {
            ensure!(
                (1..=16).contains(&pin.len()) && pin.is_ascii() && !pin.contains('\0'),
                "PIN must contain 1–16 ASCII characters without NUL"
            );
            let adapter_name = adapter.name().to_owned();
            Some(
                session
                    .register_agent(Agent {
                        request_pin_code: Some(Box::new(move |request| {
                            let pin = pin.clone();
                            let matches =
                                request.device == address && request.adapter == adapter_name;
                            Box::pin(async move {
                                if matches {
                                    Ok(pin)
                                } else {
                                    Err(bluer::agent::ReqError::Rejected)
                                }
                            })
                        })),
                        ..Default::default()
                    })
                    .await
                    .context("Registering PIN agent")?,
            )
        } else {
            None
        };
        if !device.is_paired().await? {
            tracing::info!(%address, "Pairing printer through BlueZ");
            device.pair().await.context(
                "Pairing failed; pair in bluetoothctl or supply the printer's PIN with --pin",
            )?;
        }
        let mut profile = session
            .register_profile(Profile {
                uuid: SPP_UUID,
                name: Some("Rongta R22 CLI".into()),
                role: Some(Role::Client),
                require_authentication: Some(true),
                auto_connect: Some(false),
                ..Default::default()
            })
            .await
            .context("Registering SPP client profile")?;
        tracing::info!(%address, "Connecting SPP (channel resolved by BlueZ/SDP)");
        // ConnectProfile can wait for our NewConnection acknowledgement. Poll
        // both futures together, rather than waiting for ConnectProfile first.
        let (_, stream) = tokio::try_join!(
            async {
                device
                    .connect_profile(&SPP_UUID)
                    .await
                    .context("Connecting printer SPP service")
            },
            async {
                while let Some(request) = profile.next().await {
                    if request.device() == address {
                        return request.accept().context("Accepting SPP socket");
                    }
                    request.reject(bluer::rfcomm::ReqError::Rejected);
                }
                bail!("SPP profile closed before receiving a connection")
            }
        )?;
        Ok(Self {
            stream,
            _profile: profile,
            _agent: agent,
            _session: session,
        })
    }
}

/// Linux implementation of the portable discovery interface.
pub struct SystemDiscovery<'a> {
    pub adapter_name: Option<&'a str>,
}

impl Discovery for SystemDiscovery<'_> {
    type DeviceId = super::Address;

    async fn discover(&mut self, duration: Duration) -> Result<Vec<ScanDevice<super::Address>>> {
        let devices = scan(self.adapter_name, duration).await?;
        Ok(devices
            .into_iter()
            .map(|device| ScanDevice {
                address: super::Address(device.address.0),
                name: device.name,
                rssi: device.rssi,
                class: device.class,
                paired: device.paired,
                connected: device.connected,
                spp: device.spp,
                known_before_scan: device.known_before_scan,
            })
            .collect())
    }
}

/// Discover on both supported transports without pairing or connecting.
async fn scan(adapter_name: Option<&str>, duration: Duration) -> Result<Vec<ScanDevice<Address>>> {
    ensure!(!duration.is_zero(), "Scan duration must be nonzero");
    let session = Session::new()
        .await
        .context("Connecting to BlueZ; stop bt-bridge and start bluetooth.service first")?;
    let adapter = match adapter_name {
        Some(name) => session.adapter(name)?,
        None => session
            .default_adapter()
            .await
            .context("No BlueZ adapter; stop bt-bridge and restore host Bluetooth")?,
    };
    ensure!(
        adapter.is_powered().await?,
        "Adapter {} is powered off; power it on with bluetoothctl",
        adapter.name()
    );
    let known: std::collections::BTreeSet<_> =
        adapter.device_addresses().await?.into_iter().collect();
    adapter
        .set_discovery_filter(DiscoveryFilter {
            transport: DiscoveryTransport::Auto,
            ..Default::default()
        })
        .await?;
    let events = adapter
        .discover_devices()
        .await
        .context("Starting Bluetooth discovery")?;
    tokio::pin!(events);
    tracing::info!(
        adapter = adapter.name(),
        seconds = duration.as_secs(),
        "Scanning Bluetooth devices (including BlueZ cache)"
    );
    let addresses = scan_addresses(&mut events, duration).await?;
    // Read properties at the end so inquiry has time to resolve device names.
    // Keep discovery alive while reading RSSI; stopping it can clear that property.
    let mut pending = futures_util::stream::iter(addresses.into_iter().map(|address| {
        let adapter = &adapter;
        let known = &known;
        async move {
            let device = adapter.device(address)?;
            let result = tokio::try_join!(
                device.name(),
                device.rssi(),
                device.uuids(),
                device.is_paired(),
                device.is_connected(),
                device.class()
            );
            match result {
                Ok((name, rssi, uuids, paired, connected, class)) => Ok(Some(ScanDevice {
                    address,
                    name,
                    rssi,
                    class,
                    paired,
                    connected,
                    spp: uuids.map(|uuids| uuids.contains(&SPP_UUID)),
                    known_before_scan: known.contains(&address),
                })),
                Err(error)
                    if matches!(
                        error.kind,
                        bluer::ErrorKind::NotFound | bluer::ErrorKind::DoesNotExist
                    ) =>
                {
                    Ok(None)
                }
                Err(error) => {
                    Err(error).with_context(|| format!("Reading discovered device {address}"))
                }
            }
        }
    }))
    .buffer_unordered(8);
    let mut devices = vec![];
    while let Some(result) = pending.next().await {
        if let Some(device) = result? {
            devices.push(device);
        }
    }
    devices.sort_by_key(|device| device.address);
    Ok(devices)
}

async fn scan_addresses(
    mut events: impl futures_util::Stream<Item = AdapterEvent> + Unpin,
    duration: Duration,
) -> Result<std::collections::BTreeSet<Address>> {
    let mut addresses = std::collections::BTreeSet::new();
    let deadline = tokio::time::sleep(duration);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _ = &mut deadline => return Ok(addresses),
            event = events.next() => match event {
                Some(AdapterEvent::DeviceAdded(address)) => { addresses.insert(address); },
                Some(AdapterEvent::DeviceRemoved(address)) => { addresses.remove(&address); },
                Some(AdapterEvent::PropertyChanged(bluer::AdapterProperty::Powered(false))) => bail!("Bluetooth adapter powered off during discovery"),
                Some(_) => {},
                None => bail!("Bluetooth discovery stopped before the scan completed"),
            },
        }
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test(start_paused = true)]
    async fn deduplicates_and_removes_devices_until_deadline() {
        let a: Address = "AA:BB:CC:DD:EE:01".parse().unwrap();
        let b: Address = "AA:BB:CC:DD:EE:02".parse().unwrap();
        let events = stream::iter([
            AdapterEvent::DeviceAdded(a),
            AdapterEvent::DeviceAdded(a),
            AdapterEvent::DeviceAdded(b),
            AdapterEvent::DeviceRemoved(a),
        ])
        .chain(stream::pending());
        let found = scan_addresses(events, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(found.into_iter().collect::<Vec<_>>(), vec![b]);
    }

    #[tokio::test]
    async fn premature_discovery_end_or_power_loss_is_an_error() {
        assert!(
            scan_addresses(stream::empty(), Duration::from_secs(10))
                .await
                .is_err()
        );
        let events = stream::iter([AdapterEvent::PropertyChanged(
            bluer::AdapterProperty::Powered(false),
        )]);
        assert!(
            scan_addresses(events, Duration::from_secs(10))
                .await
                .unwrap_err()
                .to_string()
                .contains("powered off")
        );
    }
}
