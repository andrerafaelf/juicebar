//! Small helpers for blocking on WinRT async operations from the monitor thread.

use std::sync::mpsc;
use std::time::Duration;
use windows::core::{IInspectable, Interface, HSTRING};
use windows::Foundation::{IPropertyValue, PropertyType};
use windows_collections::IMapView;
use windows_future::IAsyncOperation;

/// Wait for an async operation with a timeout; cancels it on timeout.
pub fn wait<T>(op: IAsyncOperation<T>, timeout: Duration) -> Option<T>
where
    T: windows_core::RuntimeType + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    op.when(move |r| {
        let _ = tx.send(r);
    })
    .ok()?;
    match rx.recv_timeout(timeout) {
        Ok(Ok(v)) => Some(v),
        _ => {
            let _ = op.Cancel();
            None
        }
    }
}

fn lookup(map: &IMapView<HSTRING, IInspectable>, key: &str) -> Option<IPropertyValue> {
    map.Lookup(&HSTRING::from(key)).ok()?.cast::<IPropertyValue>().ok()
}

pub fn prop_string(map: &IMapView<HSTRING, IInspectable>, key: &str) -> Option<String> {
    let v = lookup(map, key)?;
    (v.Type().ok()? == PropertyType::String).then(|| v.GetString().ok().map(|s| s.to_string())).flatten()
}

pub fn prop_u32(map: &IMapView<HSTRING, IInspectable>, key: &str) -> Option<u32> {
    let v = lookup(map, key)?;
    match v.Type().ok()? {
        PropertyType::UInt8 => v.GetUInt8().ok().map(u32::from),
        PropertyType::UInt16 => v.GetUInt16().ok().map(u32::from),
        PropertyType::UInt32 => v.GetUInt32().ok(),
        PropertyType::Int32 => v.GetInt32().ok().map(|x| x as u32),
        _ => None,
    }
}
