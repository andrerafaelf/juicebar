//! Simulated devices for UI development and screenshots (`JUICEBAR_DEMO=1`).

use super::{PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use crate::util::now_secs;

#[derive(Default)]
pub struct Demo;

/// A level that drains over `period` seconds and then recharges, deterministic in time.
fn wave(offset: u64, period: u64) -> (u8, Charge) {
    let t = (now_secs() + offset) % period;
    let drain = period * 4 / 5;
    if t < drain {
        (100 - (t * 100 / drain) as u8, Charge::Discharging)
    } else {
        let p = ((t - drain) * 100 / (period - drain)) as u8;
        if p >= 99 {
            (100, Charge::Full)
        } else {
            (p, Charge::Charging)
        }
    }
}

impl Provider for Demo {
    fn id(&self) -> &'static str {
        "demo"
    }

    fn poll(&mut self, _ctx: &mut PollContext) -> Vec<Reading> {
        let r = |key: &str, name: &str, vendor: &str, kind, transport, batteries| Reading {
            key: key.into(),
            name: name.into(),
            vendor: Some(vendor.into()),
            kind,
            transport,
            batteries,
            source: "demo",
        };
        let (mouse, mc) = wave(0, 6 * 3600);
        let (kb, kc) = wave(9000, 30 * 3600);
        let (pad, pc) = wave(2000, 3 * 3600);
        vec![
            r("demo:mouse", "PRO X SUPERLIGHT 2", "Logitech", DeviceKind::Mouse, Transport::Receiver, vec![Battery::new(mouse, mc)]),
            r("demo:kbd", "MX Keys S", "Logitech", DeviceKind::Keyboard, Transport::Ble, vec![Battery::new(kb, kc)]),
            r(
                "demo:headset",
                "Arctis Nova 7",
                "SteelSeries",
                DeviceKind::Headset,
                Transport::Receiver,
                vec![Battery::new(14, Charge::Discharging)],
            ),
            r("demo:pad", "DualSense", "Sony", DeviceKind::Gamepad, Transport::Bluetooth, vec![Battery::new(pad, pc).coarse(10)]),
            r(
                "demo:buds",
                "AirPods Pro",
                "Apple",
                DeviceKind::Earbuds,
                Transport::Bluetooth,
                vec![
                    Battery::new(80, Charge::Discharging).coarse(10).labeled("Left"),
                    Battery::new(70, Charge::Discharging).coarse(10).labeled("Right"),
                    Battery::new(40, Charge::Charging).coarse(10).labeled("Case"),
                ],
            ),
        ]
    }
}
