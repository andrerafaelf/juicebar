//! HyperX wireless headsets on HP's vendor id (Cloud Alpha Wireless, `03F0:098D`).
//!
//! The dongle exposes a vendor collection (usage page 0xFF43) with 31-byte input and
//! output reports, report id 0x21. A request is `21 BB <cmd>` padded with zeros; the reply
//! echoes those three bytes and carries the value in byte 3:
//!
//! * `03` connection: 1 = headset off, 2 = connected
//! * `0C` charging: 1 = charging
//! * `0B` battery level in percent
//!
//! Only these reads are sent (`10`..`13` change sidetone, auto-off and voice prompts).
//! Protocol knowledge from HeadsetControl.

use super::{PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use hidapi::{DeviceInfo, HidDevice};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const VID: u16 = 0x03F0;
const HEADSETS: [(u16, &str); 1] = [(0x098D, "HyperX Cloud Alpha Wireless")];
const USAGE_PAGE: u16 = 0xFF43;
const REPORT_ID: u8 = 0x21;
const LEN: usize = 31;

const GET_CONNECTION: u8 = 0x03;
const GET_BATTERY: u8 = 0x0B;
const GET_CHARGING: u8 = 0x0C;
const HEADSET_OFF: u8 = 0x01;

#[derive(Default)]
pub struct HyperX {
    /// Last level per product id, kept for polls where a charging headset reports none.
    last_level: HashMap<u16, u8>,
}

pub(crate) fn request(cmd: u8) -> [u8; LEN] {
    let mut buf = [0u8; LEN];
    buf[0] = REPORT_ID;
    buf[1] = 0xBB;
    buf[2] = cmd;
    buf
}

/// Value byte of the reply to `cmd`.
pub(crate) fn parse(reply: &[u8], cmd: u8) -> Option<u8> {
    (reply.len() >= 4 && reply[..3] == [REPORT_ID, 0xBB, cmd]).then(|| reply[3])
}

fn query(dev: &HidDevice, cmd: u8) -> Option<u8> {
    dev.write(&request(cmd)).ok()?;
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut buf = [0u8; 64];
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let n = dev.read_timeout(&mut buf, left.as_millis().max(1) as i32).ok()?;
        if let Some(v) = parse(&buf[..n], cmd) {
            return Some(v);
        }
    }
    None
}

fn read_battery(dev: &HidDevice, last_level: Option<u8>) -> Option<Battery> {
    let mut buf = [0u8; 64];
    while matches!(dev.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
    if query(dev, GET_CONNECTION)? == HEADSET_OFF {
        return None;
    }
    let charging = query(dev, GET_CHARGING) == Some(1);
    let level = query(dev, GET_BATTERY).filter(|l| (1..=100).contains(l)).or(last_level.filter(|_| charging))?;
    let charge = match (charging, level) {
        (true, 100) => Charge::Full,
        (true, _) => Charge::Charging,
        _ => Charge::Discharging,
    };
    Some(Battery::new(level, charge))
}

fn headset_name(pid: u16) -> Option<&'static str> {
    HEADSETS.iter().find(|(p, _)| *p == pid).map(|(_, name)| *name)
}

impl Provider for HyperX {
    fn id(&self) -> &'static str {
        "hyperx"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        let Some(api) = ctx.hid else { return vec![] };
        let infos: Vec<DeviceInfo> = api
            .device_list()
            .filter(|i| i.vendor_id() == VID && i.usage_page() == USAGE_PAGE && headset_name(i.product_id()).is_some())
            .cloned()
            .collect();
        let mut out = Vec::new();
        for info in infos {
            ctx.claim(&info);
            let pid = info.product_id();
            let Ok(dev) = info.open_device(api) else { continue };
            let Some(battery) = read_battery(&dev, self.last_level.get(&pid).copied()) else { continue };
            self.last_level.insert(pid, battery.level);
            ctx.claimed_ids.insert((VID, pid));
            out.push(Reading {
                key: format!("hyperx:{VID:04x}:{pid:04x}"),
                name: headset_name(pid).unwrap_or("HyperX Headset").into(),
                vendor: Some("HyperX".into()),
                kind: DeviceKind::Headset,
                transport: Transport::Receiver,
                batteries: vec![battery],
                source: "hyperx",
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(head: &[u8]) -> [u8; LEN] {
        let mut r = [0u8; LEN];
        r[..head.len()].copy_from_slice(head);
        r
    }

    #[test]
    fn request_layout() {
        let req = request(GET_BATTERY);
        assert_eq!(&req[..4], &[0x21, 0xBB, 0x0B, 0x00]);
        assert!(req[3..].iter().all(|&b| b == 0));
    }

    #[test]
    fn parses_replies() {
        // Captured from a Cloud Alpha Wireless dongle with the headset on, 40 %.
        assert_eq!(parse(&reply(&[0x21, 0xBB, 0x03, 0x02]), GET_CONNECTION), Some(2));
        assert_eq!(parse(&reply(&[0x21, 0xBB, 0x0C, 0x00]), GET_CHARGING), Some(0));
        assert_eq!(parse(&reply(&[0x21, 0xBB, 0x0B, 0x28, 0x0E, 0xBE, 0x01]), GET_BATTERY), Some(40));
        // A reply to another command is not the answer.
        assert_eq!(parse(&reply(&[0x21, 0xBB, 0x0C, 0x01]), GET_BATTERY), None);
        assert_eq!(parse(&[0x21, 0xBB], GET_BATTERY), None);
    }
}
