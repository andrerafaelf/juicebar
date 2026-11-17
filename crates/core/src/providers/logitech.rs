//! Logitech HID++ 2.0: Unifying, Bolt, Lightspeed and Nano receivers, plus devices on a
//! cable or Bluetooth (device index 0xFF).
//!
//! Protocol reference: Logitech's public HID++ 2.0 feature specifications
//! (x0000 Root, x0003 DeviceInformation, x0005 DeviceNameType, x1000 BatteryStatus,
//! x1001 BatteryVoltage, x1004 UnifiedBattery).

use super::{path_of, product_name, transport_of, PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use crate::util::{clean_name, guess_kind, lipo_percent};
use hidapi::{DeviceInfo, HidDevice};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const VID: u16 = 0x046D;
const REPORT_SHORT: u8 = 0x10;
const REPORT_LONG: u8 = 0x11;
const SW_ID: u8 = 0x0A;
const DIRECT: u8 = 0xFF;

const FEAT_ROOT: u16 = 0x0000;
const FEAT_DEVICE_INFO: u16 = 0x0003;
const FEAT_NAME: u16 = 0x0005;
const FEAT_BATTERY_STATUS: u16 = 0x1000;
const FEAT_BATTERY_VOLTAGE: u16 = 0x1001;
const FEAT_UNIFIED_BATTERY: u16 = 0x1004;

/// Lightspeed receivers pair with exactly one device (index 1).
const LIGHTSPEED: [u16; 9] = [0xC539, 0xC53A, 0xC53D, 0xC53F, 0xC541, 0xC545, 0xC547, 0xC54D, 0xC54F];
/// Unifying / Nano / Bolt receivers (up to six devices).
const MULTI: [u16; 9] = [0xC52B, 0xC52F, 0xC531, 0xC532, 0xC534, 0xC535, 0xC537, 0xC542, 0xC548];

#[derive(Debug)]
#[allow(dead_code)] // codes kept for debugging output
enum Error {
    Io,
    Timeout,
    /// HID++ error code returned by the device or receiver.
    Protocol(u8),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BatteryFeature {
    Unified,
    Status,
    Voltage,
}

#[derive(Debug, Clone)]
struct Meta {
    name: String,
    kind: DeviceKind,
    unit_id: Option<u32>,
    battery: Option<(BatteryFeature, u8)>,
}

enum Slot {
    Absent { retry_at: u64 },
    Present(Meta),
}

#[derive(Default)]
pub struct Logitech {
    slots: HashMap<(String, u8), Slot>,
    polls: u64,
}

struct Channel {
    dev: HidDevice,
}

impl Channel {
    fn drain(&self) {
        let mut buf = [0u8; 20];
        while matches!(self.dev.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
    }

    fn request(&self, idx: u8, feature: u8, function: u8, params: &[u8], timeout: Duration) -> Result<[u8; 16], Error> {
        let mut msg = [0u8; 20];
        msg[0] = REPORT_LONG;
        msg[1] = idx;
        msg[2] = feature;
        msg[3] = (function << 4) | SW_ID;
        msg[4..4 + params.len()].copy_from_slice(params);
        self.dev.write(&msg).map_err(|_| Error::Io)?;

        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout);
            }
            let mut r = [0u8; 20];
            let n = self.dev.read_timeout(&mut r, left.as_millis().max(1) as i32).map_err(|_| Error::Io)?;
            if n < 7 || r[1] != idx || !(r[0] == REPORT_LONG || r[0] == REPORT_SHORT) {
                continue;
            }
            // HID++ 2.0 error: [id][idx][FF][feature][fn|sw][code]
            if r[2] == 0xFF && r[3] == feature && r[4] == msg[3] {
                return Err(Error::Protocol(r[5]));
            }
            // HID++ 1.0 error (receiver says the device is not there): [id][idx][8F][subid][addr][code]
            if r[2] == 0x8F && r[3] == feature && r[4] == msg[3] {
                return Err(Error::Protocol(r[5]));
            }
            if r[2] == feature && r[3] == msg[3] {
                let mut out = [0u8; 16];
                out.copy_from_slice(&r[4..20]);
                return Ok(out);
            }
        }
    }

    fn feature_index(&self, idx: u8, feature: u16) -> Result<Option<u8>, Error> {
        let r = self.request(idx, 0, 0, &[(feature >> 8) as u8, feature as u8], Duration::from_millis(500))?;
        Ok((r[0] != 0).then_some(r[0]))
    }

    fn ping(&self, idx: u8) -> Result<u8, Error> {
        let r = self.request(idx, 0, 1, &[0, 0, 0x5A], Duration::from_millis(250))?;
        Ok(r[0])
    }

    fn discover(&self, idx: u8, fallback_name: &str) -> Result<Meta, Error> {
        let _ = FEAT_ROOT;
        if self.ping(idx)? < 2 {
            return Err(Error::Protocol(0)); // HID++ 1.0 device: not supported
        }
        let mut meta = Meta { name: fallback_name.to_string(), kind: guess_kind(fallback_name), unit_id: None, battery: None };

        if let Some(fi) = self.feature_index(idx, FEAT_NAME)? {
            let len = self.request(idx, fi, 0, &[], Duration::from_millis(500))?[0] as usize;
            let mut name = Vec::with_capacity(len);
            while name.len() < len {
                let chunk = self.request(idx, fi, 1, &[name.len() as u8], Duration::from_millis(500))?;
                let take = (len - name.len()).min(16);
                name.extend_from_slice(&chunk[..take]);
            }
            let name = String::from_utf8_lossy(&name).trim_matches(char::from(0)).trim().to_string();
            if !name.is_empty() {
                meta.name = name;
            }
            if let Ok(t) = self.request(idx, fi, 2, &[], Duration::from_millis(500)) {
                meta.kind = match t[0] {
                    0 | 2 => DeviceKind::Keyboard,
                    3..=5 => DeviceKind::Mouse,
                    8 => DeviceKind::Headset,
                    11 | 12 => DeviceKind::Gamepad,
                    _ => guess_kind(&meta.name),
                };
            }
        }
        if let Some(fi) = self.feature_index(idx, FEAT_DEVICE_INFO)? {
            if let Ok(r) = self.request(idx, fi, 0, &[], Duration::from_millis(500)) {
                let unit = u32::from_be_bytes([r[1], r[2], r[3], r[4]]);
                meta.unit_id = (unit != 0).then_some(unit);
            }
        }
        for (feature, kind) in [
            (FEAT_UNIFIED_BATTERY, BatteryFeature::Unified),
            (FEAT_BATTERY_STATUS, BatteryFeature::Status),
            (FEAT_BATTERY_VOLTAGE, BatteryFeature::Voltage),
        ] {
            if let Some(fi) = self.feature_index(idx, feature)? {
                meta.battery = Some((kind, fi));
                break;
            }
        }
        Ok(meta)
    }

    fn battery(&self, idx: u8, feature: BatteryFeature, fi: u8) -> Result<Battery, Error> {
        let t = Duration::from_millis(800);
        Ok(match feature {
            BatteryFeature::Unified => decode_unified(&self.request(idx, fi, 1, &[], t)?),
            BatteryFeature::Status => decode_status(&self.request(idx, fi, 0, &[], t)?),
            BatteryFeature::Voltage => decode_voltage(&self.request(idx, fi, 0, &[], t)?),
        })
    }
}

