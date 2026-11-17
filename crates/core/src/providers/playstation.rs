//! Sony DualShock 4, DualSense and DualSense Edge over USB or Bluetooth.
//!
//! Battery lives in the regular input report, so this provider only *reads*. Over
//! Bluetooth the controllers start in a reduced report mode; requesting the calibration
//! feature report (0x05) switches them to the full report, as every PC driver does.

use super::{path_of, transport_of, PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading};
use hidapi::DeviceInfo;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const VID: u16 = 0x054C;
const DS4: [u16; 3] = [0x05C4, 0x09CC, 0x0BA0];
const DUALSENSE: u16 = 0x0CE6;
const DUALSENSE_EDGE: u16 = 0x0DF2;

#[derive(Default)]
pub struct PlayStation {
    /// Bluetooth controllers we already switched to full reports.
    upgraded: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Model {
    Ds4,
    DualSense,
}

/// DualSense status byte: low nibble 0..=10 level, high nibble charging state.
pub(crate) fn decode_dualsense_status(status: u8) -> Option<Battery> {
    let data = status & 0x0F;
    let level = (data * 10 + 5).min(100);
    match status >> 4 {
        0x0 => Some(Battery::new(level, Charge::Discharging).coarse(10)),
        0x1 => Some(Battery::new(level, Charge::Charging).coarse(10)),
        0x2 => Some(Battery::new(100, Charge::Full).coarse(10)),
        _ => None, // voltage / temperature / charging error
    }
}

/// DualShock 4 status byte: low nibble level, bit 4 = cable connected.
pub(crate) fn decode_ds4_status(status: u8) -> Option<Battery> {
    let data = status & 0x0F;
    if status & 0x10 != 0 {
        match data {
            0..=9 => Some(Battery::new(data * 10 + 5, Charge::Charging).coarse(10)),
            10 => Some(Battery::new(100, Charge::Charging).coarse(10)),
            11 => Some(Battery::new(100, Charge::Full).coarse(10)),
            _ => None,
        }
    } else {
        Some(Battery::new((data * 10 + 5).min(100), Charge::Discharging).coarse(10))
    }
}

/// Locate the status byte in a raw input report (report id included).
pub(crate) fn parse_report(model: Model, r: &[u8]) -> Option<Battery> {
    match model {
        // USB 0x01 (64 bytes): data at [1]; BT 0x31: data at [2]. Status at data[52].
        Model::DualSense => match r.first()? {
            0x01 if r.len() >= 64 => decode_dualsense_status(r[1 + 52]),
            0x31 if r.len() >= 64 => decode_dualsense_status(r[2 + 52]),
            _ => None,
        },
        // USB 0x01 (64 bytes): common at [1]; BT 0x11: common at [3]. Status at common[29].
        Model::Ds4 => match r.first()? {
            0x01 if r.len() >= 64 => decode_ds4_status(r[1 + 29]),
            0x11 if r.len() >= 64 => decode_ds4_status(r[3 + 29]),
            _ => None,
        },
    }
}

fn model_of(info: &DeviceInfo) -> Option<(Model, &'static str)> {
    if info.vendor_id() != VID {
        return None;
    }
    match info.product_id() {
        p if DS4.contains(&p) => Some((Model::Ds4, "DualShock 4")),
        DUALSENSE => Some((Model::DualSense, "DualSense")),
        DUALSENSE_EDGE => Some((Model::DualSense, "DualSense Edge")),
        _ => None,
    }
}

impl Provider for PlayStation {
    fn id(&self) -> &'static str {
        "playstation"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        let Some(api) = ctx.hid else { return vec![] };
        let infos: Vec<DeviceInfo> =
            api.device_list().filter(|i| model_of(i).is_some() && i.usage_page() == 0x01 && i.usage() == 0x05).cloned().collect();
        let mut out = Vec::new();

        for info in infos {
            let (model, name) = model_of(&info).unwrap();
            ctx.claim(&info);
            let path = path_of(&info);
            let Ok(dev) = info.open_device(api) else { continue };
            let deadline = Instant::now() + Duration::from_millis(400);
            let mut battery = None;
            let mut buf = [0u8; 128];
            while battery.is_none() && Instant::now() < deadline {
                let Ok(n) = dev.read_timeout(&mut buf, 100) else { break };
                if n == 0 {
                    continue;
                }
                battery = parse_report(model, &buf[..n]);
                // Short 0x01 report over Bluetooth: ask for full reports once.
                if battery.is_none() && buf[0] == 0x01 && n < 64 && self.upgraded.insert(path.clone()) {
                    let mut feature = [0u8; 41];
                    feature[0] = 0x05;
                    let _ = dev.get_feature_report(&mut feature);
                }
            }
            let Some(battery) = battery else { continue };
            ctx.claimed_ids.insert((info.vendor_id(), info.product_id()));
            let serial = info.serial_number().unwrap_or_default().replace([':', '-'], "").to_ascii_lowercase();
            out.push(Reading {
                key: if serial.is_empty() { format!("sony:{}", path) } else { format!("sony:{serial}") },
                name: name.into(),
                vendor: Some("Sony".into()),
                kind: DeviceKind::Gamepad,
                transport: transport_of(&info),
                batteries: vec![battery],
                source: "playstation",
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dualsense_usb() {
        let mut r = [0u8; 64];
        r[0] = 0x01;
        r[53] = 0x07; // discharging, level 7
        let b = parse_report(Model::DualSense, &r).unwrap();
        assert_eq!((b.level, b.charge, b.step), (75, Charge::Discharging, 10));
        r[53] = 0x29;
        assert_eq!(parse_report(Model::DualSense, &r).unwrap().charge, Charge::Full);
        r[53] = 0xA0; // error
        assert!(parse_report(Model::DualSense, &r).is_none());
    }

    #[test]
    fn ds4_bt_and_usb() {
        let mut r = [0u8; 78];
        r[0] = 0x11;
        r[32] = 0x08;
        assert_eq!(parse_report(Model::Ds4, &r).unwrap().level, 85);
        let mut u = [0u8; 64];
        u[0] = 0x01;
        u[30] = 0x13; // cable + level 3
        let b = parse_report(Model::Ds4, &u).unwrap();
        assert_eq!((b.level, b.charge), (35, Charge::Charging));
        u[30] = 0x1B;
        assert_eq!(parse_report(Model::Ds4, &u).unwrap().charge, Charge::Full);
    }
}
