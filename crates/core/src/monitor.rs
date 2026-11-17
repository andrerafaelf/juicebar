//! The polling engine: runs providers, merges duplicate sightings of the same device,
//! tracks online/offline state and feeds the history.

use crate::history::History;
use crate::model::{primary_battery, Charge, DeviceKind, DeviceView, Reading};
use crate::providers::{default_providers, PollContext, Provider, ProviderOptions};
use crate::util::{normalize_name, now_ms};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct MonitorOptions {
    pub providers: ProviderOptions,
    /// Offline devices are dropped after this long without a reading.
    pub forget_after: Duration,
}

impl Default for MonitorOptions {
    fn default() -> Self {
        Self { providers: ProviderOptions::default(), forget_after: Duration::from_secs(24 * 3600) }
    }
}

struct State {
    reading: Reading,
    last_seen: u64,
    online: bool,
}

/// Timing of one provider during the last poll (shown by the CLI with `--verbose`).
#[derive(Debug, Clone)]
pub struct ProviderTiming {
    pub id: &'static str,
    pub elapsed: Duration,
    pub found: usize,
}

pub struct Monitor {
    providers: Vec<Box<dyn Provider>>,
    hid: Option<hidapi::HidApi>,
    states: HashMap<String, State>,
    pub history: History,
    opts: MonitorOptions,
    pub last_timings: Vec<ProviderTiming>,
}

impl Monitor {
    pub fn new(opts: MonitorOptions) -> Self {
        Self {
            providers: default_providers(&opts.providers),
            hid: None,
            states: HashMap::new(),
            history: History::default(),
            opts,
            last_timings: Vec::new(),
        }
    }

    /// Run every provider once and return the merged device list.
    pub fn poll(&mut self, full_scan: bool) -> Vec<DeviceView> {
        match &mut self.hid {
            Some(api) => {
                let _ = api.refresh_devices();
            }
            None => self.hid = hidapi::HidApi::new().ok(),
        }
        let mut ctx = PollContext {
            hid: self.hid.as_ref(),
            claimed_paths: HashSet::new(),
            claimed_ids: HashSet::new(),
            bt_connected: Vec::new(),
            full_scan,
        };
        let mut readings = Vec::new();
        self.last_timings.clear();
        for p in self.providers.iter_mut() {
            let started = Instant::now();
            let found = catch_unwind(AssertUnwindSafe(|| p.poll(&mut ctx))).unwrap_or_default();
            self.last_timings.push(ProviderTiming { id: p.id(), elapsed: started.elapsed(), found: found.len() });
            readings.extend(found);
        }
        let merged = merge(readings);

        let now = now_ms();
        let seen: HashSet<String> = merged.iter().map(|r| r.key.clone()).collect();
        for r in merged {
            if let Some(b) = r.primary() {
                self.history.record(&r.key, b.level, b.charge.is_charging(), now / 1000);
            }
            self.states.insert(r.key.clone(), State { reading: r, last_seen: now, online: true });
        }
        let forget_ms = self.opts.forget_after.as_millis() as u64;
        self.states.retain(|k, s| {
            if !seen.contains(k) {
                s.online = false;
            }
            now.saturating_sub(s.last_seen) < forget_ms
        });
        self.snapshot()
    }

    pub fn snapshot(&self) -> Vec<DeviceView> {
        let mut out: Vec<DeviceView> = self
            .states
            .values()
            .filter_map(|s| {
                let r = &s.reading;
                let p = primary_battery(&r.batteries)?;
                Some(DeviceView {
                    key: r.key.clone(),
                    name: r.name.clone(),
                    vendor: r.vendor.clone(),
                    kind: r.kind,
                    transport: r.transport,
                    batteries: r.batteries.clone(),
                    level: p.level,
                    charge: if s.online { p.charge } else { Charge::Unknown },
                    step: p.step,
                    source: r.source,
                    online: s.online,
                    last_seen: s.last_seen,
                    estimate: if s.online { self.history.estimate(&r.key) } else { None },
                })
            })
            .collect();
        // Stable order (kind, then name) so devices don't jump around when one falls asleep.
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.name.cmp(&b.name)).then(a.key.cmp(&b.key)));
        out
    }

    pub fn forget(&mut self, key: &str) {
        self.states.remove(key);
        self.history.forget(key);
    }

    /// Every device seen recently, for persisting across restarts.
    pub fn known(&self) -> Vec<KnownDevice> {
        self.states
            .values()
            .map(|s| KnownDevice {
                key: s.reading.key.clone(),
                name: s.reading.name.clone(),
                vendor: s.reading.vendor.clone(),
                kind: s.reading.kind,
                transport: s.reading.transport,
                batteries: s.reading.batteries.clone(),
                source: s.reading.source.to_string(),
                last_seen: s.last_seen,
            })
            .collect()
    }

    /// Re-insert previously known devices as offline (last known values) until they answer.
    pub fn restore(&mut self, devices: Vec<KnownDevice>) {
        let cutoff = now_ms().saturating_sub(self.opts.forget_after.as_millis() as u64);
        for d in devices.into_iter().filter(|d| d.last_seen >= cutoff && !d.batteries.is_empty()) {
            let reading = Reading {
                key: d.key.clone(),
                name: d.name,
                vendor: d.vendor,
                kind: d.kind,
                transport: d.transport,
                batteries: d.batteries,
                source: intern_source(&d.source),
            };
            self.states.entry(d.key).or_insert(State { reading, last_seen: d.last_seen, online: false });
        }
    }
}

