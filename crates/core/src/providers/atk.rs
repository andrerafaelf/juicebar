//! ATK and other Compx-platform mice (vendor id 0x373B), on a cable or through their
//! 1K/8K dongles. The dongle reports the mouse's serial number, so both links map to one
//! device.
//!
//! Commands go to the vendor collection 0xFF02:0x0002 whose report 8 is 17 bytes long:
//! `08 <opcode> <14 bytes> <checksum>`, the checksum making bytes 1..=16 sum to 0x4D.
//! GET_BATTERY (0x04) answers `08 04 00 00 00 02 <percent> <charging> <mV hi> <mV lo> …`.
//! It is the only opcode sent: the others write settings, pair or flash firmware. The
//! dongle also pushes status frames on report 8, which are skipped. A sleeping mouse does
//! not answer.
//!
//! Protocol notes: Techxey/atk-mouse-battery (MIT) and surfaceflinger/atk-info.

use super::{path_of, PollContext, Provider};
use crate::hid_descriptor::{parse, ReportKind};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use crate::util::{clean_name, normalize_name};
use hidapi::{DeviceInfo, HidDevice};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const VID: u16 = 0x373B;
const REPORT_ID: u8 = 0x08;
const LEN: usize = 17;
const GET_BATTERY: u8 = 0x04;
const CHECKSUM_TARGET: u8 = 0x4D;

#[derive(Default)]
pub struct Atk {
    /// Whether a collection is the 17-byte command channel, by path.
    channel: HashMap<String, bool>,
    /// Product name seen over the cable, by key: dongles only report a generic one.
    names: HashMap<String, String>,
}

fn checksum(body: &[u8]) -> u8 {
    CHECKSUM_TARGET.wrapping_sub(body.iter().fold(0u8, |a, b| a.wrapping_add(*b)))
}

pub(crate) fn request(opcode: u8) -> [u8; LEN] {
    let mut buf = [0u8; LEN];
    buf[0] = REPORT_ID;
    buf[1] = opcode;
    buf[LEN - 1] = checksum(&buf[1..LEN - 1]);
    buf
}

/// (percent, charging) from a GET_BATTERY reply.
pub(crate) fn parse_battery(r: &[u8]) -> Option<(u8, bool)> {
    let valid = r.len() >= LEN && r[0] == REPORT_ID && r[1] == GET_BATTERY && r[LEN - 1] == checksum(&r[1..LEN - 1]);
    (valid && (1..=100).contains(&r[6])).then(|| (r[6], r[7] == 1))
}

/// The dongle has a second 0xFF02:0x0002 collection with other report lengths.
fn is_command_channel(dev: &HidDevice) -> bool {
    let mut raw = vec![0u8; 4096];
    let Ok(n) = dev.get_report_descriptor(&mut raw) else { return false };
    let desc = parse(&raw[..n]);
    desc.report_len(ReportKind::Output, REPORT_ID) == Some(LEN) && desc.report_len(ReportKind::Input, REPORT_ID) == Some(LEN)
}

fn read_battery(dev: &HidDevice) -> Option<(u8, bool)> {
    let mut buf = [0u8; 64];
    while matches!(dev.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
    dev.write(&request(GET_BATTERY)).ok()?;
    let deadline = Instant::now() + Duration::from_millis(600);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let n = dev.read_timeout(&mut buf, left.as_millis().max(1) as i32).ok()?;
        if let Some(v) = parse_battery(&buf[..n]) {
            return Some(v);
        }
    }
    None
}

/// Dongles call themselves e.g. "Wireless mouse 8k dongle-L".
fn is_dongle(product: &str) -> bool {
    product.is_empty() || product.to_ascii_lowercase().contains("dongle")
}

impl Provider for Atk {
    fn id(&self) -> &'static str {
        "atk"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        let Some(api) = ctx.hid else { return vec![] };
        if ctx.full_scan {
            self.channel.clear();
        }
        let infos: Vec<DeviceInfo> =
            api.device_list().filter(|i| i.vendor_id() == VID && i.usage_page() == 0xFF02 && i.usage() == 0x0002).cloned().collect();
        let mut out = Vec::new();
        for info in infos {
            let path = path_of(&info);
            if self.channel.get(&path) == Some(&false) {
                continue;
            }
            let Ok(dev) = info.open_device(api) else { continue };
            if !*self.channel.entry(path).or_insert_with(|| is_command_channel(&dev)) {
                continue;
            }
            ctx.claim(&info);
            let Some((level, charging)) = read_battery(&dev) else { continue };
            ctx.claimed_ids.insert((VID, info.product_id()));

            let serial = info.serial_number().map(normalize_name).filter(|s| !s.is_empty());
            let key = format!("atk:{}", serial.unwrap_or_else(|| format!("{:04x}", info.product_id())));
            let product = info.product_string().unwrap_or("").trim();
            let dongle = is_dongle(product);
            if !dongle {
                self.names.insert(key.clone(), clean_name(product));
            }
            let charge = match (charging, level) {
                (true, 100) => Charge::Full,
                (true, _) => Charge::Charging,
                _ => Charge::Discharging,
            };
            out.push(Reading {
                name: self.names.get(&key).cloned().unwrap_or_else(|| "Wireless Mouse".into()),
                key,
                vendor: info.manufacturer_string().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
                kind: DeviceKind::Mouse,
                transport: if dongle { Transport::Receiver } else { Transport::Usb },
                batteries: vec![Battery::new(level, charge)],
                source: "atk",
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    #[test]
    fn request_checksum() {
        assert_eq!(request(GET_BATTERY).to_vec(), hex("08 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 49"));
    }

    #[test]
    fn parses_battery_replies() {
        // Captured from an ATK A9 Air on its 8K dongle (373B:11D9): 95 %, 4349 mV.
        assert_eq!(parse_battery(&hex("08 04 00 00 00 02 5F 00 10 FD 00 00 00 00 00 00 DB")), Some((95, false)));
        // From Techxey's protocol notes (373B:10C9): 45 %, 3687 mV.
        assert_eq!(parse_battery(&hex("08 04 00 00 00 02 2D 00 0E 67 00 00 00 00 00 00 A5")), Some((45, false)));
        // Bad checksum, and an unsolicited status frame.
        assert_eq!(parse_battery(&hex("08 04 00 00 00 02 5F 00 10 FD 00 00 00 00 00 00 DC")), None);
        let mut status = hex("08 1C 00 00 00 01 01 00 00 00 00 00 00 00 00 00 00");
        status[16] = checksum(&status[1..16]);
        assert_eq!(parse_battery(&status), None);
    }

    #[test]
    fn dongle_names() {
        assert!(is_dongle("Wireless mouse 8k dongle-L"));
        assert!(!is_dongle("ATK A9 Air"));
    }
}
