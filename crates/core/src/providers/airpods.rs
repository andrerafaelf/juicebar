//! Apple AirPods / Beats: left, right and case levels decoded from the BLE
//! "proximity pairing" advertisement (Apple manufacturer data, type 0x07).
//!
//! The scanner only runs while an AirPods/Beats device is connected, and only strong
//! (nearby) advertisements are accepted so a neighbour's earbuds are not picked up.

use super::{PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher, BluetoothLEScanningMode,
};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::DataReader;

const APPLE: u16 = 0x004C;
const MIN_RSSI: i16 = -65;
const MAX_AGE: Duration = Duration::from_secs(90);

#[derive(Default)]
struct Latest {
    at: Option<Instant>,
    rssi: i16,
    data: Vec<u8>,
}

#[derive(Default)]
pub struct AirPods {
    watcher: Option<(BluetoothLEAdvertisementWatcher, i64)>,
    latest: Arc<Mutex<Latest>>,
}

fn is_apple_audio(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("airpods") || n.contains("beats") || n.contains("powerbeats")
}

/// Decode proximity-pairing manufacturer data (starting at the 0x07 type byte).
pub(crate) fn decode(d: &[u8]) -> Option<Vec<Battery>> {
    if d.len() < 8 || d[0] != 0x07 {
        return None;
    }
    // Bit 5 of the status byte tells which pod is "primary"; nibbles swap with it.
    let flipped = (d[5] >> 4) & 0x02 == 0;
    let (hi, lo) = (d[6] >> 4, d[6] & 0x0F);
    let (left, right) = if flipped { (hi, lo) } else { (lo, hi) };
    let flags = d[7] >> 4;
    let case = d[7] & 0x0F;
    let (left_chg, right_chg) = if flipped { (flags & 0b10, flags & 0b01) } else { (flags & 0b01, flags & 0b10) };
    let cell = |label: &str, v: u8, charging: bool| -> Option<Battery> {
        (v <= 10).then(|| {
            Battery::new((v * 10).min(100), if charging { Charge::Charging } else { Charge::Discharging }).coarse(10).labeled(label)
        })
    };
    let cells: Vec<Battery> =
        [cell("Left", left, left_chg != 0), cell("Right", right, right_chg != 0), cell("Case", case, flags & 0b100 != 0)]
            .into_iter()
            .flatten()
            .collect();
    (!cells.is_empty()).then_some(cells)
}

impl AirPods {
    fn start(&mut self) {
        if self.watcher.is_some() {
            return;
        }
        let Ok(watcher) = BluetoothLEAdvertisementWatcher::new() else { return };
        let _ = watcher.SetScanningMode(BluetoothLEScanningMode::Passive);
        let latest = self.latest.clone();
        let handler =
            TypedEventHandler::<BluetoothLEAdvertisementWatcher, BluetoothLEAdvertisementReceivedEventArgs>::new(move |_, args| {
                let Some(args) = args.as_ref() else { return Ok(()) };
                let rssi = args.RawSignalStrengthInDBm()?;
                if rssi < MIN_RSSI {
                    return Ok(());
                }
                for md in args.Advertisement()?.ManufacturerData()? {
                    if md.CompanyId()? != APPLE {
                        continue;
                    }
                    let buf = md.Data()?;
                    let mut bytes = vec![0u8; buf.Length()? as usize];
                    DataReader::FromBuffer(&buf)?.ReadBytes(&mut bytes)?;
                    if decode(&bytes).is_none() {
                        continue;
                    }
                    let mut l = latest.lock().unwrap();
                    let fresh = l.at.is_some_and(|t| t.elapsed() < Duration::from_secs(10));
                    // Prefer the strongest signal among recent packets.
                    if !fresh || rssi >= l.rssi {
                        *l = Latest { at: Some(Instant::now()), rssi, data: bytes };
                    }
                }
                Ok(())
            });
        let Ok(token) = watcher.Received(&handler) else { return };
        if watcher.Start().is_ok() {
            self.watcher = Some((watcher, token));
        }
    }

    fn stop(&mut self) {
        if let Some((w, token)) = self.watcher.take() {
            let _ = w.Stop();
            let _ = w.RemoveReceived(token);
        }
    }
}

impl Drop for AirPods {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Provider for AirPods {
    fn id(&self) -> &'static str {
        "airpods"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        let Some(peer) = ctx.bt_connected.iter().find(|p| is_apple_audio(&p.name)).cloned() else {
            self.stop();
            return vec![];
        };
        self.start();
        let l = self.latest.lock().unwrap();
        if !l.at.is_some_and(|t| t.elapsed() < MAX_AGE) {
            return vec![];
        }
        let Some(batteries) = decode(&l.data) else { return vec![] };
        let max = peer.name.to_ascii_lowercase().contains("max");
        vec![Reading {
            key: peer.key,
            name: peer.name,
            vendor: Some("Apple".into()),
            kind: if max { DeviceKind::Headset } else { DeviceKind::Earbuds },
            transport: Transport::Bluetooth,
            batteries,
            source: "airpods",
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_pods_and_case() {
        // type, len, prefix, model(2), status=0x75 (bit 5 set → not flipped), pods=0x89, flags/case=0x16
        let d = [0x07, 0x19, 0x01, 0x14, 0x20, 0x75, 0x89, 0x16];
        let b = decode(&d).unwrap();
        assert_eq!(b.len(), 3);
        // not flipped: left = low nibble (9), right = high nibble (8)
        assert_eq!((b[0].label.as_deref(), b[0].level, b[0].charge), (Some("Left"), 90, Charge::Charging));
        assert_eq!((b[1].label.as_deref(), b[1].level), (Some("Right"), 80));
        assert_eq!((b[2].label.as_deref(), b[2].level), (Some("Case"), 60));
        // unknown values (0xF) are dropped
        assert_eq!(decode(&[0x07, 0x19, 0x01, 0x14, 0x20, 0x55, 0xFF, 0x0F]), None);
    }
}
