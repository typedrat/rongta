//! macOS IOBluetooth Classic SPP connection and discovery.
//!
//! IOBluetooth delivers every callback on the run loop of the thread that
//! started the operation, so each scan or connection owns a dedicated thread
//! that runs its CFRunLoop. Callbacks land on a delegate object whose state is
//! only touched from that thread.
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    ffi::c_void,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use objc2::{
    AnyThread, DefinedClass, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
};
use objc2_core_foundation::{CFRunLoop, CFRunLoopRunResult, kCFRunLoopDefaultMode};
use objc2_foundation::NSArray;
use objc2_io_bluetooth::{
    BluetoothDeviceAddress, BluetoothNumericValue, BluetoothPINCode, IOBluetoothDevice,
    IOBluetoothDeviceAsyncCallbacks, IOBluetoothDeviceInquiry, IOBluetoothDeviceInquiryDelegate,
    IOBluetoothDevicePair, IOBluetoothDevicePairDelegate, IOBluetoothDeviceSearchTypesBits,
    IOBluetoothRFCOMMChannel, IOBluetoothRFCOMMChannelDelegate, IOBluetoothSDPUUID,
};
use rongta::bluetooth::{Discovery, ScanDevice};
use tokio::{
    io::DuplexStream,
    sync::{mpsc, oneshot},
};

use super::{Address, bridge, no_adapter_selection};

type IOReturn = std::ffi::c_int;

const SPP_UUID16: u16 = 0x1101;
/// Longest a run-loop slice may block before outgoing writes are checked.
const POLL: Duration = Duration::from_millis(5);
/// HCI's "RSSI not available" value.
const RSSI_UNAVAILABLE: i8 = 127;

/// Keep the IOBluetooth thread alive until the stream is closed. Dropping the
/// stream closes the RFCOMM channel and the baseband connection.
pub struct Connection {
    pub stream: DuplexStream,
}

impl Connection {
    /// Pair through IOBluetooth if needed, resolve SPP by SDP, and open RFCOMM.
    /// Without --pin, a PIN request fails; macOS has no agent to forward it to.
    pub async fn connect(
        address: Address,
        adapter: Option<&str>,
        pin: Option<String>,
        timeout: Duration,
    ) -> Result<Self> {
        no_adapter_selection(adapter)?;
        let (stream, ends) = bridge::stream();
        let (ready_tx, ready_rx) = oneshot::channel();
        let deadline = Instant::now() + timeout;
        std::thread::Builder::new()
            .name("iobluetooth".into())
            .spawn(move || {
                let delegate = Delegate::new(pin.map(String::into_bytes), Some(ends.incoming));
                match open(&delegate, address, deadline) {
                    Ok((device, channel)) => {
                        if ready_tx.send(Ok(())).is_ok() {
                            serve(&delegate, &channel, ends.outgoing);
                        }
                        close(&device, &channel);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
                delegate.ivars().incoming.take();
            })
            .context("Starting IOBluetooth thread")?;
        match tokio::time::timeout(timeout, ready_rx).await {
            Ok(Ok(result)) => result.map(|()| Self { stream }),
            Ok(Err(_)) => bail!("IOBluetooth thread exited while connecting {address}"),
            Err(_) => bail!("Timed out after {timeout:?} pairing/connecting {address}"),
        }
    }
}

/// macOS implementation of the portable discovery interface.
pub struct SystemDiscovery<'a> {
    pub adapter_name: Option<&'a str>,
}

impl Discovery for SystemDiscovery<'_> {
    type DeviceId = Address;

    async fn discover(&mut self, duration: Duration) -> Result<Vec<ScanDevice<Address>>> {
        no_adapter_selection(self.adapter_name)?;
        ensure!(!duration.is_zero(), "Scan duration must be nonzero");
        let (tx, rx) = oneshot::channel();
        std::thread::Builder::new()
            .name("iobluetooth-scan".into())
            .spawn(move || {
                let _ = tx.send(scan(duration));
            })
            .context("Starting IOBluetooth thread")?;
        rx.await.context("IOBluetooth scan thread exited")?
    }
}

