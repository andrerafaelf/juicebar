//! # juicebar-core
//!
//! Reads battery levels of wireless peripherals: mice, keyboards, headsets, earbuds and
//! controllers, across vendor protocols (Logitech HID++, Razer, Sony), OS sources
//! (Windows Bluetooth, XInput, Linux power_supply), standard HID battery usages and
//! HeadsetControl.
//!
//! ```no_run
//! let mut monitor = juicebar_core::Monitor::new(Default::default());
//! for d in monitor.poll(true) {
//!     println!("{:<28} {:>3}%  {:?}", d.name, d.level, d.charge);
//! }
//! ```

pub mod hid_descriptor;
pub mod history;
pub mod model;
pub mod monitor;
pub mod providers;
pub mod util;
#[cfg(windows)]
pub mod watch;
#[cfg(windows)]
mod winrt;

pub use model::*;
pub use monitor::{Monitor, MonitorOptions};
pub use providers::ProviderOptions;
