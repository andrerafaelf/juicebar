//! Data model shared by every provider and by the UI layer.

use serde::{Deserialize, Serialize};

/// What kind of peripheral a battery belongs to. Drives the icon and sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Headset,
    Earbuds,
    Mouse,
    Keyboard,
    Gamepad,
    Speaker,
    Phone,
    Watch,
    Pen,
    Other,
}

/// How the device reaches the PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Usb,
    Receiver,
    Bluetooth,
    Ble,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Charge {
    Discharging,
    Charging,
    Full,
    Unknown,
}

impl Charge {
    pub fn is_charging(self) -> bool {
        matches!(self, Charge::Charging)
    }
}

/// A single battery cell. Most devices have one; earbuds report left / right / case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Battery {
    /// `None` for the main battery, otherwise e.g. "Left", "Right", "Case".
    pub label: Option<String>,
    /// 0..=100
    pub level: u8,
    pub charge: Charge,
    /// Resolution of `level` in percent: 1 for exact readings, 10/20/25… for devices
    /// that only report coarse steps. The UI shows "~" for coarse values.
    pub step: u8,
}

impl Battery {
    pub fn new(level: u8, charge: Charge) -> Self {
        Self { label: None, level: level.min(100), charge, step: 1 }
    }
    pub fn coarse(mut self, step: u8) -> Self {
        self.step = step.max(1);
        self
    }
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn is_case(&self) -> bool {
        self.label.as_deref() == Some("Case")
    }
}

/// One observation produced by a provider during a poll.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    /// Stable identity across polls, e.g. `bt:a0c9a0123456` or `hidpp:4a2f19c0`.
    pub key: String,
    pub name: String,
    pub vendor: Option<String>,
    pub kind: DeviceKind,
    pub transport: Transport,
    pub batteries: Vec<Battery>,
    /// Provider id that produced the reading.
    pub source: &'static str,
}

impl Reading {
    /// The battery that represents the device as a whole: the lowest non-case cell.
    pub fn primary(&self) -> Option<&Battery> {
        primary_battery(&self.batteries)
    }
}

pub fn primary_battery(batteries: &[Battery]) -> Option<&Battery> {
    batteries.iter().filter(|b| !b.is_case()).min_by_key(|b| b.level).or_else(|| batteries.first())
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    /// Minutes until empty (discharging) or until full (charging).
    pub minutes: u32,
    pub to_full: bool,
    /// Observed rate in percent per hour (negative while discharging).
    pub rate_per_hour: f32,
}

/// What the UI receives: a reading plus monitor-side state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub key: String,
    pub name: String,
    pub vendor: Option<String>,
    pub kind: DeviceKind,
    pub transport: Transport,
    pub batteries: Vec<Battery>,
    pub level: u8,
    pub charge: Charge,
    pub step: u8,
    pub source: &'static str,
    /// False when the device stopped answering (asleep / out of range); values are last known.
    pub online: bool,
    /// Unix epoch milliseconds of the last successful reading.
    pub last_seen: u64,
    pub estimate: Option<Estimate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    /// Unix epoch seconds.
    pub t: u64,
    pub level: u8,
    pub charging: bool,
}
