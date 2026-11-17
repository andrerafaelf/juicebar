//! Linux: peripheral batteries exposed by the kernel under /sys/class/power_supply
//! (hid-logitech-hidpp, hid-sony, hid-playstation, BlueZ battery plugin, …).

use super::{PollContext, Provider};
use crate::model::{Battery, Charge, Reading, Transport};
use crate::util::{guess_kind, normalize_name};
use std::fs;
use std::path::Path;

pub struct Sysfs;

fn read(dir: &Path, file: &str) -> Option<String> {
    fs::read_to_string(dir.join(file)).ok().map(|s| s.trim().to_string())
}

impl Provider for Sysfs {
    fn id(&self) -> &'static str {
        "sysfs"
    }

    fn poll(&mut self, _ctx: &mut PollContext) -> Vec<Reading> {
        let Ok(entries) = fs::read_dir("/sys/class/power_supply") else { return vec![] };
        entries
            .flatten()
            .filter_map(|e| {
                let dir = e.path();
                // "Device" scope = peripheral battery; skip the laptop's own battery and AC.
                if read(&dir, "scope").as_deref() != Some("Device") {
                    return None;
                }
                let (level, step) = match read(&dir, "capacity").and_then(|c| c.parse::<u8>().ok()) {
                    Some(c) => (c, 1),
                    None => match read(&dir, "capacity_level")?.as_str() {
                        "Full" => (100, 25),
                        "High" => (75, 25),
                        "Normal" => (50, 25),
                        "Low" => (20, 25),
                        "Critical" => (5, 25),
                        _ => return None,
                    },
                };
                let charge = match read(&dir, "status").as_deref() {
                    Some("Charging") => Charge::Charging,
                    Some("Full") => Charge::Full,
                    Some("Discharging") | Some("Not charging") => Charge::Discharging,
                    _ => Charge::Unknown,
                };
                let id = e.file_name().to_string_lossy().into_owned();
                let name = read(&dir, "model_name").filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());
                Some(Reading {
                    key: format!("sysfs:{}", normalize_name(&id)),
                    kind: guess_kind(&name),
                    vendor: read(&dir, "manufacturer").filter(|s| !s.is_empty()),
                    name,
                    transport: Transport::Unknown,
                    batteries: vec![Battery::new(level, charge).coarse(step)],
                    source: "sysfs",
                })
            })
            .collect()
    }
}
