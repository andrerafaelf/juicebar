//! Any HID device that declares a standard battery usage in its report descriptor
//! (Battery Strength 0x06:0x20, or Battery System state-of-charge 0x85:0x65/0x66).
//!
//! The descriptor is parsed once per collection and cached, including negative results,
//! so steady-state polls only touch devices that actually have a battery field.

use super::{path_of, product_name, transport_of, PollContext, Provider};
use crate::hid_descriptor::{parse, Field, ReportKind, BATTERY_USAGES, CHARGING_USAGES};
use crate::model::{Battery, Charge, Reading};
use crate::util::{clean_name, guess_kind, normalize_name};
use hidapi::{DeviceInfo, HidDevice};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const APPLE_VID: u16 = 0x05AC;

#[derive(Clone)]
struct Plan {
    battery: Field,
    charging: Option<Field>,
    report_len: usize,
}

#[derive(Default)]
pub struct GenericHid {
    /// `None` = no battery usage (or not openable): never probe again until a full scan.
    plans: HashMap<String, Option<Plan>>,
}

fn plan_for(dev: &HidDevice) -> Option<Plan> {
    let mut raw = vec![0u8; 4096];
    let n = dev.get_report_descriptor(&mut raw).ok()?;
    let desc = parse(&raw[..n]);
    // UPS / power devices use the same usages for a different purpose.
    if matches!(desc.application_page, Some(0x84) | Some(0x85)) {
        return None;
    }
    let battery = desc
        .fields
        .iter()
        .filter(|f| f.kind != ReportKind::Output && !f.constant)
        .find(|f| BATTERY_USAGES.contains(&(f.usage_page, f.usage)))?
        .clone();
    let charging = desc
        .fields
        .iter()
        .find(|f| f.kind == battery.kind && f.report_id == battery.report_id && CHARGING_USAGES.contains(&(f.usage_page, f.usage)))
        .cloned();
    let report_len = desc.report_len(battery.kind, battery.report_id)?;
    Some(Plan { battery, charging, report_len })
}

fn read_report(dev: &HidDevice, plan: &Plan) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; plan.report_len.max(2) + 1];
    buf[0] = plan.battery.report_id;
    let n = match plan.battery.kind {
        ReportKind::Feature => dev.get_feature_report(&mut buf).ok()?,
        _ => match dev.get_input_report(&mut buf) {
            Ok(n) if n > 0 => n,
            // Not every stack supports GET_REPORT(Input); wait for the device to send it.
            _ => {
                let deadline = Instant::now() + Duration::from_millis(300);
                loop {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    let n = dev.read_timeout(&mut buf, 100).ok()?;
                    if n > 0 && (plan.battery.report_id == 0 || buf[0] == plan.battery.report_id) {
                        break n;
                    }
                }
            }
        },
    };
    buf.truncate(n);
    // hidapi strips the id byte for devices without numbered reports.
    if plan.battery.report_id == 0 && n == plan.report_len + 1 {
        buf.remove(0);
    }
    Some(buf)
}

impl Provider for GenericHid {
    fn id(&self) -> &'static str {
        "hid-battery"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        let Some(api) = ctx.hid else { return vec![] };
        if ctx.full_scan {
            self.plans.retain(|_, p| p.is_some());
        }
        let infos: Vec<DeviceInfo> = api
            .device_list()
            .filter(|i| {
                !ctx.claimed_paths.contains(&path_of(i))
                    && !ctx.claimed_ids.contains(&(i.vendor_id(), i.product_id()))
                    && !matches!(i.usage_page(), 0x84 | 0x85)
            })
            .cloned()
            .collect();

        let mut out = Vec::new();
        for info in infos {
            let path = path_of(&info);
            if matches!(self.plans.get(&path), Some(None)) {
                continue;
            }
            let Ok(dev) = info.open_device(api) else {
                self.plans.insert(path, None);
                continue;
            };
            let plan = match self.plans.get(&path) {
                Some(Some(p)) => p.clone(),
                _ => match plan_for(&dev) {
                    Some(p) => {
                        self.plans.insert(path.clone(), Some(p.clone()));
                        p
                    }
                    None => {
                        self.plans.insert(path, None);
                        continue;
                    }
                },
            };
            let Some(report) = read_report(&dev, &plan) else { continue };
            let Some(raw) = plan.battery.extract(&report) else { continue };
            let level = if info.vendor_id() == APPLE_VID { raw.clamp(0, 100) as u8 } else { plan.battery.to_percent(raw) };
            let charge = match plan.charging.as_ref().and_then(|f| f.extract(&report)) {
                Some(v) if v != 0 => {
                    if level >= 100 {
                        Charge::Full
                    } else {
                        Charge::Charging
                    }
                }
                Some(_) => Charge::Discharging,
                None => Charge::Unknown,
            };
            let name = clean_name(&product_name(&info));
            out.push(Reading {
                key: format!(
                    "hid:{:04x}:{:04x}:{}",
                    info.vendor_id(),
                    info.product_id(),
                    normalize_name(info.serial_number().unwrap_or(&name))
                ),
                kind: guess_kind(&name),
                vendor: info.manufacturer_string().map(str::to_string).filter(|s| !s.trim().is_empty()),
                name,
                transport: transport_of(&info),
                batteries: vec![Battery::new(level, charge)],
                source: "hid-battery",
            });
        }
        out
    }
}
