use rongta::bluetooth::{DeviceFilter, Discovery, ScanDevice, scan};
use std::time::Duration;

fn device(name: Option<&str>, class: Option<u32>) -> ScanDevice<String> {
    ScanDevice {
        address: "platform-specific-id".into(),
        name: name.map(str::to_owned),
        rssi: None,
        class,
        paired: false,
        connected: false,
        spp: None,
        known_before_scan: false,
    }
}

#[test]
fn printer_filter_matches_stock_java_reference_regardless_of_class_or_spp() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: Option<String>,
        expected: bool,
    }
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/stock-device-filter.json")).unwrap();
    for case in cases {
        for class in [None, Some(0x40680), Some(0x6c0), Some(0x480)] {
            for spp in [None, Some(false), Some(true)] {
                let mut device = device(case.name.as_deref(), class);
                device.spp = spp;
                assert_eq!(
                    device.is_printer_candidate(),
                    case.expected,
                    "name={:?}, class={class:?}, spp={spp:?}",
                    case.name
                );
            }
        }
    }
}

struct FakeDiscovery {
    called: bool,
    fail: bool,
}
impl Discovery for FakeDiscovery {
    type DeviceId = String;

    async fn discover(&mut self, duration: Duration) -> anyhow::Result<Vec<ScanDevice<String>>> {
        assert_eq!(duration, Duration::from_secs(2));
        self.called = true;
        anyhow::ensure!(!self.fail, "adapter unavailable");
        Ok(vec![
            device(Some("R22Pro"), None),
            device(Some("Headphones"), None),
            device(None, None),
        ])
    }
}

#[tokio::test]
async fn portable_scan_filters_backend_results_and_keeps_opaque_ids() {
    let mut backend = FakeDiscovery {
        called: false,
        fail: false,
    };
    let printers = scan(&mut backend, Duration::from_secs(2), DeviceFilter::Printers)
        .await
        .unwrap();
    assert!(backend.called);
    assert_eq!(printers.len(), 1);
    assert_eq!(printers[0].address, "platform-specific-id");
    assert_eq!(printers[0].name.as_deref(), Some("R22Pro"));
    let all = scan(&mut backend, Duration::from_secs(2), DeviceFilter::All)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert!(all[2].name.is_none());
}

#[tokio::test]
async fn invalid_duration_does_not_touch_backend_and_backend_errors_propagate() {
    let mut backend = FakeDiscovery {
        called: false,
        fail: true,
    };
    assert!(
        scan(&mut backend, Duration::ZERO, DeviceFilter::All)
            .await
            .is_err()
    );
    assert!(!backend.called);
    let error = scan(&mut backend, Duration::from_secs(2), DeviceFilter::Printers)
        .await
        .unwrap_err();
    assert!(backend.called);
    assert_eq!(error.to_string(), "adapter unavailable");
}

#[tokio::test]
async fn discovery_accepts_thread_bound_backends_and_native_handles() {
    use std::rc::Rc;

    struct ThreadBoundDiscovery(Rc<str>);
    impl Discovery for ThreadBoundDiscovery {
        type DeviceId = Rc<str>;

        async fn discover(
            &mut self,
            _: Duration,
        ) -> anyhow::Result<Vec<ScanDevice<Self::DeviceId>>> {
            let handle = self.0.clone();
            tokio::task::yield_now().await;
            Ok(vec![ScanDevice {
                address: handle,
                name: Some("R22".into()),
                rssi: None,
                class: None,
                paired: false,
                connected: false,
                spp: None,
                known_before_scan: false,
            }])
        }
    }

    let mut backend = ThreadBoundDiscovery(Rc::from("native handle"));
    let devices = scan(&mut backend, Duration::from_secs(1), DeviceFilter::Printers)
        .await
        .unwrap();
    assert_eq!(devices.len(), 1);
    assert!(Rc::ptr_eq(&devices[0].address, &backend.0));
}
