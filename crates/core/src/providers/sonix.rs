//! SONiX-based 2.4 GHz keyboard dongles (AJAZZ AK820 MAX, AULA F108 Pro and other
//! boards shipping the `05AC:024F` "2.4G Dongle" or `0C45:FEFE`).
//!
//! The dongle exposes a 32-byte raw-HID channel (usage page 0xFF60, usage 0x61).
//! Battery query: `20 01 00… <sum>`; the reply echoes `20 01` and carries the level in
//! byte 3, with the same additive checksum in byte 31. A level of 0 means the keyboard
//! is off or out of range. Protocol documented by the sonix-keyboard-studio project.

use super::{PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use hidapi::{DeviceInfo, HidDevice};
use std::time::{Duration, Instant};

const DONGLES: [(u16, u16); 2] = [(0x05AC, 0x024F), (0x0C45, 0xFEFE)];
const LEN: usize = 32;

#[derive(Default)]
pub struct Sonix {
    /// Poll number before which a silent dongle is not queried again.
    quiet_until: u64,
    polls: u64,
}

fn checksum(p: &[u8]) -> u8 {
    p[..31].iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

pub(crate) fn request() -> [u8; LEN + 1] {
    let mut buf = [0u8; LEN + 1]; // [0] = report id 0
    buf[1] = 0x20;
    buf[2] = 0x01;
    buf[LEN] = checksum(&buf[1..]);
    buf
}

/// Returns the level from a 32-byte reply payload.
pub(crate) fn parse(r: &[u8]) -> Option<u8> {
    (r.len() >= LEN && r[0] == 0x20 && r[1] == 0x01 && r[31] == checksum(r)).then_some(r[3])
}

fn query(dev: &HidDevice) -> Option<u8> {
    let mut buf = [0u8; 64];
    while matches!(dev.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
    dev.write(&request()).ok()?;
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        let n = dev.read_timeout(&mut buf, 100).ok()?;
        if n >= LEN {
            if let Some(level) = parse(&buf[..n]) {
                return Some(level);
            }
        }
    }
    None
}

fn matches(info: &DeviceInfo) -> bool {
    DONGLES.contains(&(info.vendor_id(), info.product_id())) && info.usage_page() == 0xFF60 && info.usage() == 0x61
}

impl Provider for Sonix {
    fn id(&self) -> &'static str {
        "sonix"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        let Some(api) = ctx.hid else { return vec![] };
        self.polls += 1;
        let infos: Vec<DeviceInfo> = api.device_list().filter(|i| matches(i)).cloned().collect();
        let mut out = Vec::new();
        for info in infos {
            ctx.claim(&info);
            let Ok(dev) = info.open_device(api) else { continue };
            // The first request can go unanswered while the radio link wakes up.
            if self.polls < self.quiet_until && !ctx.full_scan {
                continue;
            }
            let Some(level) = query(&dev).or_else(|| query(&dev)) else {
                self.quiet_until = self.polls + 3;
                continue;
            };
            if level == 0 || level > 100 {
                continue;
            }
            ctx.claimed_ids.insert((info.vendor_id(), info.product_id()));
            let product = info.product_string().unwrap_or("").trim();
            let name = match product {
                "" | "2.4G Dongle" => "2.4 GHz Keyboard".to_string(),
                p => p.trim_end_matches(" 2.4G").to_string(),
            };
            out.push(Reading {
                key: format!("sonix:{:04x}:{:04x}", info.vendor_id(), info.product_id()),
                name,
                vendor: None,
                kind: DeviceKind::Keyboard,
                transport: Transport::Receiver,
                batteries: vec![Battery::new(level, Charge::Unknown)],
                source: "sonix",
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_reply() {
        let req = request();
        assert_eq!(&req[..3], &[0x00, 0x20, 0x01]);
        assert_eq!(req[32], 0x21);
        let mut reply = [0u8; 32];
        reply[0] = 0x20;
        reply[1] = 0x01;
        reply[3] = 77;
        reply[31] = checksum(&reply);
        assert_eq!(parse(&reply), Some(77));
        reply[31] ^= 1;
        assert_eq!(parse(&reply), None);
    }
}
