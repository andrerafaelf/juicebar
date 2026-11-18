//! Logitech G435 LIGHTSPEED headset. Its dongles have no HID++ interface: the battery
//! sits behind the dongle's radio SoC (Airoha/Audiowise), reached through a vendor
//! bridge.
//!
//! * `046D:0ACB` (Realtek ALC4021 USB side): vendor collection 0xFFC0, report 6. The
//!   bridge must be put in "bootloader" mode (`06 01 03`), commands are passed with
//!   `06 04 03 <len16> <cmd>` as 514-byte output reports, replies are polled with
//!   GET_FEATURE, and `06 08 03` leaves the mode again. The dongle firmware pauses the
//!   audio streams while in that mode, so a read costs roughly half a second of audio:
//!   this provider therefore reads rarely (configurable) and reports the cached value
//!   in between.
//! * `046D:0ADF`: vendor collection 0xFF00, plain 64-byte frames with a CRC-16.
//!
//! SoC commands are `"PA" <category> <seq> <len> <command…>`, replies start with "PI".
//! GET_BATTERY returns millivolts, converted with a Li-Po discharge curve. GET_BATTERY
//! must never be the first command of a session (it can hang the 0ACB dongle until it is
//! replugged), and it only reaches the headset in remote mode 1.
//!
//! Protocol research: endes ("USB HID down the rabbit hole: Logitech G435 dongle"),
//! deddroid/logitech-g435-battery and portyporty/g435-battery-percentage (MIT).

