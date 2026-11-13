//! Windows Bluetooth Classic SPP over Winsock RFCOMM sockets.
//!
//! These Win32 APIs need no package identity or app capability, so they work
//! from a plain unpackaged executable. Sockets are blocking: the connection
//! lives on its own threads and is bridged to an async stream.
use std::{
    ffi::c_void,
    sync::{Arc, OnceLock},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use rongta::bluetooth::{Discovery, SPP_UUID, ScanDevice};
use tokio::{
    io::DuplexStream,
    sync::{mpsc, oneshot},
};
use windows::{
    Win32::{
        Devices::Bluetooth::{
            AF_BTH, BLUETOOTH_ADDRESS, BLUETOOTH_ADDRESS_0, BLUETOOTH_AUTHENTICATE_RESPONSE,
            BLUETOOTH_AUTHENTICATION_CALLBACK_PARAMS, BLUETOOTH_AUTHENTICATION_METHOD_LEGACY,
            BLUETOOTH_AUTHENTICATION_METHOD_NUMERIC_COMPARISON, BLUETOOTH_DEVICE_INFO,
            BLUETOOTH_DEVICE_SEARCH_PARAMS, BTHPROTO_RFCOMM, BluetoothAuthenticateDeviceEx,
            BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindNextDevice,
            BluetoothGetDeviceInfo, BluetoothRegisterForAuthenticationEx,
            BluetoothSendAuthenticationResponseEx, BluetoothUnregisterAuthentication,
            MITMProtectionNotRequiredGeneralBonding, SOCKADDR_BTH,
        },
        Foundation::{ERROR_NO_MORE_ITEMS, WIN32_ERROR},
        Networking::WinSock::{
            SD_BOTH, SEND_RECV_FLAGS, SOCK_STREAM, SOCKADDR, SOCKET, WSADATA, WSAStartup,
            closesocket, connect, recv, send, shutdown, socket,
        },
    },
    core::{BOOL, GUID},
};

use super::{Address, bridge, no_adapter_selection};

/// Keep the socket threads alive until the stream is closed. Dropping the
/// stream shuts the socket down.
pub struct Connection {
    pub stream: DuplexStream,
}

impl Connection {
    /// Pair if needed, then connect to the SPP service; Windows resolves the
    /// RFCOMM channel by SDP. Without --pin, Windows shows its pairing prompt.
    pub async fn connect(
        address: Address,
        adapter: Option<&str>,
        pin: Option<String>,
        timeout: Duration,
    ) -> Result<Self> {
        no_adapter_selection(adapter)?;
        let (stream, ends) = bridge::stream();
        let (ready_tx, ready_rx) = oneshot::channel();
        thread::Builder::new()
            .name("bluetooth".into())
            .spawn(move || match open(address, pin) {
                Ok(socket) => {
                    if ready_tx.send(Ok(())).is_ok() {
                        serve(Arc::new(socket), ends);
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .context("Starting Bluetooth thread")?;
        match tokio::time::timeout(timeout, ready_rx).await {
            Ok(Ok(result)) => result.map(|()| Self { stream }),
            Ok(Err(_)) => bail!("Bluetooth thread exited while connecting {address}"),
            Err(_) => bail!("Timed out after {timeout:?} pairing/connecting {address}"),
        }
    }
}

/// Windows implementation of the portable discovery interface.
pub struct SystemDiscovery<'a> {
    pub adapter_name: Option<&'a str>,
}

impl Discovery for SystemDiscovery<'_> {
    type DeviceId = Address;

    async fn discover(&mut self, duration: Duration) -> Result<Vec<ScanDevice<Address>>> {
        no_adapter_selection(self.adapter_name)?;
        ensure!(!duration.is_zero(), "Scan duration must be nonzero");
        tokio::task::spawn_blocking(move || scan(duration))
            .await
            .context("Bluetooth scan thread panicked")?
    }
}

struct Socket(SOCKET);

impl Drop for Socket {
    fn drop(&mut self) {
        unsafe { closesocket(self.0) };
    }
}

fn win32(code: u32) -> windows::core::Result<()> {
    WIN32_ERROR(code).to_hresult().ok()
}

fn winsock_startup() -> Result<()> {
    static STARTUP: OnceLock<i32> = OnceLock::new();
    let code = *STARTUP.get_or_init(|| {
        let mut data = WSADATA::default();
        unsafe { WSAStartup(0x0202, &mut data) }
    });
    win32(code as u32).context("Initializing Winsock")
}

fn bluetooth_address(address: Address) -> u64 {
    let [a, b, c, d, e, f] = address.0;
    u64::from_be_bytes([0, 0, a, b, c, d, e, f])
}

fn address_of(info: &BLUETOOTH_DEVICE_INFO) -> Address {
    let bytes = unsafe { info.Address.Anonymous.ullLong }.to_be_bytes();
    Address(bytes[2..].try_into().expect("six address bytes"))
}

fn device_info(address: Address) -> BLUETOOTH_DEVICE_INFO {
    BLUETOOTH_DEVICE_INFO {
        dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
        Address: BLUETOOTH_ADDRESS {
            Anonymous: BLUETOOTH_ADDRESS_0 {
                ullLong: bluetooth_address(address),
            },
        },
        ..Default::default()
    }
}

fn open(address: Address, pin: Option<String>) -> Result<Socket> {
    winsock_startup()?;
    let mut info = device_info(address);
    // An unknown device is not an error here; it simply is not paired yet.
    let known = win32(unsafe { BluetoothGetDeviceInfo(None, &mut info) }).is_ok();
    if !known || !info.fAuthenticated.as_bool() {
        tracing::info!(%address, "Pairing printer through Windows");
        pair(&mut info, pin)?;
    }

    tracing::info!(%address, "Connecting SPP (channel resolved by Windows/SDP)");
    let socket = Socket(
        unsafe { socket(AF_BTH.into(), SOCK_STREAM, BTHPROTO_RFCOMM as i32) }
            .context("Creating Bluetooth RFCOMM socket")?,
    );
    let target = SOCKADDR_BTH {
        addressFamily: AF_BTH,
        btAddr: bluetooth_address(address),
        serviceClassId: GUID::from_u128(SPP_UUID),
        port: 0,
    };
    let status = unsafe {
        connect(
            socket.0,
            (&raw const target).cast::<SOCKADDR>(),
            size_of::<SOCKADDR_BTH>() as i32,
        )
    };
    if status != 0 {
        return Err(windows::core::Error::from_thread()).context("Connecting printer SPP service");
    }
    Ok(socket)
}

fn pair(info: &mut BLUETOOTH_DEVICE_INFO, pin: Option<String>) -> Result<()> {
    let pin = pin.map(String::into_bytes);
    let mut registration = 0;
    if let Some(pin) = &pin {
        win32(unsafe {
            BluetoothRegisterForAuthenticationEx(
                Some(&raw const *info),
                &mut registration,
                Some(respond),
                Some((&raw const *pin).cast::<c_void>()),
            )
        })
        .context("Registering PIN pairing callback")?;
    }
    let status = unsafe {
        BluetoothAuthenticateDeviceEx(
            None,
            None,
            info,
            None,
            MITMProtectionNotRequiredGeneralBonding,
        )
    };
    if registration != 0 {
        // The callback borrows `pin`, so unregister before it is dropped.
        let _ = unsafe { BluetoothUnregisterAuthentication(registration) };
    }
    if status == ERROR_NO_MORE_ITEMS.0 {
        return Ok(()); // Already paired.
    }
    win32(status)
        .context("Pairing failed; pair in Windows Settings or supply the printer's PIN with --pin")
}

/// Answers a pairing request with the PIN passed in `param` (a `Vec<u8>`).
unsafe extern "system" fn respond(
    param: *const c_void,
    request: *const BLUETOOTH_AUTHENTICATION_CALLBACK_PARAMS,
) -> BOOL {
    // SAFETY: Windows passes back the registered parameter and a valid request.
    let (pin, request) = unsafe { (&*param.cast::<Vec<u8>>(), &*request) };
    let mut response = BLUETOOTH_AUTHENTICATE_RESPONSE {
        bthAddressRemote: request.deviceInfo.Address,
        authMethod: request.authenticationMethod,
        ..Default::default()
    };
    match request.authenticationMethod {
        BLUETOOTH_AUTHENTICATION_METHOD_LEGACY => {
            let info = unsafe { &mut response.Anonymous.pinInfo };
            info.pin[..pin.len()].copy_from_slice(pin);
            info.pinLength = pin.len() as u8;
        }
        // A printer has no display, so there is no value to compare; accept
        // confirmation ("just works") rather than failing the pairing.
        BLUETOOTH_AUTHENTICATION_METHOD_NUMERIC_COMPARISON => {
            let value = unsafe { request.Anonymous.Numeric_Value };
            tracing::warn!(
                numeric_value = value,
                "Accepting Secure Simple Pairing confirmation"
            );
            response.Anonymous.numericCompInfo.NumericValue = value;
        }
        method => {
            tracing::error!(method = method.0, "Unsupported pairing method requested");
            response.negativeResponse = 1;
        }
    }
    win32(unsafe { BluetoothSendAuthenticationResponseEx(None, &response) })
        .is_ok()
        .into()
}

fn serve(socket: Arc<Socket>, ends: bridge::Ends) {
    let reader_socket = socket.clone();
    let incoming = ends.incoming;
    let reader = thread::spawn(move || {
        let mut buffer = [0; 4096];
        loop {
            let n = unsafe { recv(reader_socket.0, &mut buffer, SEND_RECV_FLAGS(0)) };
            if n < 0 {
                let error = windows::core::Error::from_thread();
                tracing::error!("Reading from the printer failed: {error}");
            }
            if n <= 0 || incoming.send(buffer[..n as usize].to_vec()).is_err() {
                break;
            }
        }
        // Unblocks the writer's next send, if the printer went away first.
        unsafe { shutdown(reader_socket.0, SD_BOTH) };
    });
    let mut outgoing: mpsc::Receiver<Vec<u8>> = ends.outgoing;
    while let Some(data) = outgoing.blocking_recv() {
        if let Err(error) = send_all(&socket, &data) {
            tracing::error!("{error:#}");
            break;
        }
    }
    // Unblocks the reader's recv so both threads exit.
    unsafe { shutdown(socket.0, SD_BOTH) };
    let _ = reader.join();
}

fn send_all(socket: &Socket, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        let n = unsafe { send(socket.0, data, SEND_RECV_FLAGS(0)) };
        if n <= 0 {
            return Err(windows::core::Error::from_thread()).context("Writing to the printer");
        }
        data = &data[n as usize..];
    }
    Ok(())
}

/// Inquire for devices; Windows also returns devices it already remembers.
fn scan(duration: Duration) -> Result<Vec<ScanDevice<Address>>> {
    // Inquiry length is a multiple of 1.28 seconds, at most 48 (61.44 s).
    let multiplier = (duration.as_secs_f64() / 1.28).ceil().clamp(1.0, 48.0) as u8;
    if duration.as_secs_f64() > 61.44 {
        tracing::warn!("Windows limits a Bluetooth inquiry to 61 seconds");
    }
    let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
        dwSize: size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
        fReturnAuthenticated: true.into(),
        fReturnRemembered: true.into(),
        fReturnUnknown: true.into(),
        fReturnConnected: true.into(),
        fIssueInquiry: true.into(),
        cTimeoutMultiplier: multiplier,
        ..Default::default()
    };
    let blank = || BLUETOOTH_DEVICE_INFO {
        dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
        ..Default::default()
    };
    tracing::info!(
        seconds = f64::from(multiplier) * 1.28,
        "Scanning Bluetooth devices"
    );
    let mut info = blank();
    let find = match unsafe { BluetoothFindFirstDevice(&params, &mut info) } {
        Ok(find) => find,
        Err(error) if WIN32_ERROR::from_error(&error) == Some(ERROR_NO_MORE_ITEMS) => {
            return Ok(vec![]);
        }
        Err(error) => return Err(error).context("Scanning Bluetooth devices"),
    };
    let mut devices = vec![];
    loop {
        devices.push(describe(&info));
        info = blank();
        if unsafe { BluetoothFindNextDevice(find, &mut info) }.is_err() {
            break;
        }
    }
    let _ = unsafe { BluetoothFindDeviceClose(find) };
    devices.sort_by_key(|device| device.address);
    devices.dedup_by_key(|device| device.address);
    Ok(devices)
}

fn describe(info: &BLUETOOTH_DEVICE_INFO) -> ScanDevice<Address> {
    let name_length = info
        .szName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szName.len());
    let name = String::from_utf16_lossy(&info.szName[..name_length]);
    ScanDevice {
        address: address_of(info),
        name: (!name.is_empty()).then_some(name),
        rssi: None,
        class: (info.ulClassofDevice != 0).then_some(info.ulClassofDevice),
        paired: info.fAuthenticated.as_bool(),
        connected: info.fConnected.as_bool(),
        spp: None,
        known_before_scan: info.fRemembered.as_bool(),
    }
}
