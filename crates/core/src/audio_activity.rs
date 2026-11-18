//! Is a physical device playing or recording audio right now? Used to hold back battery
//! reads that would interrupt the sound (see the G435 provider).
//!
//! The HID interface and the audio endpoints of one USB device share a PnP container id.
//! An endpoint counts as busy while any of its audio sessions is active (a stream is
//! running, including silent ones such as an open voice call). Every uncertainty answers
//! "busy": an interrupted song is worse than a stale battery level.

/// True when `hid_path`'s device may be playing or capturing audio.
#[cfg(windows)]
pub fn device_audio_busy(hid_path: &str) -> bool {
    imp::device_audio_busy(hid_path)
}

/// No activity check off Windows, so never risk it.
#[cfg(not(windows))]
pub fn device_audio_busy(_hid_path: &str) -> bool {
    true
}

#[cfg(windows)]
mod imp {
    use windows::core::{Result, GUID, HSTRING, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows::Win32::Devices::Properties::{DEVPROPTYPE, DEVPROP_TYPE_GUID, DEVPROP_TYPE_STRING};
    use windows::Win32::Foundation::DEVPROPKEY;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;

    /// `DEVPKEY_Device_ContainerId`.
    const CONTAINER_ID: DEVPROPKEY =
        DEVPROPKEY { fmtid: GUID::from_values(0x8C7E_D206, 0x3F8A, 0x4827, [0xB3, 0xAB, 0xAE, 0x9E, 0x1F, 0xAE, 0xFC, 0x6C]), pid: 2 };
    /// `DEVPKEY_Device_InstanceId`.
    const INSTANCE_ID: DEVPROPKEY =
        DEVPROPKEY { fmtid: GUID::from_values(0x78C3_4FC8, 0x104A, 0x4ACA, [0x9E, 0xA4, 0x52, 0x4D, 0x52, 0x99, 0x6E, 0x57]), pid: 256 };
    /// Container of every built-in device ("this computer"): matches unrelated endpoints.
    const ROOT_CONTAINER: GUID = GUID::from_values(0, 0, 0, [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);

    struct Com(bool);

    impl Com {
        fn init() -> Self {
            // SAFETY: balanced by CoUninitialize in Drop when it succeeded (S_OK or S_FALSE).
            Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: matches the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        }
    }

    fn devnode_container(instance_id: &str) -> Option<GUID> {
        let id = HSTRING::from(instance_id);
        let mut node = 0u32;
        let mut ty = DEVPROPTYPE::default();
        let mut guid = GUID::zeroed();
        let mut size = std::mem::size_of::<GUID>() as u32;
        // SAFETY: the buffer is a GUID and `size` is its length.
        unsafe {
            if CM_Locate_DevNodeW(&mut node, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
                return None;
            }
            let r = CM_Get_DevNode_PropertyW(node, &CONTAINER_ID, &mut ty, Some(&mut guid as *mut GUID as *mut u8), &mut size, 0);
            (r == CR_SUCCESS && ty == DEVPROP_TYPE_GUID).then_some(guid)
        }
    }

    fn interface_container(interface_path: &str) -> Option<GUID> {
        let path = HSTRING::from(interface_path);
        let mut ty = DEVPROPTYPE::default();
        let mut buf = [0u16; 512];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: `size` is the byte length of `buf`.
        let r = unsafe {
            CM_Get_Device_Interface_PropertyW(PCWSTR(path.as_ptr()), &INSTANCE_ID, &mut ty, Some(buf.as_mut_ptr() as *mut u8), &mut size, 0)
        };
        if r != CR_SUCCESS || ty != DEVPROP_TYPE_STRING {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        devnode_container(&String::from_utf16_lossy(&buf[..len]))
    }

    fn endpoint_container(endpoint: &IMMDevice) -> Option<GUID> {
        // SAFETY: GetId returns a CoTaskMem string that is freed after copying.
        let id = unsafe {
            let raw = endpoint.GetId().ok()?;
            let id = raw.to_string().ok();
            CoTaskMemFree(Some(raw.0 as *const _));
            id?
        };
        devnode_container(&format!("SWD\\MMDEVAPI\\{id}"))
    }

    fn has_active_session(endpoint: &IMMDevice) -> Result<bool> {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let manager: IAudioSessionManager2 = endpoint.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for i in 0..sessions.GetCount()? {
                if sessions.GetSession(i)?.GetState()? == AudioSessionStateActive {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub fn device_audio_busy(hid_path: &str) -> bool {
        let _com = Com::init();
        let target = interface_container(hid_path).filter(|g| *g != ROOT_CONTAINER);
        let scan = || -> Result<bool> {
            // SAFETY: plain COM calls; COM is initialised for this thread above.
            let endpoints = unsafe {
                let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                enumerator.EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE)?
            };
            let mut found = false;
            let mut anything_active = false;
            for i in 0..unsafe { endpoints.GetCount()? } {
                let endpoint = unsafe { endpoints.Item(i)? };
                let mine = target.is_some() && endpoint_container(&endpoint) == target;
                // An endpoint that cannot be inspected counts as active.
                let active = has_active_session(&endpoint).unwrap_or(true);
                if mine && active {
                    return Ok(true);
                }
                found |= mine;
                anything_active |= active;
            }
            // Endpoint not identified: only read while the whole PC is silent.
            Ok(!found && anything_active)
        };
        scan().unwrap_or(true)
    }
}
