//! Low-battery and fully-charged notifications, with hysteresis so a device hovering
//! around the threshold does not spam.

use crate::settings::Settings;
use juicebar_core::{Charge, DeviceView};
use std::collections::HashSet;

#[derive(Default)]
pub struct Alerts {
    started: bool,
    low: HashSet<String>,
    full: HashSet<String>,
}

pub struct Notice {
    pub title: String,
    pub body: String,
}

impl Alerts {
    pub fn check(&mut self, devices: &[DeviceView], s: &Settings) -> Vec<Notice> {
        let mut out = Vec::new();
        for d in devices.iter().filter(|d| d.online && !s.hidden.contains(&d.key)) {
            let charging = matches!(d.charge, Charge::Charging | Charge::Full);
            let name = s.alias(&d.key).unwrap_or(&d.name);

            if d.level <= s.low_threshold && !charging {
                if self.low.insert(d.key.clone()) && s.notify_low {
                    out.push(Notice { title: format!("{name} is running low"), body: format!("{}% left. Time to charge.", d.level) });
                }
            } else if charging || d.level > s.low_threshold.saturating_add(5) {
                self.low.remove(&d.key);
            }

            let full = d.charge == Charge::Full || (d.charge == Charge::Charging && d.level >= 100);
            if full {
                // Do not announce devices that were already full when the app started.
                if self.full.insert(d.key.clone()) && self.started && s.notify_full {
                    out.push(Notice { title: format!("{name} is fully charged"), body: "You can unplug it now.".into() });
                }
            } else if !charging && d.level < 95 {
                self.full.remove(&d.key);
            }
        }
        self.started = true;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use juicebar_core::{DeviceKind, Transport};

    fn dev(level: u8, charge: Charge) -> DeviceView {
        DeviceView {
            key: "k".into(),
            name: "Mouse".into(),
            vendor: None,
            kind: DeviceKind::Mouse,
            transport: Transport::Usb,
            batteries: vec![],
            level,
            charge,
            step: 1,
            source: "test",
            online: true,
            last_seen: 0,
            estimate: None,
        }
    }

    #[test]
    fn low_alert_fires_once_with_hysteresis() {
        let s = Settings::default();
        let mut a = Alerts::default();
        assert_eq!(a.check(&[dev(50, Charge::Discharging)], &s).len(), 0);
        assert_eq!(a.check(&[dev(19, Charge::Discharging)], &s).len(), 1);
        assert_eq!(a.check(&[dev(18, Charge::Discharging)], &s).len(), 0);
        assert_eq!(a.check(&[dev(22, Charge::Discharging)], &s).len(), 0); // within hysteresis
        assert_eq!(a.check(&[dev(19, Charge::Discharging)], &s).len(), 0);
        assert_eq!(a.check(&[dev(40, Charge::Charging)], &s).len(), 0); // re-arms
        assert_eq!(a.check(&[dev(15, Charge::Discharging)], &s).len(), 1);
    }

    #[test]
    fn full_alert_skips_startup() {
        let s = Settings::default();
        let mut a = Alerts::default();
        assert_eq!(a.check(&[dev(100, Charge::Full)], &s).len(), 0);
        assert_eq!(a.check(&[dev(80, Charge::Discharging)], &s).len(), 0);
        assert_eq!(a.check(&[dev(100, Charge::Full)], &s).len(), 1);
    }
}