/// Serializable form of a device for [`Monitor::known`] / [`Monitor::restore`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownDevice {
    pub key: String,
    pub name: String,
    pub vendor: Option<String>,
    pub kind: DeviceKind,
    pub transport: crate::model::Transport,
    pub batteries: Vec<crate::model::Battery>,
    pub source: String,
    pub last_seen: u64,
}

fn intern_source(s: &str) -> &'static str {
    crate::providers::SOURCE_IDS.iter().copied().find(|id| *id == s).unwrap_or("restored")
}

/// Collapse duplicate sightings. Readings arrive in provider priority order.
pub fn merge(readings: Vec<Reading>) -> Vec<Reading> {
    let mut out: Vec<Reading> = Vec::new();
    for r in readings {
        // Same key: keep the richer reading (e.g. AirPods L/R/Case beats the single HFP level).
        if let Some(existing) = out.iter_mut().find(|e| e.key == r.key) {
            if r.batteries.len() > existing.batteries.len() {
                *existing = r;
            }
            continue;
        }
        // Same product name from a different provider: first (higher priority) wins.
        let n = normalize_name(&r.name);
        if out.iter().any(|e| e.source != r.source && normalize_name(&e.name) == n) {
            continue;
        }
        out.push(r);
    }
    // XInput only gives four coarse steps; prefer a Bluetooth reading of the same pad.
    let has_bt_xbox =
        out.iter().any(|r| r.source == "bluetooth" && r.kind == DeviceKind::Gamepad && r.name.to_ascii_lowercase().contains("xbox"));
    if has_bt_xbox {
        out.retain(|r| !(r.source == "xinput" && r.kind == DeviceKind::Gamepad));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Battery, Transport};

    fn r(key: &str, name: &str, source: &'static str, cells: usize) -> Reading {
        Reading {
            key: key.into(),
            name: name.into(),
            vendor: None,
            kind: DeviceKind::Mouse,
            transport: Transport::Usb,
            batteries: (0..cells).map(|_| Battery::new(50, Charge::Discharging)).collect(),
            source,
        }
    }

    #[test]
    fn merges_duplicates() {
        let out = merge(vec![
            r("hidpp:1", "MX Master 3S", "logitech-hidpp", 1),
            r("bt:1", "MX Master 3S", "bluetooth", 1),
            r("bt:2", "AirPods Pro", "bluetooth", 1),
            r("bt:2", "AirPods Pro", "airpods", 3),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].source, "logitech-hidpp");
        assert_eq!(out[1].batteries.len(), 3);
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use crate::model::{Battery, Transport};

    #[test]
    fn known_devices_round_trip_as_offline() {
        let mut m = Monitor::new(MonitorOptions { providers: ProviderOptions { demo: true, ..Default::default() }, ..Default::default() });
        m.restore(vec![KnownDevice {
            key: "sonix:05ac:024f".into(),
            name: "AULA F75 Max".into(),
            vendor: None,
            kind: DeviceKind::Keyboard,
            transport: Transport::Receiver,
            batteries: vec![Battery::new(71, Charge::Unknown)],
            source: "sonix".into(),
            last_seen: now_ms(),
        }]);
        let v = m.snapshot();
        assert_eq!(v.len(), 1);
        assert!(!v[0].online);
        assert_eq!((v[0].level, v[0].source), (71, "sonix"));
        let json = serde_json::to_string(&m.known()).unwrap();
        let back: Vec<KnownDevice> = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0].name, "AULA F75 Max");
    }
}