#[derive(Default)]
struct Ivars {
    pin: Option<Vec<u8>>,
    pin_missing: Cell<bool>,
    pairing: Cell<Option<IOReturn>>,
    sdp: Cell<Option<IOReturn>>,
    inquiry: Cell<Option<IOReturn>>,
    closed: Cell<bool>,
    incoming: RefCell<Option<mpsc::UnboundedSender<Vec<u8>>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and Delegate has no Drop.
    #[unsafe(super(NSObject))]
    #[name = "RongtaIOBluetoothDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl IOBluetoothDevicePairDelegate for Delegate {
        #[unsafe(method(devicePairingPINCodeRequest:))]
        unsafe fn pin_code_request(&self, sender: Option<&AnyObject>) {
            let Some(pair) = sender.and_then(|s| s.downcast_ref::<IOBluetoothDevicePair>()) else {
                return;
            };
            match &self.ivars().pin {
                Some(pin) => {
                    let mut code = BluetoothPINCode { data: [0; 16] };
                    code.data[..pin.len()].copy_from_slice(pin);
                    unsafe { pair.replyPINCode_PINCode(pin.len() as _, &mut code) };
                }
                None => {
                    self.ivars().pin_missing.set(true);
                    unsafe { pair.stop() };
                }
            }
        }

        // A printer has no display, so there is no value to compare; accept
        // confirmation ("just works") rather than failing the pairing.
        #[unsafe(method(devicePairingUserConfirmationRequest:numericValue:))]
        unsafe fn user_confirmation_request(
            &self,
            sender: Option<&AnyObject>,
            numeric_value: BluetoothNumericValue,
        ) {
            tracing::warn!(
                numeric_value,
                "Accepting Secure Simple Pairing confirmation"
            );
            if let Some(pair) = sender.and_then(|s| s.downcast_ref::<IOBluetoothDevicePair>()) {
                unsafe { pair.replyUserConfirmation(true) };
            }
        }

        #[unsafe(method(devicePairingFinished:error:))]
        unsafe fn pairing_finished(&self, _sender: Option<&AnyObject>, error: IOReturn) {
            self.ivars().pairing.set(Some(error));
        }
    }

    unsafe impl IOBluetoothDeviceAsyncCallbacks for Delegate {
        #[unsafe(method(remoteNameRequestComplete:status:))]
        unsafe fn remote_name_request_complete(
            &self,
            _device: Option<&IOBluetoothDevice>,
            _status: IOReturn,
        ) {
        }

        #[unsafe(method(connectionComplete:status:))]
        unsafe fn connection_complete(
            &self,
            _device: Option<&IOBluetoothDevice>,
            _status: IOReturn,
        ) {
        }

        #[unsafe(method(sdpQueryComplete:status:))]
        unsafe fn sdp_query_complete(&self, _device: Option<&IOBluetoothDevice>, status: IOReturn) {
            self.ivars().sdp.set(Some(status));
        }
    }

    unsafe impl IOBluetoothRFCOMMChannelDelegate for Delegate {
        #[unsafe(method(rfcommChannelData:data:length:))]
        unsafe fn channel_data(
            &self,
            _channel: Option<&IOBluetoothRFCOMMChannel>,
            data: *mut c_void,
            length: usize,
        ) {
            if data.is_null() || length == 0 {
                return;
            }
            // SAFETY: IOBluetooth passes a buffer of `length` bytes valid for this call.
            let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
            if let Some(incoming) = &*self.ivars().incoming.borrow() {
                let _ = incoming.send(bytes.to_vec());
            }
        }

        #[unsafe(method(rfcommChannelClosed:))]
        unsafe fn channel_closed(&self, _channel: Option<&IOBluetoothRFCOMMChannel>) {
            self.ivars().closed.set(true);
        }
    }

    unsafe impl IOBluetoothDeviceInquiryDelegate for Delegate {
        #[unsafe(method(deviceInquiryComplete:error:aborted:))]
        unsafe fn inquiry_complete(
            &self,
            _sender: Option<&IOBluetoothDeviceInquiry>,
            error: IOReturn,
            _aborted: bool,
        ) {
            self.ivars().inquiry.set(Some(error));
        }
    }
);

impl Delegate {
    fn new(
        pin: Option<Vec<u8>>,
        incoming: Option<mpsc::UnboundedSender<Vec<u8>>>,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            pin,
            incoming: RefCell::new(incoming),
            ..Default::default()
        });
        unsafe { msg_send![super(this), init] }
    }

    fn object(&self) -> &AnyObject {
        self.as_ref()
    }
}

fn check(status: IOReturn, action: &str) -> Result<()> {
    ensure!(
        status == 0,
        "{action} failed (IOReturn {:#010x}); check that Bluetooth is on and that \
         this terminal has Bluetooth permission in System Settings > Privacy & Security",
        status as u32
    );
    Ok(())
}

