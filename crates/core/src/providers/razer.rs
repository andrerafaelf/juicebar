//! Razer wireless mice and keyboards (and their dongles).
//!
//! Razer peripherals answer a 90-byte feature-report command protocol. Only two read
//! commands are used here: class 0x07 / id 0x80 (battery level, 0..=255) and
//! class 0x07 / id 0x84 (charging flag).

use super::{path_of, product_name, transport_of, PollContext, Provider};
use crate::model::{Battery, Charge, Reading, Transport};
use crate::util::{clean_name, guess_kind, normalize_name};
use hidapi::{DeviceInfo, HidDevice};
use std::collections::HashMap;
use std::thread::sleep;
use std::time::Duration;

pub const VID: u16 = 0x1532;
const REPORT_LEN: usize = 90;
const STATUS_OK: u8 = 0x02;
const STATUS_BUSY: u8 = 0x01;
/// Transaction ids used by different device generations.
const TRANSACTION_IDS: [u8; 3] = [0x1F, 0x3F, 0xFF];

#[derive(Default)]
pub struct Razer {
    /// Paths that do not answer the battery command, with the poll number to retry at.
    negative: HashMap<String, u64>,
    /// Working transaction id per product id.
    tid: HashMap<u16, u8>,
    polls: u64,
}

pub(crate) fn build(tid: u8, class: u8, id: u8, size: u8) -> [u8; REPORT_LEN + 1] {
    let mut buf = [0u8; REPORT_LEN + 1]; // buf[0] = report id 0
    let r = &mut buf[1..];
    r[1] = tid;
    r[5] = size;
    r[6] = class;
    r[7] = id;
    r[88] = crc(r);
    buf
}

/// XOR of bytes 2..88 of the 90-byte report.
pub(crate) fn crc(report: &[u8]) -> u8 {
    report[2..88].iter().fold(0, |a, b| a ^ b)
}

fn query(dev: &HidDevice, tid: u8, class: u8, id: u8) -> Option<[u8; REPORT_LEN]> {
    let req = build(tid, class, id, 0x02);
    for _attempt in 0..3 {
        dev.send_feature_report(&req).ok()?;
        sleep(Duration::from_millis(40));
        let mut buf = [0u8; REPORT_LEN + 1];
        let n = dev.get_feature_report(&mut buf).ok()?;
        if n < REPORT_LEN {
            return None;
        }
        // hidapi may or may not keep the report-id byte; normalise to the 90-byte report.
        let start = n - REPORT_LEN;
        let mut r = [0u8; REPORT_LEN];
        r.copy_from_slice(&buf[start..start + REPORT_LEN]);
        match r[0] {
            STATUS_OK if r[6] == class && r[7] == id => return Some(r),
            STATUS_BUSY => sleep(Duration::from_millis(60)),
            _ => return None,
        }
    }
    None
}

fn read_battery(dev: &HidDevice, tid: u8) -> Option<Battery> {
    let level = query(dev, tid, 0x07, 0x80)?;
    let pct = ((level[9] as u32 * 100 + 127) / 255) as u8;
    let charging = query(dev, tid, 0x07, 0x84).is_some_and(|r| r[9] == 1);
    let charge = match (charging, pct) {
        (true, 100) => Charge::Full,
        (true, _) => Charge::Charging,
        _ => Charge::Discharging,
    };
    Some(Battery::new(pct, charge))
}

impl Provider for Razer {
    fn id(&self) -> &'static str {
        "razer"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        self.polls += 1;
        let Some(api) = ctx.hid else { return vec![] };
        let mut infos: Vec<DeviceInfo> = api.device_list().filter(|i| i.vendor_id() == VID).cloned().collect();
        // Interface 0 usually hosts the control endpoint.
        infos.sort_by_key(|i| i.interface_number());

        let mut done: Vec<u16> = Vec::new();
        let mut out = Vec::new();
        for info in infos {
            let pid = info.product_id();
            if done.contains(&pid) {
                ctx.claim(&info);
                continue;
            }
            let path = path_of(&info);
            if !ctx.full_scan && self.negative.get(&path).is_some_and(|retry| self.polls < *retry) {
                continue;
            }
            let Ok(dev) = info.open_device(api) else { continue };
            let tids: Vec<u8> = match self.tid.get(&pid) {
                Some(t) => vec![*t],
                None => TRANSACTION_IDS.to_vec(),
            };
            let found = tids.into_iter().find_map(|t| read_battery(&dev, t).map(|b| (t, b)));
            match found {
                Some((tid, battery)) => {
                    self.tid.insert(pid, tid);
                    done.push(pid);
                    ctx.claim(&info);
                    ctx.claimed_ids.insert((VID, pid));
                    let name = clean_name(&product_name(&info));
                    let transport = match transport_of(&info) {
                        Transport::Usb if product_name(&info).to_ascii_lowercase().contains("dongle") => Transport::Receiver,
                        t => t,
                    };
                    out.push(Reading {
                        key: format!("razer:{pid:04x}:{}", normalize_name(&name)),
                        kind: guess_kind(&name).or_mouse(),
                        name,
                        vendor: Some("Razer".into()),
                        transport,
                        batteries: vec![battery],
                        source: "razer",
                    });
                }
                None => {
                    self.negative.insert(path, self.polls + 30);
                }
            }
        }
        out
    }
}

trait OrMouse {
    fn or_mouse(self) -> Self;
}
impl OrMouse for crate::model::DeviceKind {
    /// Most Razer wireless devices with this protocol are mice.
    fn or_mouse(self) -> Self {
        if self == crate::model::DeviceKind::Other {
            crate::model::DeviceKind::Mouse
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_layout_and_crc() {
        let buf = build(0x1F, 0x07, 0x80, 0x02);
        assert_eq!(buf.len(), 91);
        assert_eq!(buf[0], 0); // report id
        assert_eq!(&buf[1..9], &[0x00, 0x1F, 0, 0, 0, 0x02, 0x07, 0x80]);
        // crc covers bytes 2..88 of the report: 0x02 ^ 0x07 ^ 0x80
        assert_eq!(buf[89], 0x02 ^ 0x07 ^ 0x80);
    }
}
