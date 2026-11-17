//! Tray icon(s): a single icon for the lowest battery, or one icon per device.

use crate::icon::{render, system_metrics, IconState};
use crate::settings::{Settings, TrayMode};
use crate::{popup, AppState};
use juicebar_core::{Charge, DeviceView};
use std::collections::HashSet;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;

const MAIN: &str = "main";

fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Open Juicebar", true, None::<&str>)?,
            &MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &CheckMenuItem::with_id(app, "autostart", "Start with Windows", true, autostart, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => popup::show(app, None),
        "refresh" => crate::request_refresh(app),
        "autostart" => {
            let al = app.autolaunch();
            let _ = if al.is_enabled().unwrap_or(false) { al.disable() } else { al.enable() };
        }
        "quit" => crate::quit(app),
        _ => {}
    }
}

fn on_click(tray: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
        popup::toggle(tray.app_handle(), Some(rect));
    }
}

fn builder(id: &str, app: &AppHandle) -> tauri::Result<TrayIconBuilder<tauri::Wry>> {
    Ok(TrayIconBuilder::with_id(id)
        .menu(&menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| on_menu(app, e.id().as_ref()))
        .on_tray_icon_event(on_click))
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let (size, light) = system_metrics();
    let px = render(&IconState { level: None, charge: Charge::Unknown, online: false, low_threshold: 20 }, size, light);
    builder(MAIN, app)?.icon(Image::new_owned(px, size, size)).tooltip("Juicebar — scanning…").build(app)?;
    Ok(())
}

fn label(d: &DeviceView, s: &Settings) -> String {
    let name = s.alias(&d.key).unwrap_or(&d.name);
    let approx = if d.step > 1 { "~" } else { "" };
    let state = match (d.online, d.charge) {
        (false, _) => " (asleep)",
        (_, Charge::Charging) => " ⚡",
        (_, Charge::Full) => " (full)",
        _ => "",
    };
    format!("{name}: {approx}{}%{state}", d.level)
}

/// Windows truncates tooltips at 127 characters.
fn clip(s: String) -> String {
    if s.chars().count() <= 127 {
        s
    } else {
        s.chars().take(126).collect::<String>() + "…"
    }
}

fn state_of(d: Option<&DeviceView>, s: &Settings) -> IconState {
    IconState {
        level: d.map(|d| d.level),
        charge: d.map(|d| d.charge).unwrap_or(Charge::Unknown),
        online: d.is_some_and(|d| d.online),
        low_threshold: s.low_threshold,
    }
}

pub fn update(app: &AppHandle, devices: &[DeviceView], s: &Settings) {
    let (size, light) = system_metrics();
    let visible: Vec<&DeviceView> = devices.iter().filter(|d| !s.hidden.contains(&d.key) && (d.online || s.show_offline)).collect();
    let state = app.state::<AppState>();
    let mut device_trays = state.device_trays.lock().unwrap();

    #[cfg(windows)]
    crate::widget::update(visible.iter().map(|d| widget_item(d, s)).collect());
    #[cfg(windows)]
    let widget_showing = s.taskbar_widget && crate::widget::is_attached() && !visible.is_empty();
    #[cfg(not(windows))]
    let widget_showing = false;
    // Without a visible widget the tray icon is the only entry point: never hide it then.
    let mode = match s.tray_mode {
        TrayMode::Off if !widget_showing => TrayMode::Single,
        m => m,
    };

    let per_device = mode == TrayMode::PerDevice && !visible.is_empty();
    if let Some(main) = app.tray_by_id(MAIN) {
        let _ = main.set_visible(mode == TrayMode::Single || (mode == TrayMode::PerDevice && !per_device));
        if !per_device {
            let lowest = visible.iter().filter(|d| d.online).min_by_key(|d| d.level).or(visible.first()).copied();
            let px = render(&state_of(lowest, s), size, light);
            let _ = main.set_icon(Some(Image::new_owned(px, size, size)));
            let tip = if visible.is_empty() {
                "Juicebar — no devices with battery info".to_string()
            } else {
                visible.iter().map(|d| label(d, s)).collect::<Vec<_>>().join("\n")
            };
            let _ = main.set_tooltip(Some(clip(tip)));
        }
    }

    let wanted: HashSet<String> = if per_device { visible.iter().map(|d| format!("dev:{}", d.key)).collect() } else { HashSet::new() };
    for id in device_trays.iter().filter(|id| !wanted.contains(*id)).cloned().collect::<Vec<_>>() {
        let _ = app.remove_tray_by_id(&id);
        device_trays.remove(&id);
    }
    if !per_device {
        return;
    }
    // Iterate in reverse so the notification area shows devices in list order.
    for d in visible.iter().rev() {
        let id = format!("dev:{}", d.key);
        let px = render(&state_of(Some(d), s), size, light);
        let icon = Image::new_owned(px, size, size);
        let tip = clip(label(d, s));
        match app.tray_by_id(&id) {
            Some(t) => {
                let _ = t.set_icon(Some(icon));
                let _ = t.set_tooltip(Some(tip));
            }
            None => {
                if let Ok(b) = builder(&id, app) {
                    if b.icon(icon).tooltip(tip).build(app).is_ok() {
                        device_trays.insert(id);
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn widget_item(d: &DeviceView, s: &Settings) -> crate::widget::Item {
    use crate::widget::{glyph, Item, Tone};
    use juicebar_core::DeviceKind::*;
    let charging = d.online && matches!(d.charge, Charge::Charging);
    Item {
        key: d.key.clone(),
        name: d.name.clone(),
        level: d.level,
        approx: d.step > 1,
        glyph: match d.kind {
            Headset | Earbuds => glyph::HEADSET,
            Mouse => glyph::MOUSE,
            Keyboard => glyph::KEYBOARD,
            Gamepad => glyph::GAMEPAD,
            Phone => glyph::PHONE,
            Speaker => glyph::SPEAKER,
            Pen => glyph::PEN,
            Watch => glyph::WATCH,
            Other => glyph::BLUETOOTH,
        },
        tone: if d.level <= s.low_threshold && !charging { Tone::Low } else { Tone::Normal },
        charging,
        asleep: !d.online,
    }
}
