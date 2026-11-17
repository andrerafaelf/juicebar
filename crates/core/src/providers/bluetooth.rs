//! Windows Bluetooth: every connected Bluetooth device that Windows itself knows a
//! battery level for (hands-free headsets, HID-over-GATT mice/keyboards/pens…), plus a
//! direct GATT Battery Service read (0x180F/0x2A19) for BLE devices Windows does not
//! surface on its own.

use super::{BtPeer, PollContext, Provider};
use crate::model::{Battery, Charge, DeviceKind, Reading, Transport};
use crate::util::{clean_name, guess_kind};
use crate::winrt::{prop_string, prop_u32, wait};
use std::collections::HashMap;
use std::time::Duration;
use windows::core::{GUID, HSTRING, PCWSTR};
use windows::Devices::Bluetooth::GenericAttributeProfile::{GattCharacteristicUuids, GattCommunicationStatus, GattServiceUuids};
use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Storage::Streams::DataReader;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Properties::{DEVPROPTYPE, DEVPROP_TYPE_BYTE};
use windows::Win32::Foundation::DEVPROPKEY;
use windows_collections::IIterable;

/// `DEVPKEY_Bluetooth_Battery`: a BYTE 0..=100 on the device node.
const BATTERY_KEY: DEVPROPKEY =
    DEVPROPKEY { fmtid: GUID::from_values(0x104E_A319, 0x6EE2, 0x4701, [0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5]), pid: 2 };

const TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone)]
struct Peer {
    address: u64,
    name: String,
    le: bool,
    kind: DeviceKind,
}

#[derive(Default)]
pub struct Bluetooth {
    /// BLE addresses without a readable Battery Service, with the poll number to retry at.
    no_gatt: HashMap<u64, u64>,
    polls: u64,
}

fn parse_address(s: &str) -> Option<u64> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    (hex.len() == 12).then(|| u64::from_str_radix(&hex, 16).ok()).flatten()
}

/// Pull the 12-hex-digit Bluetooth address out of a device instance id such as
/// `BTHENUM\{0000111E-…}_LOCALMFG&0002\7&2A5BC8A4&0&A0C9A0123456_C00000000` or
/// `BTHLE\DEV_A0C9A0123456\7&…`.
pub(crate) fn address_from_instance(id: &str) -> Option<u64> {
    id.split(['\\', '&', '_', '#'])
        .rev()
        .find(|t| t.len() == 12 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .and_then(|t| u64::from_str_radix(t, 16).ok())
}

/// Bluetooth Class-of-Device major/minor to a device kind.
fn kind_from_cod(major: u32, minor: u32) -> Option<DeviceKind> {
    match major {
        0x02 => Some(DeviceKind::Phone),
        0x04 => match minor {
            0x05 => Some(DeviceKind::Speaker),
            0x01 | 0x02 | 0x06 => Some(DeviceKind::Headset),
            _ => None,
        },
        0x05 => match (minor >> 4, minor & 0x0F) {
            (_, 0x01 | 0x02) => Some(DeviceKind::Gamepad),
            (1, _) | (3, _) => Some(DeviceKind::Keyboard),
            (2, _) => Some(DeviceKind::Mouse),
            _ => None,
        },
        0x07 => Some(DeviceKind::Watch),
        _ => None,
    }
}

fn connected_peers() -> Vec<Peer> {
    let mut peers = Vec::new();
    let selectors = [
        (BluetoothDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected), false),
        (BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected), true),
    ];
    for (selector, le) in selectors {
        let Ok(selector) = selector else { continue };
        let props: Vec<HSTRING> = vec![
            "System.Devices.Aep.DeviceAddress".into(),
            "System.Devices.Aep.Bluetooth.Cod.Major".into(),
            "System.Devices.Aep.Bluetooth.Cod.Minor".into(),
        ];
        let props = IIterable::<HSTRING>::from(props);
        let Ok(op) = DeviceInformation::FindAllAsyncAqsFilterAndAdditionalProperties(&selector, &props) else { continue };
        let Some(list) = wait(op, TIMEOUT) else { continue };
        for info in list {
            let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
            let Ok(map) = info.Properties() else { continue };
            let address = prop_string(&map, "System.Devices.Aep.DeviceAddress")
                .and_then(|a| parse_address(&a))
                .or_else(|| info.Id().ok().and_then(|id| id.to_string().rsplit('-').next().and_then(parse_address)));
            let Some(address) = address else { continue };
            let cod = prop_u32(&map, "System.Devices.Aep.Bluetooth.Cod.Major")
                .zip(prop_u32(&map, "System.Devices.Aep.Bluetooth.Cod.Minor"))
                .and_then(|(ma, mi)| kind_from_cod(ma, mi));
            let guessed = guess_kind(&name);
            let kind = if guessed != DeviceKind::Other { guessed } else { cod.unwrap_or(DeviceKind::Other) };
            if !peers.iter().any(|p: &Peer| p.address == address) {
                peers.push(Peer { address, name, le, kind });
            }
        }
    }
    peers
}

