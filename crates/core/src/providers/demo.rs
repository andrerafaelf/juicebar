//! Simulated devices for UI development and screenshots (`JUICEBAR_DEMO=1`).
//!
//! Levels are anchored to the moment demo mode starts, so every launch shows the same
//! picture (a low headset, a charging controller…), then they drift realistically.
//! [`seed_history`] back-fills 24 h of samples so charts and estimates have data.

use super::{PollContext, Provider};
use crate::history::History;
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use crate::util::now_secs;

const HOUR: i64 = 3600;

pub struct Demo {
    start: u64,
}

impl Default for Demo {
    fn default() -> Self {
        Self { start: now_secs() }
    }
}

/// A battery that drains over 80 % of `period` and recharges in the rest. `phase`
/// places the cycle so that `dt = 0` (launch) shows a chosen level.
#[derive(Clone, Copy)]
struct Wave {
    period: i64,
    phase: i64,
}

impl Wave {
    const fn draining(level: i64, period_h: i64) -> Self {
        let period = period_h * HOUR;
        Self { period, phase: (100 - level) * (period * 4 / 5) / 100 }
    }

    const fn charging(level: i64, period_h: i64) -> Self {
        let period = period_h * HOUR;
        let drain = period * 4 / 5;
        Self { period, phase: drain + level * (period - drain) / 100 }
    }

    fn at(self, dt: i64) -> (u8, Charge) {
        let drain = self.period * 4 / 5;
        let t = (self.phase + dt).rem_euclid(self.period);
        if t < drain {
            ((100 - t * 100 / drain) as u8, Charge::Discharging)
        } else {
            match ((t - drain) * 100 / (self.period - drain)) as u8 {
                p if p >= 99 => (100, Charge::Full),
                p => (p, Charge::Charging),
            }
        }
    }
}

const MOUSE: Wave = Wave::draining(72, 60);
const KEYBOARD: Wave = Wave::draining(88, 240);
const HEADSET: Wave = Wave::draining(14, 30);
const GAMEPAD: Wave = Wave::charging(46, 10);
const CONTROLLER: Wave = Wave::draining(57, 40);

/// Every simulated device, `dt` seconds after launch (negative = before).
fn readings(dt: i64) -> Vec<Reading> {
    let r = |key: &str, name: &str, vendor: &str, kind, transport, batteries| Reading {
        key: key.into(),
        name: name.into(),
        vendor: Some(vendor.into()),
        kind,
        transport,
        batteries,
        source: "demo",
    };
    let one = |w: Wave| {
        let (level, charge) = w.at(dt);
        vec![Battery::new(level, charge)]
    };
    let coarse = |w: Wave, step: u8| {
        let (level, charge) = w.at(dt);
        vec![Battery::new(level / step * step, charge).coarse(step)]
    };
    vec![
        r("demo:mouse", "PRO X SUPERLIGHT 2", "Logitech", DeviceKind::Mouse, Transport::Receiver, one(MOUSE)),
        r("demo:kbd", "MX Keys S", "Logitech", DeviceKind::Keyboard, Transport::Ble, one(KEYBOARD)),
        r("demo:headset", "Arctis Nova 7", "SteelSeries", DeviceKind::Headset, Transport::Receiver, one(HEADSET)),
        r("demo:pad", "DualSense", "Sony", DeviceKind::Gamepad, Transport::Bluetooth, coarse(GAMEPAD, 10)),
        r("demo:xbox", "Xbox Wireless Controller", "Microsoft", DeviceKind::Gamepad, Transport::Bluetooth, one(CONTROLLER)),
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

/// Back-fills the last 24 h of samples for the simulated devices, as if demo mode had
/// been running since then.
pub fn seed_history(history: &mut History, now: u64) {
    for step in (1..=24 * 6).rev() {
        let dt = -(step as i64) * 600;
        for r in readings(dt) {
            if let Some(b) = r.primary() {
                history.record(&r.key, b.level, b.charge.is_charging(), now.saturating_add_signed(dt));
            }
        }
    }
}

impl Provider for Demo {
    fn id(&self) -> &'static str {
        "demo"
    }

    fn poll(&mut self, _ctx: &mut PollContext) -> Vec<Reading> {
        readings(now_secs() as i64 - self.start as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_levels_are_fixed() {
        assert_eq!(MOUSE.at(0), (72, Charge::Discharging));
        assert_eq!(HEADSET.at(0), (14, Charge::Discharging));
        assert_eq!(GAMEPAD.at(0), (46, Charge::Charging));
        let mut h = History::default();
        seed_history(&mut h, now_secs());
        let s = h.series("demo:headset");
        assert!(s.len() > 10 && s.first().unwrap().level > s.last().unwrap().level);
        assert!(h.estimate("demo:headset").is_some());
    }
}
