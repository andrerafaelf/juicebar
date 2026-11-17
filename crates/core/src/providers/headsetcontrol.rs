//! Bridge to [HeadsetControl](https://github.com/Sapd/HeadsetControl), which supports
//! dozens of gaming headsets (SteelSeries Arctis, Corsair Void/HS, HyperX Cloud,
//! Logitech G, Audeze Maxwell, Roccat…). Used only when the executable is installed.

use super::{PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use crate::util::normalize_name;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct HeadsetControl {
    exe: Option<PathBuf>,
    explicit: Option<PathBuf>,
    searched: bool,
}

impl HeadsetControl {
    pub fn new(explicit: Option<PathBuf>) -> Self {
        Self { exe: None, explicit, searched: false }
    }

    fn locate(&mut self) -> Option<PathBuf> {
        if let Some(p) = &self.explicit {
            return p.is_file().then(|| p.clone());
        }
        if !self.searched {
            self.searched = true;
            self.exe = find_on_path();
        }
        self.exe.clone()
    }
}

fn find_on_path() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["headsetcontrol.exe", "HeadsetControl.exe"] } else { &["headsetcontrol"] };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // Also look next to our own executable (portable installs).
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        dirs.push(dir);
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

fn run(exe: &Path, timeout: Duration) -> Option<String> {
    let mut cmd = Command::new(exe);
    cmd.args(["-b", "-o", "json"]).stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().ok()?;
    // Drain stdout on another thread so a large JSON document cannot fill the pipe and stall.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut s).ok().map(|_| s)
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    reader.join().ok().flatten()
}

/// Parse `headsetcontrol -o json` output.
pub fn parse(json: &str) -> Vec<Reading> {
    let Ok(root) = serde_json::from_str::<Value>(json) else { return vec![] };
    let Some(devices) = root.get("devices").and_then(Value::as_array) else { return vec![] };
    devices
        .iter()
        .filter_map(|d| {
            let battery = d.get("battery")?;
            let status = battery.get("status").and_then(Value::as_str).unwrap_or("");
            let level = battery.get("level").and_then(Value::as_i64).unwrap_or(-1);
            let charge = match status {
                "BATTERY_AVAILABLE" => Charge::Discharging,
                "BATTERY_CHARGING" => Charge::Charging,
                _ => return None,
            };
            if !(0..=100).contains(&level) {
                return None;
            }
            let name = d.get("device").and_then(Value::as_str).unwrap_or("Headset").to_string();
            let vid = d.get("id_vendor").and_then(Value::as_str).unwrap_or("");
            let pid = d.get("id_product").and_then(Value::as_str).unwrap_or("");
            Some(Reading {
                key: format!("hsc:{}:{}:{}", vid.trim_start_matches("0x"), pid.trim_start_matches("0x"), normalize_name(&name)),
                vendor: d.get("vendor").and_then(Value::as_str).map(str::to_string),
                name,
                kind: DeviceKind::Headset,
                transport: Transport::Receiver,
                batteries: vec![Battery::new(level as u8, charge)],
                source: "headsetcontrol",
            })
        })
        .collect()
}

impl Provider for HeadsetControl {
    fn id(&self) -> &'static str {
        "headsetcontrol"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        if ctx.full_scan && self.explicit.is_none() {
            self.searched = false;
        }
        let Some(exe) = self.locate() else { return vec![] };
        run(&exe, Duration::from_secs(6)).map(|s| parse(&s)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json() {
        let json = r#"{"name":"HeadsetControl","version":"3.0.0","device_count":2,"devices":[
            {"status":"success","device":"SteelSeries Arctis Nova 7","vendor":"SteelSeries","product":"Arctis Nova 7",
             "id_vendor":"0x1038","id_product":"0x2202","battery":{"status":"BATTERY_CHARGING","level":64}},
            {"status":"success","device":"Logitech G535","id_vendor":"0x046d","id_product":"0x0ac4",
             "battery":{"status":"BATTERY_UNAVAILABLE","level":-1}}]}"#;
        let r = parse(json);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "SteelSeries Arctis Nova 7");
        assert_eq!(r[0].batteries[0].level, 64);
        assert_eq!(r[0].batteries[0].charge, Charge::Charging);
        assert_eq!(r[0].key, "hsc:1038:2202:steelseriesarctisnova7");
    }
}