/// Runs this thread's run loop until `done` or the deadline; false on timeout.
fn run_until(deadline: Instant, mut done: impl FnMut() -> bool) -> bool {
    while !done() {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        run_slice((deadline - now).min(POLL));
    }
    true
}

fn run_slice(duration: Duration) {
    // SAFETY: kCFRunLoopDefaultMode is an immutable CoreFoundation constant.
    let mode = unsafe { kCFRunLoopDefaultMode };
    // With no sources registered yet, the run loop returns at once; sleep
    // instead so callers polling for a callback do not spin.
    if CFRunLoop::run_in_mode(mode, duration.as_secs_f64(), true) == CFRunLoopRunResult::Finished {
        std::thread::sleep(duration);
    }
}

fn open(
    delegate: &Delegate,
    address: Address,
    deadline: Instant,
) -> Result<(
    Retained<IOBluetoothDevice>,
    Retained<IOBluetoothRFCOMMChannel>,
)> {
    let device = unsafe {
        IOBluetoothDevice::deviceWithAddress(&BluetoothDeviceAddress { data: address.0 })
    }
    .with_context(|| format!("IOBluetooth has no device object for {address}"))?;
    if !unsafe { device.isPaired() } {
        tracing::info!(%address, "Pairing printer through IOBluetooth");
        pair(delegate, &device, deadline)?;
    }

    tracing::info!(%address, "Querying printer services (SDP)");
    check(
        unsafe { device.performSDPQuery(Some(delegate.object())) },
        "Starting SDP query",
    )?;
    ensure!(
        run_until(deadline, || delegate.ivars().sdp.get().is_some()),
        "Timed out waiting for the printer's SDP records"
    );
    check(
        delegate.ivars().sdp.get().unwrap_or(0),
        "Querying printer services",
    )?;
    let uuid = unsafe { IOBluetoothSDPUUID::uuid16(SPP_UUID16) }.context("Creating SPP UUID")?;
    let record = unsafe { device.getServiceRecordForUUID(Some(&uuid)) }
        .context("Printer does not advertise a Serial Port Profile service")?;
    let mut channel_id = 0;
    check(
        unsafe { record.getRFCOMMChannelID(&mut channel_id) },
        "Reading the SPP RFCOMM channel",
    )?;

    tracing::info!(%address, channel_id, "Connecting SPP");
    let mut channel = None;
    check(
        unsafe {
            device.openRFCOMMChannelSync_withChannelID_delegate(
                Some(&mut channel),
                channel_id,
                Some(delegate.object()),
            )
        },
        "Opening the printer's RFCOMM channel",
    )?;
    let channel = channel.context("IOBluetooth reported success without an RFCOMM channel")?;
    Ok((device, channel))
}

fn pair(delegate: &Delegate, device: &IOBluetoothDevice, deadline: Instant) -> Result<()> {
    let pair = unsafe { IOBluetoothDevicePair::pairWithDevice(Some(device)) }
        .context("Creating IOBluetooth pairing request")?;
    unsafe { pair.setDelegate(Some(delegate.object())) };
    let result = (|| {
        check(unsafe { pair.start() }, "Starting pairing")?;
        let finished = run_until(deadline, || {
            delegate.ivars().pairing.get().is_some() || delegate.ivars().pin_missing.get()
        });
        ensure!(
            !delegate.ivars().pin_missing.get(),
            "The printer requested a PIN; supply it with --pin"
        );
        ensure!(finished, "Timed out pairing with the printer");
        check(
            delegate.ivars().pairing.get().unwrap_or(0),
            "Pairing (pair in System Settings or supply the printer's PIN with --pin)",
        )
    })();
    if result.is_err() {
        unsafe { pair.stop() };
    }
    unsafe { pair.setDelegate(None) };
    result
}