/// x1004 get_status: [soc %][level flags][charging status][external power]
fn decode_unified(r: &[u8; 16]) -> Battery {
    let charge = match r[2] {
        0 => Charge::Discharging,
        1 | 2 => Charge::Charging,
        3 => Charge::Full,
        _ => Charge::Unknown,
    };
    if r[0] > 0 {
        return Battery::new(r[0], charge);
    }
    // No state-of-charge support: map the coarse level flags.
    let level = match r[1] {
        f if f & 0x08 != 0 => 100,
        f if f & 0x04 != 0 => 60,
        f if f & 0x02 != 0 => 20,
        _ => 5,
    };
    Battery::new(level, charge).coarse(20)
}

/// x1000 get_level_status: [discharge level %][next level][status]
fn decode_status(r: &[u8; 16]) -> Battery {
    let charge = match r[2] {
        0 => Charge::Discharging,
        1 | 2 | 4 => Charge::Charging,
        3 => Charge::Full,
        _ => Charge::Unknown,
    };
    let level = if charge == Charge::Full { 100 } else { r[0] };
    // Devices with this feature commonly report in steps (5/20/50/90%).
    let step = if level % 10 == 0 || level == 5 { 10 } else { 1 };
    Battery::new(level, charge).coarse(step)
}

/// x1001 get_battery_voltage: [mV hi][mV lo][flags]
fn decode_voltage(r: &[u8; 16]) -> Battery {
    let mv = u16::from_be_bytes([r[0], r[1]]);
    let flags = r[2];
    let level = lipo_percent(mv);
    let charge = if flags & 0x80 != 0 {
        if flags & 0x07 == 2 || level >= 100 {
            Charge::Full
        } else {
            Charge::Charging
        }
    } else {
        Charge::Discharging
    };
    Battery::new(level, charge)
}

