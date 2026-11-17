//! Battery providers. Each one knows how to talk to one family of devices.
//!
//! Providers run in priority order on the monitor thread. Vendor-specific HID drivers run
//! first and *claim* the HID collections they handle, so the generic HID reader never
//! sends anything to a device that already has a dedicated driver.

use crate::model::Reading;
use std::collections::HashSet;
use std::path::PathBuf;

pub mod demo;
pub mod generic_hid;
pub mod headsetcontrol;
pub mod logitech;
pub mod playstation;
pub mod razer;
pub mod sonix;

#[cfg(windows)]
pub mod airpods;
#[cfg(windows)]
pub mod bluetooth;
#[cfg(target_os = "linux")]
pub mod linux_sysfs;
#[cfg(windows)]
pub mod xinput;

/// A Bluetooth peer that is currently connected, shared with providers that need to know
/// what is paired (e.g. AirPods advertisement decoding).
#[derive(Debug, Clone)]
pub struct BtPeer {
    pub address: u64,
    pub name: String,
    pub key: String,
}

pub struct PollContext<'a> {
    pub hid: Option<&'a hidapi::HidApi>,
    /// HID paths claimed by a vendor driver.
    pub claimed_paths: HashSet<String>,
    /// (vid, pid) pairs a vendor driver successfully read.
    pub claimed_ids: HashSet<(u16, u16)>,
    pub bt_connected: Vec<BtPeer>,
    /// Set after a device arrival/removal or a manual refresh: providers should ignore
    /// their negative caches and re-probe everything.
    pub full_scan: bool,
}

impl PollContext<'_> {
    pub fn claim(&mut self, info: &hidapi::DeviceInfo) {
        self.claimed_paths.insert(path_of(info));
    }
}

/// Every provider id, used to restore persisted devices.
pub const SOURCE_IDS: [&str; 11] = [
    "logitech-hidpp",
    "razer",
    "playstation",
    "sonix",
    "headsetcontrol",
    "xinput",
    "bluetooth",
    "airpods",
    "hid-battery",
    "sysfs",
    "demo",
];

pub trait Provider {
    /// Short stable id, shown in the UI as the data source.
    fn id(&self) -> &'static str;
    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading>;
}

#[derive(Debug, Clone, Default)]
pub struct ProviderOptions {
    /// Explicit path to the HeadsetControl executable. Searched on PATH when `None`.
    pub headsetcontrol_path: Option<PathBuf>,
    /// Replace every real provider with simulated devices (UI development, screenshots).
    pub demo: bool,
}

pub fn default_providers(opts: &ProviderOptions) -> Vec<Box<dyn Provider>> {
    if opts.demo {
        return vec![Box::new(demo::Demo)];
    }
    // Order matters: dedicated drivers first, generic fallbacks last.
    let mut v: Vec<Box<dyn Provider>> = vec![
        Box::new(logitech::Logitech::default()),
        Box::new(razer::Razer::default()),
        Box::new(playstation::PlayStation::default()),
        Box::new(sonix::Sonix::default()),
        Box::new(headsetcontrol::HeadsetControl::new(opts.headsetcontrol_path.clone())),
    ];
    #[cfg(windows)]
    {
        v.push(Box::new(xinput::XInput));
        v.push(Box::new(bluetooth::Bluetooth::default()));
        v.push(Box::new(airpods::AirPods::default()));
    }
    #[cfg(target_os = "linux")]
    v.push(Box::new(linux_sysfs::Sysfs));
    v.push(Box::new(generic_hid::GenericHid::default()));
    v
}

pub(crate) fn path_of(info: &hidapi::DeviceInfo) -> String {
    info.path().to_string_lossy().into_owned()
}

/// Product string or a hex fallback.
pub(crate) fn product_name(info: &hidapi::DeviceInfo) -> String {
    info.product_string()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("HID {:04x}:{:04x}", info.vendor_id(), info.product_id()))
}

pub(crate) fn transport_of(info: &hidapi::DeviceInfo) -> crate::model::Transport {
    match info.bus_type() {
        hidapi::BusType::Bluetooth => crate::model::Transport::Bluetooth,
        hidapi::BusType::Usb => crate::model::Transport::Usb,
        _ => crate::model::Transport::Unknown,
    }
}