fn serve(
    delegate: &Delegate,
    channel: &IOBluetoothRFCOMMChannel,
    mut outgoing: mpsc::Receiver<Vec<u8>>,
) {
    loop {
        if delegate.ivars().closed.get() {
            tracing::info!("Printer closed the RFCOMM channel");
            return;
        }
        match outgoing.try_recv() {
            Ok(mut data) => {
                if let Err(error) = write(channel, &mut data) {
                    tracing::error!("{error:#}");
                    return;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => run_slice(POLL),
            Err(mpsc::error::TryRecvError::Disconnected) => return,
        }
    }
}

fn write(channel: &IOBluetoothRFCOMMChannel, data: &mut [u8]) -> Result<()> {
    let mtu = usize::from(unsafe { channel.getMTU() }).max(1);
    for chunk in data.chunks_mut(mtu) {
        let length = u16::try_from(chunk.len()).expect("chunk is bounded by the u16 MTU");
        check(
            unsafe { channel.writeSync_length(chunk.as_mut_ptr().cast(), length) },
            "Writing to the printer",
        )?;
    }
    Ok(())
}

fn close(device: &IOBluetoothDevice, channel: &IOBluetoothRFCOMMChannel) {
    unsafe {
        channel.closeChannel();
        // The delegate is about to be freed, and IOBluetooth does not retain it.
        channel.setDelegate(None);
        device.closeConnection();
    }
}

/// Inquire for Classic devices, then add paired devices that were not found.
fn scan(duration: Duration) -> Result<Vec<ScanDevice<Address>>> {
    let delegate = Delegate::new(None, None);
    let paired = devices(unsafe { IOBluetoothDevice::pairedDevices() });
    let known: BTreeSet<Address> = paired.iter().filter_map(|d| address_of(d)).collect();
    let inquiry = unsafe { IOBluetoothDeviceInquiry::inquiryWithDelegate(Some(delegate.object())) }
        .context("Creating Bluetooth inquiry")?;
    let result = run_inquiry(&delegate, &inquiry, duration);
    unsafe { inquiry.setDelegate(None) };
    result?;

    let found = devices(unsafe { inquiry.foundDevices() });
    let mut seen = BTreeSet::new();
    let mut scanned = vec![];
    for device in found.iter().chain(&paired) {
        let Some(address) = address_of(device) else {
            continue;
        };
        if seen.insert(address) {
            scanned.push(describe(device, address, known.contains(&address)));
        }
    }
    scanned.sort_by_key(|device| device.address);
    Ok(scanned)
}

fn run_inquiry(
    delegate: &Delegate,
    inquiry: &IOBluetoothDeviceInquiry,
    duration: Duration,
) -> Result<()> {
    let deadline = Instant::now() + duration;
    // A single inquiry lasts at most 255 seconds; restart until the deadline.
    let length = duration.as_secs().clamp(1, 255) as u8;
    unsafe {
        inquiry.setInquiryLength(length);
        inquiry.setSearchType(IOBluetoothDeviceSearchTypesBits::Classic.0);
        inquiry.setUpdateNewDeviceNames(true);
    }
    tracing::info!(
        seconds = duration.as_secs(),
        "Scanning Bluetooth Classic devices"
    );
    loop {
        delegate.ivars().inquiry.set(None);
        check(unsafe { inquiry.start() }, "Starting Bluetooth inquiry")?;
        if !run_until(deadline, || delegate.ivars().inquiry.get().is_some()) {
            unsafe { inquiry.stop() };
            return Ok(());
        }
        check(
            delegate.ivars().inquiry.get().unwrap_or(0),
            "Bluetooth inquiry",
        )?;
        if Instant::now() >= deadline {
            return Ok(());
        }
    }
}

fn devices(array: Option<Retained<NSArray>>) -> Vec<Retained<IOBluetoothDevice>> {
    array
        .map(|array| {
            array
                .iter()
                .filter_map(|object| object.downcast::<IOBluetoothDevice>().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[allow(deprecated)] // getAddress is the only accessor that returns raw bytes.
fn address_of(device: &IOBluetoothDevice) -> Option<Address> {
    unsafe { device.getAddress().as_ref() }.map(|address| Address(address.data))
}

#[allow(deprecated)] // getName returns nil safely; `name` is declared nonnull but is not.
fn describe(device: &IOBluetoothDevice, address: Address, known: bool) -> ScanDevice<Address> {
    let rssi = unsafe { device.RSSI() };
    let class = unsafe { device.classOfDevice() };
    let spp = unsafe { device.services() }.map(|_| {
        unsafe { IOBluetoothSDPUUID::uuid16(SPP_UUID16) }
            .is_some_and(|uuid| unsafe { device.getServiceRecordForUUID(Some(&uuid)) }.is_some())
    });
    ScanDevice {
        address,
        name: unsafe { device.getName() }
            .map(|name| name.to_string())
            .filter(|name| !name.is_empty()),
        rssi: (rssi != RSSI_UNAVAILABLE).then_some(i16::from(rssi)),
        class: (class != 0).then_some(class),
        paired: unsafe { device.isPaired() },
        connected: unsafe { device.isConnected() },
        spp,
        known_before_scan: known,
    }
}