use super::{PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use crate::util::lipo_percent;
use hidapi::{DeviceInfo, HidDevice};
use std::thread::sleep;
use std::time::{Duration, Instant};

const VID: u16 = 0x046D;
const PID_ACB: u16 = 0x0ACB;
const PID_ADF: u16 = 0x0ADF;

const CAT_OTA: u8 = 0x06;
const CAT_WIRELESS: u8 = 0x0E;
const GET_BATTERY: [u8; 2] = [0x00, 0x31];
const REMOTE_SET_MODE: u8 = 0xE1;
const REMOTE_GET_CONNECT_STATUS: u8 = 0xE3;

/// After a manual refresh or a device change, read again if the cache is older than this.
const MIN_REFRESH: Duration = Duration::from_secs(60);

#[derive(Debug)]
enum Fail {
    /// The dongle answered that no headset is connected.
    HeadsetOff,
    NoReply,
}

impl From<hidapi::HidError> for Fail {
    fn from(_: hidapi::HidError) -> Self {
        Fail::NoReply
    }
}

type Res<T> = Result<T, Fail>;

pub struct G435 {
    /// `None` disables reading (each read briefly pauses the headset audio).
    interval: Option<Duration>,
    last_try: Option<Instant>,
    cached: Option<(Instant, Reading)>,
}

impl G435 {
    pub fn new(interval: Option<Duration>) -> Self {
        Self { interval, last_try: None, cached: None }
    }

    fn due(&self, full_scan: bool) -> bool {
        let (Some(interval), Some(last)) = (self.interval, self.last_try) else {
            return self.interval.is_some();
        };
        let age = last.elapsed();
        age >= interval || (full_scan && age >= MIN_REFRESH)
    }
}

// ---------------------------------------------------------------------------------------
// Pure protocol helpers

fn pa_command(category: u8, seq: u8, len: u8, payload: &[u8]) -> Vec<u8> {
    let mut cmd = vec![0x50, 0x41, category, seq, len];
    cmd.extend_from_slice(payload);
    cmd
}

fn next_seq(seq: &mut u8) -> u8 {
    let s = *seq;
    *seq = *seq % 127 + 1;
    s
}

/// GET_BATTERY reply anywhere in `buf`: `07 00 06 <seq|80> 00 <mV lo> <mV hi>`.
pub(crate) fn parse_battery_raw(buf: &[u8]) -> Option<u16> {
    buf.windows(7).find_map(|w| {
        (w[0] == 0x07 && w[1] == 0x00 && w[2] == CAT_OTA && w[3] & 0x80 != 0 && w[4] == 0x00).then(|| u16::from_le_bytes([w[5], w[6]]))
    })
}

/// In remote mode 0 the dongle answers GET_BATTERY itself with a meaningless value.
pub(crate) fn is_battery_mv(mv: u16) -> bool {
    (2500..=4600).contains(&mv)
}

/// Value of a wireless-category "get" reply: `02 00 <cmd> <value>`.
pub(crate) fn parse_remote_value(buf: &[u8], cmd: u8) -> Option<u8> {
    buf.windows(4).find(|w| w[0] == 0x02 && w[1] == 0x00 && w[2] == cmd).map(|w| w[3])
}

fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// 0ADF frame: `cmd, body_len, crc_lo, crc_hi, body…`, CRC over the frame with a zero CRC field.
fn adf_frame(cmd: u8, body: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    f[0] = cmd;
    f[1] = body.len() as u8;
    f[4..4 + body.len()].copy_from_slice(body);
    let crc = crc16_xmodem(&f);
    f[2..4].copy_from_slice(&crc.to_le_bytes());
    f
}

// ---------------------------------------------------------------------------------------
// 046D:0ACB, Realtek bridge on report 6

mod acb {
    use super::*;

    const REPORT: u8 = 6;
    const FEATURE_LEN: usize = 64;
    const OUTPUT_LEN: usize = 515;
    const ENTER: [u8; 2] = [0x01, 0x03];
    const LEAVE: [u8; 2] = [0x08, 0x03];
    const SEND: [u8; 2] = [0x04, 0x03];

    /// Sends a bridge message and waits for its one-byte ACK on the input endpoint
    /// (volume-key reports may be interleaved).
    fn bridge(dev: &HidDevice, op: [u8; 2], body: &[u8]) -> Res<u8> {
        let mut report = [0u8; OUTPUT_LEN];
        report[0] = REPORT;
        report[1..3].copy_from_slice(&op);
        report[3..3 + body.len()].copy_from_slice(body);
        // hidapi's send_output_report() sizes its buffer for the feature report on
        // Windows; write() uses the output report length.
        dev.write(&report)?;
        let deadline = Instant::now() + Duration::from_millis(800);
        let mut buf = [0u8; 64];
        while Instant::now() < deadline {
            let n = dev.read_timeout(&mut buf, 100)?;
            if n >= 2 && buf[0] == REPORT {
                return Ok(buf[1]);
            }
        }
        Err(Fail::NoReply)
    }

    /// Always leaves the bootloader mode, which is what pauses the audio.
    struct Session<'a> {
        dev: &'a HidDevice,
        seq: u8,
    }

    impl Drop for Session<'_> {
        fn drop(&mut self) {
            let _ = bridge(self.dev, LEAVE, &[]);
        }
    }

    impl Session<'_> {
        fn query<T>(&mut self, category: u8, len: u8, payload: &[u8], parse: impl Fn(&[u8]) -> Option<T>) -> Res<Option<T>> {
            let cmd = pa_command(category, next_seq(&mut self.seq), len, payload);
            let mut msg = (cmd.len() as u16).to_le_bytes().to_vec();
            msg.extend_from_slice(&cmd);
            bridge(self.dev, SEND, &msg)?;
            let deadline = Instant::now() + Duration::from_millis(1500);
            while Instant::now() < deadline {
                let mut buf = [0u8; FEATURE_LEN];
                buf[0] = REPORT;
                let n = self.dev.get_feature_report(&mut buf)?;
                let buf = &buf[..n];
                // An empty reply buffer reads `06 ff ff …`.
                if buf.get(1).is_some_and(|&b| b != 0xFF) {
                    if let Some(v) = parse(buf) {
                        return Ok(Some(v));
                    }
                    continue;
                }
                sleep(Duration::from_millis(30));
            }
            Ok(None)
        }

        fn battery(&mut self) -> Res<Option<u16>> {
            self.query(CAT_OTA, 1, &GET_BATTERY, parse_battery_raw)
        }

        fn drain(&self) {
            for _ in 0..16 {
                let mut buf = [0u8; FEATURE_LEN];
                buf[0] = REPORT;
                match self.dev.get_feature_report(&mut buf) {
                    Ok(n) if n >= 2 && buf[1] != 0xFF => {}
                    _ => break,
                }
            }
        }
    }

    pub fn read_millivolts(dev: &HidDevice) -> Res<u16> {
        let mut buf = [0u8; 64];
        while matches!(dev.read_timeout(&mut buf, 0), Ok(n) if n > 0) {}
        let mut s = Session { dev, seq: 1 };
        bridge(dev, ENTER, &[])?; // 0x5a = ok, 0xfe = already in
        s.drain();
        // Never start a session with GET_BATTERY (it can hang the dongle).
        let connected = s.query(CAT_WIRELESS, 1, &[REMOTE_GET_CONNECT_STATUS], |b| parse_remote_value(b, REMOTE_GET_CONNECT_STATUS))?;
        if connected == Some(0) {
            return Err(Fail::HeadsetOff);
        }
        if let Some(mv) = s.battery()?.filter(|&mv| is_battery_mv(mv)) {
            return Ok(mv);
        }
        // Remote mode 1 routes the command to the headset; the dongle starts in mode 0.
        s.query(CAT_WIRELESS, 2, &[REMOTE_SET_MODE, 0x01], |_| Some(()))?;
        s.battery()?.filter(|&mv| is_battery_mv(mv)).ok_or(Fail::NoReply)
    }
}