/// Battery level for every Bluetooth device node Windows has one for, keyed by address.
fn pnp_batteries() -> HashMap<u64, u8> {
    let mut out = HashMap::new();
    for enumerator in ["BTHENUM", "BTHLE"] {
        let wide = HSTRING::from(enumerator);
        // SAFETY: standard SetupAPI enumeration; buffers are sized and the set is destroyed.
        unsafe {
            let Ok(set) = SetupDiGetClassDevsW(None, PCWSTR(wide.as_ptr()), None, DIGCF_ALLCLASSES | DIGCF_PRESENT) else { continue };
            let mut index = 0;
            loop {
                let mut data = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
                if SetupDiEnumDeviceInfo(set, index, &mut data).is_err() {
                    break;
                }
                index += 1;
                let mut ty = DEVPROPTYPE::default();
                let mut value = [0u8; 4];
                if SetupDiGetDevicePropertyW(set, &data, &BATTERY_KEY, &mut ty, Some(&mut value), None, 0).is_err()
                    || ty != DEVPROP_TYPE_BYTE
                {
                    continue;
                }
                let mut id = [0u16; 512];
                if SetupDiGetDeviceInstanceIdW(set, &data, Some(&mut id), None).is_err() {
                    continue;
                }
                let len = id.iter().position(|&c| c == 0).unwrap_or(id.len());
                if let Some(addr) = address_from_instance(&String::from_utf16_lossy(&id[..len])) {
                    let level = value[0].min(100);
                    out.entry(addr).and_modify(|l: &mut u8| *l = (*l).max(level)).or_insert(level);
                }
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
        }
    }
    out
}

fn gatt_battery(address: u64) -> Option<u8> {
    let dev = wait(BluetoothLEDevice::FromBluetoothAddressAsync(address).ok()?, TIMEOUT)?;
    let result = (|| {
        let services = wait(
            dev.GetGattServicesForUuidWithCacheModeAsync(GattServiceUuids::Battery().ok()?, BluetoothCacheMode::Cached).ok()?,
            TIMEOUT,
        )?;
        let service = services.Services().ok()?.GetAt(0).ok()?;
        let chars = wait(
            service
                .GetCharacteristicsForUuidWithCacheModeAsync(GattCharacteristicUuids::BatteryLevel().ok()?, BluetoothCacheMode::Cached)
                .ok()?,
            TIMEOUT,
        )?;
        let ch = chars.Characteristics().ok()?.GetAt(0).ok()?;
        let read = wait(ch.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached).ok()?, TIMEOUT)?;
        let _ = service.Close();
        if read.Status().ok()? != GattCommunicationStatus::Success {
            return None;
        }
        let reader = DataReader::FromBuffer(&read.Value().ok()?).ok()?;
        reader.ReadByte().ok().map(|b| b.min(100))
    })();
    let _ = dev.Close();
    result
}

impl Provider for Bluetooth {
    fn id(&self) -> &'static str {
        "bluetooth"
    }

    fn poll(&mut self, ctx: &mut PollContext) -> Vec<Reading> {
        self.polls += 1;
        let peers = connected_peers();
        if peers.is_empty() {
            return vec![];
        }
        let pnp = pnp_batteries();
        let mut out = Vec::new();
        for peer in peers {
            let key = format!("bt:{:012x}", peer.address);
            ctx.bt_connected.push(BtPeer { address: peer.address, name: peer.name.clone(), key: key.clone() });
            let level = match pnp.get(&peer.address) {
                Some(l) => Some(*l),
                None if peer.le => {
                    let skip = !ctx.full_scan && self.no_gatt.get(&peer.address).is_some_and(|r| self.polls < *r);
                    if skip {
                        None
                    } else {
                        let l = gatt_battery(peer.address);
                        if l.is_none() {
                            self.no_gatt.insert(peer.address, self.polls + 30);
                        }
                        l
                    }
                }
                None => None,
            };
            let Some(level) = level else { continue };
            let name = clean_name(&peer.name);
            out.push(Reading {
                key,
                kind: if peer.kind == DeviceKind::Other { guess_kind(&name) } else { peer.kind },
                name,
                vendor: None,
                transport: if peer.le { Transport::Ble } else { Transport::Bluetooth },
                batteries: vec![Battery::new(level, Charge::Unknown)],
                source: "bluetooth",
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_ids() {
        assert_eq!(
            address_from_instance(r"BTHENUM\{0000111E-0000-1000-8000-00805F9B34FB}_LOCALMFG&0002\7&2A5BC8A4&0&A0C9A0123456_C00000000"),
            Some(0xA0C9A0123456)
        );
        assert_eq!(address_from_instance(r"BTHLE\DEV_A0C9A0123456\7&1E0E9B3&0&A0C9A0123456"), Some(0xA0C9A0123456));
        assert_eq!(parse_address("a0:c9:a0:12:34:56"), Some(0xA0C9A0123456));
    }

    #[test]
    fn class_of_device() {
        assert_eq!(kind_from_cod(0x04, 0x01), Some(DeviceKind::Headset));
        assert_eq!(kind_from_cod(0x05, 0x20), Some(DeviceKind::Mouse));
        assert_eq!(kind_from_cod(0x05, 0x10), Some(DeviceKind::Keyboard));
        assert_eq!(kind_from_cod(0x05, 0x02), Some(DeviceKind::Gamepad));
    }
}
