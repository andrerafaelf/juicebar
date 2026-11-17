//! Device arrival/removal notifications (Windows), used to refresh right away when a
//! receiver is plugged in or a Bluetooth device connects, instead of waiting for the
//! next poll.

use std::ffi::c_void;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;

type Callback = Box<dyn Fn() + Send + Sync>;

pub struct DeviceWatcher {
    handle: HCMNOTIFICATION,
    _callback: Box<Callback>,
}

// SAFETY: the handle is only used to unregister, which is thread-safe.
unsafe impl Send for DeviceWatcher {}

unsafe extern "system" fn on_event(
    _h: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    _data: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL || action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL {
        // SAFETY: context is the Box<Callback> owned by DeviceWatcher, alive until unregistration.
        let cb = unsafe { &*(context as *const Callback) };
        cb();
    }
    0
}

/// Call `cb` (on a system thread) whenever any device interface arrives or leaves.
/// Expect bursts: debounce on the receiving side.
pub fn watch(cb: impl Fn() + Send + Sync + 'static) -> Option<DeviceWatcher> {
    let callback: Box<Callback> = Box::new(Box::new(cb));
    let filter = CM_NOTIFY_FILTER {
        cbSize: std::mem::size_of::<CM_NOTIFY_FILTER>() as u32,
        Flags: CM_NOTIFY_FILTER_FLAG_ALL_INTERFACE_CLASSES,
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
        ..Default::default()
    };
    let mut handle = HCMNOTIFICATION::default();
    // SAFETY: filter and callback outlive the registration (callback is moved into the watcher).
    let r = unsafe { CM_Register_Notification(&filter, Some(&*callback as *const Callback as *const c_void), Some(on_event), &mut handle) };
    (r == CR_SUCCESS).then_some(DeviceWatcher { handle, _callback: callback })
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        // SAFETY: handle came from a successful CM_Register_Notification.
        unsafe {
            let _ = CM_Unregister_Notification(self.handle);
        }
    }
}