// ---------------------------------------------------------------------------------------
// 046D:0ADF, CRC-framed 64-byte reports

mod adf {
    use super::*;

    /// Session init (OTA_CMD_IOCTL_READ_MAX_LEN, seq 1).
    const SESSION_INIT: [u8; 19] =
        [0x50, 0x41, 0x06, 0x01, 0x0D, 0x00, 0x25, 0x34, 0x12, 0x5A, 0x5A, 0x01, 0x00, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00];

    fn transact(dev: &HidDevice, frame: &[u8; 64], wait_ms: u64) -> Res<Vec<u8>> {
        let mut out = [0u8; 65];
        out[1..].copy_from_slice(frame);
        dev.write(&out)?;
        let mut replies = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        let mut buf = [0u8; 65];
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let n = dev.read_timeout(&mut buf, left.as_millis().max(1) as i32)?;
            replies.extend_from_slice(&buf[..n]);
        }
        Ok(replies)
    }

    fn pa(seq: &mut u8, category: u8, len: u8, payload: &[u8]) -> [u8; 64] {
        adf_frame(0x80, &pa_command(category, next_seq(seq), len, payload))
    }

    pub fn read_millivolts(dev: &HidDevice) -> Res<u16> {
        transact(dev, &adf_frame(0x84, &[0x01]), 50)?;
        transact(dev, &adf_frame(0x85, &[0x01]), 50)?;
        transact(dev, &adf_frame(0x80, &SESSION_INIT), 450)?;
        let mut seq = 2;
        let status = transact(dev, &pa(&mut seq, CAT_WIRELESS, 1, &[REMOTE_GET_CONNECT_STATUS]), 120);
        let result = (|| {
            if parse_remote_value(&status?, REMOTE_GET_CONNECT_STATUS) == Some(0) {
                return Err(Fail::HeadsetOff);
            }
            transact(dev, &pa(&mut seq, CAT_WIRELESS, 2, &[REMOTE_SET_MODE, 0x01]), 120)?;
            let replies = transact(dev, &pa(&mut seq, CAT_OTA, 1, &GET_BATTERY), 250)?;
            parse_battery_raw(&replies).filter(|&mv| is_battery_mv(mv)).ok_or(Fail::NoReply)
        })();
        let _ = transact(dev, &adf_frame(0x85, &[0x02]), 50);
        result
    }
}

// ---------------------------------------------------------------------------------------

