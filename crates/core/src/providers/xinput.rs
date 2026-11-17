//! Xbox controllers (wireless adapter, USB, Bluetooth) and headsets plugged into them,
//! via XInput. XInput only reports four coarse levels.

use super::{PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use windows::Win32::UI::Input::XboxController::*;

pub struct XInput;

fn level(info: &XINPUT_BATTERY_INFORMATION) -> Option<Battery> {
    match info.BatteryType {
        BATTERY_TYPE_DISCONNECTED | BATTERY_TYPE_WIRED => return None,
        _ => {}
    }
    let pct = match info.BatteryLevel {
        BATTERY_LEVEL_EMPTY => 5,
        BATTERY_LEVEL_LOW => 25,
        BATTERY_LEVEL_MEDIUM => 60,
        BATTERY_LEVEL_FULL => 100,
        _ => return None,
    };
    Some(Battery::new(pct, Charge::Unknown).coarse(25))
}

impl Provider for XInput {
    fn id(&self) -> &'static str {
        "xinput"
    }

    fn poll(&mut self, _ctx: &mut PollContext) -> Vec<Reading> {
        let mut out = Vec::new();
        for user in 0..4u32 {
            let mut state = XINPUT_STATE::default();
            // SAFETY: valid out-pointer, user index in range.
            if unsafe { XInputGetState(user, &mut state) } != 0 {
                continue;
            }
            for (devtype, name, kind) in [
                (BATTERY_DEVTYPE_GAMEPAD, "Xbox Controller", DeviceKind::Gamepad),
                (BATTERY_DEVTYPE_HEADSET, "Xbox Headset", DeviceKind::Headset),
            ] {
                let mut info = XINPUT_BATTERY_INFORMATION::default();
                // SAFETY: valid out-pointer.
                if unsafe { XInputGetBatteryInformation(user, devtype, &mut info) } != 0 {
                    continue;
                }
                if let Some(battery) = level(&info) {
                    out.push(Reading {
                        key: format!("xinput:{user}:{}", devtype.0),
                        name: if user == 0 { name.to_string() } else { format!("{name} {}", user + 1) },
                        vendor: Some("Microsoft".into()),
                        kind,
                        transport: Transport::Receiver,
                        batteries: vec![battery],
                        source: "xinput",
                    });
                }
            }
        }
        out
    }
}
