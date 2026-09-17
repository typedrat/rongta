//! BlueZ-managed Classic Bluetooth SPP connection, with SDP channel resolution.
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use bluer::{
    AdapterEvent, Address, DiscoveryFilter, DiscoveryTransport, Session, Uuid,
    agent::{Agent, AgentHandle},
    rfcomm::{Profile, ProfileHandle, Role, Stream},
};
use futures_util::StreamExt;

pub const SPP_UUID: Uuid = Uuid::from_u128(0x00001101_0000_1000_8000_00805f9b34fb);

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
        address: Address,
        adapter: Option<&str>,
        pin: Option<String>,
        timeout: Duration,
    ) -> Result<Self> {
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