fn vendor_collection(info: &DeviceInfo) -> bool {
    info.vendor_id() == VID
        && match info.product_id() {
            PID_ACB => info.usage_page() == 0xFFC0,
            PID_ADF => info.usage_page() == 0xFF00,
            _ => false,
        }
}

impl Provider for G435 {
    fn id(&self) -> &'static str {
        "g435"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        let Some(api) = ctx.hid else { return vec![] };
        let dongles: Vec<DeviceInfo> =
            api.device_list().filter(|i| i.vendor_id() == VID && matches!(i.product_id(), PID_ACB | PID_ADF)).cloned().collect();
        // Keep the generic reader away from every collection of the dongle.
        for info in &dongles {
            ctx.claim(info);
        }
        let Some(info) = dongles.iter().find(|i| vendor_collection(i)) else {
            self.cached = None;
            return vec![];
        };
        let Some(interval) = self.interval else { return vec![] };

        if self.due(ctx.full_scan) {
            self.last_try = Some(Instant::now());
            let result = match info.open_device(api) {
                Ok(dev) if info.product_id() == PID_ACB => acb::read_millivolts(&dev),
                Ok(dev) => adf::read_millivolts(&dev),
                Err(_) => Err(Fail::NoReply),
            };
            match result {
                Ok(mv) => {
                    // Voltage only approximates the charge: round to 5 % and mark as coarse.
                    let level = ((lipo_percent(mv) as u32 + 2) / 5 * 5).min(100) as u8;
                    let reading = Reading {
                        key: format!("g435:{:04x}:{:04x}", info.vendor_id(), info.product_id()),
                        name: "Logitech G435".into(),
                        vendor: Some("Logitech".into()),
                        kind: DeviceKind::Headset,
                        transport: Transport::Receiver,
                        batteries: vec![Battery::new(level, Charge::Unknown).coarse(5)],
                        source: "g435",
                    };
                    self.cached = Some((Instant::now(), reading));
                }
                Err(Fail::HeadsetOff) => self.cached = None,
                // A missed reply keeps the last value for a while.
                Err(Fail::NoReply) => {
                    if self.cached.as_ref().is_some_and(|(t, _)| t.elapsed() > interval * 2) {
                        self.cached = None;
                    }
                }
            }
        }
        if let Some((_, r)) = &self.cached {
            ctx.claimed_ids.insert((VID, info.product_id()));
            return vec![r.clone()];
        }
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn parses_battery_reply() {
        // Captured from a 0ACB dongle (feature report 6).
        let buf = hex("06504901c875480d00070006820032100000ffffff");
        assert_eq!(parse_battery_raw(&buf), Some(4146));
        // Same reply in remote mode 0: not a headset voltage.
        let buf = hex("06504901c0d64300000700068200d2000000ffff");
        assert_eq!(parse_battery_raw(&buf), Some(210));
        assert!(!is_battery_mv(210));
    }

    #[test]
    fn parses_remote_values() {
        let buf = hex("0650490ec2000000000200e301ffff");
        assert_eq!(parse_remote_value(&buf, REMOTE_GET_CONNECT_STATUS), Some(1));
        assert_eq!(parse_remote_value(&buf, 0xE0), None);
    }

    #[test]
    fn frames() {
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
        let f = adf_frame(0x84, &[0x01]);
        let mut zeroed = f;
        zeroed[2] = 0;
        zeroed[3] = 0;
        assert_eq!(crc16_xmodem(&zeroed).to_le_bytes(), [f[2], f[3]]);
        assert_eq!(pa_command(CAT_OTA, 2, 1, &GET_BATTERY), [0x50, 0x41, 0x06, 0x02, 0x01, 0x00, 0x31]);
    }

    #[test]
    fn rate_limited() {
        let mut p = G435::new(Some(Duration::from_secs(600)));
        assert!(p.due(false));
        p.last_try = Some(Instant::now());
        assert!(!p.due(false) && !p.due(true));
        assert!(!G435::new(None).due(true));
    }
}