fn indices_for(info: &DeviceInfo) -> (Vec<u8>, bool) {
    let pid = info.product_id();
    let is_receiver_name = info.product_string().is_some_and(|s| s.to_ascii_lowercase().contains("receiver"));
    if LIGHTSPEED.contains(&pid) {
        (vec![1], true)
    } else if MULTI.contains(&pid) || is_receiver_name {
        ((1..=6).collect(), true)
    } else {
        (vec![DIRECT], false)
    }
}

fn is_hidpp_collection(info: &DeviceInfo) -> bool {
    if info.vendor_id() != VID {
        return false;
    }
    let (page, usage) = (info.usage_page(), info.usage());
    if cfg!(windows) {
        // Windows splits collections: we need the long-report one.
        (page == 0xFF00 && usage == 0x0002) || (page == 0xFF43 && usage == 0x0202)
    } else {
        page == 0xFF00 || page == 0xFF43
    }
}

impl Provider for Logitech {
    fn id(&self) -> &'static str {
        "logitech-hidpp"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        self.polls += 1;
        let Some(api) = ctx.hid else { return vec![] };
        let infos: Vec<DeviceInfo> = api.device_list().filter(|i| is_hidpp_collection(i)).cloned().collect();
        let mut out = Vec::new();

        for info in infos {
            let path = path_of(&info);
            let (indices, receiver) = indices_for(&info);
            let Ok(dev) = info.open_device(api) else { continue };
            ctx.claim(&info);
            let ch = Channel { dev };
            ch.drain();
            let fallback = clean_name(&product_name(&info));

            for idx in indices {
                let slot_key = (path.clone(), idx);
                if let Some(Slot::Absent { retry_at }) = self.slots.get(&slot_key) {
                    if self.polls < *retry_at && !ctx.full_scan {
                        continue;
                    }
                }
                if !matches!(self.slots.get(&slot_key), Some(Slot::Present(_))) {
                    match ch.discover(idx, &fallback) {
                        Ok(meta) => {
                            self.slots.insert(slot_key.clone(), Slot::Present(meta));
                        }
                        Err(_) => {
                            // Asleep or not paired: check again in a few polls.
                            self.slots.insert(slot_key, Slot::Absent { retry_at: self.polls + 5 });
                            continue;
                        }
                    }
                }
                let Some(Slot::Present(meta)) = self.slots.get(&slot_key) else { continue };
                let Some((feature, fi)) = meta.battery else { continue };
                match ch.battery(idx, feature, fi) {
                    Ok(battery) => {
                        let key = match meta.unit_id {
                            Some(u) => format!("hidpp:{u:08x}"),
                            None => format!("hidpp:{:04x}:{idx}:{}", info.product_id(), crate::util::normalize_name(&meta.name)),
                        };
                        let transport = if receiver {
                            Transport::Receiver
                        } else if info.usage_page() == 0xFF43 {
                            Transport::Ble
                        } else {
                            transport_of(&info)
                        };
                        ctx.claimed_ids.insert((info.vendor_id(), info.product_id()));
                        out.push(Reading {
                            key,
                            name: meta.name.clone(),
                            vendor: Some("Logitech".into()),
                            kind: meta.kind,
                            transport,
                            batteries: vec![battery],
                            source: "logitech-hidpp",
                        });
                    }
                    // Timed out: device asleep. Keep metadata, the monitor keeps the last value.
                    Err(Error::Timeout) => {}
                    // Anything else: device changed (re-paired, firmware) – rediscover next time.
                    Err(Error::Io) | Err(Error::Protocol(_)) => {
                        self.slots.remove(&slot_key);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(bytes: &[u8]) -> [u8; 16] {
        let mut r = [0u8; 16];
        r[..bytes.len()].copy_from_slice(bytes);
        r
    }

    #[test]
    fn unified_battery() {
        let b = decode_unified(&p(&[73, 0x04, 0, 0]));
        assert_eq!((b.level, b.charge, b.step), (73, Charge::Discharging, 1));
        let b = decode_unified(&p(&[0, 0x02, 1, 1]));
        assert_eq!((b.level, b.charge, b.step), (20, Charge::Charging, 20));
        assert_eq!(decode_unified(&p(&[100, 0x08, 3, 1])).charge, Charge::Full);
    }

    #[test]
    fn battery_status_and_voltage() {
        let b = decode_status(&p(&[50, 20, 0]));
        assert_eq!((b.level, b.charge), (50, Charge::Discharging));
        assert_eq!(decode_status(&p(&[90, 0, 3])).level, 100);
        let b = decode_voltage(&p(&[0x0E, 0xE3, 0x00])); // 3811 mV
        assert_eq!((b.level, b.charge), (50, Charge::Discharging));
        assert_eq!(decode_voltage(&p(&[0x0F, 0xA0, 0x80])).charge, Charge::Charging);
    }
}
