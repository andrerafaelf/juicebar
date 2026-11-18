//! The flyout window. Created on demand next to the tray icon and destroyed after it
//! has been hidden for a while, so the WebView2 runtime is not resident while idle.

use crate::AppState;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Rect, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

pub const LABEL: &str = "popup";
const WIDTH: f64 = 380.0;
pub const DEFAULT_HEIGHT: f64 = 420.0;
/// Tallest the flyout gets; the settings page always uses it so it opens without a resize.
pub const MAX_HEIGHT: f64 = 600.0;
const MARGIN: f64 = 12.0;
/// Free the webview after it has been hidden this long.
const DESTROY_AFTER: Duration = Duration::from_secs(45);

#[derive(Clone, Copy)]
pub struct Anchor {
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
}

fn create(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let height = *app.state::<AppState>().popup_height.lock().unwrap();
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
        .title("Juicebar")
        .inner_size(WIDTH, height)
        .decorations(false)
        .transparent(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .shadow(true)
        .visible(false)
        .build()?;

    #[cfg(windows)]
    {
        if window_vibrancy::apply_mica(&win, None).is_err() {
            let _ = window_vibrancy::apply_acrylic(&win, Some((24, 24, 27, 160)));
        }
    }

    let handle = app.clone();
    win.on_window_event(move |e| {
        if let WindowEvent::Focused(false) = e {
            hide(&handle);
        }
    });
    Ok(win)
}

fn place(win: &WebviewWindow, anchor: Option<Anchor>, height: f64) {
    let Ok(Some(monitor)) = (match anchor {
        Some(a) => win.monitor_from_point(a.pos.x as f64 + a.size.width as f64 / 2.0, a.pos.y as f64 + a.size.height as f64 / 2.0),
        None => win.primary_monitor(),
    }) else {
        return;
    };
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    // Use the intended size: right after set_size the OS may still report the old one.
    let size = PhysicalSize::new((WIDTH * scale) as u32, (height * scale) as u32);
    let margin = (MARGIN * scale) as i32;
    let (wx, wy) = (work.position.x, work.position.y);
    let (ww, wh) = (work.size.width as i32, work.size.height as i32);
    let (w, h) = (size.width as i32, size.height as i32);

    let (x, y) = match anchor {
        Some(a) => {
            let cx = a.pos.x + a.size.width as i32 / 2;
            let taskbar_at_top = a.pos.y < wy + wh / 2;
            let y = if taskbar_at_top { wy + margin } else { wy + wh - h - margin };
            (cx - w / 2, y)
        }
        None => (wx + ww - w - margin, wy + wh - h - margin),
    };
    let x = x.clamp(wx + margin, (wx + ww - w - margin).max(wx + margin));
    // The outer rect includes invisible resize borders on the left/right (and bottom);
    // offset by them so the *visible* edge lands on the margin.
    let (dw, dh) = match (win.outer_size(), win.inner_size()) {
        (Ok(o), Ok(i)) => (o.width.saturating_sub(i.width) as i32, o.height.saturating_sub(i.height) as i32),
        _ => (0, 0),
    };
    set_bounds(win, x - dw / 2, y, w + dw, h + dh);
}

/// Moves and resizes in one step. Two separate calls (size, then position) show an
/// intermediate frame where the flyout hangs over the taskbar or its edge jumps.
#[cfg(windows)]
fn set_bounds(win: &WebviewWindow, x: i32, y: i32, w: i32, h: i32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER};
    let Ok(hwnd) = win.hwnd() else { return };
    // SAFETY: the handle belongs to a live window owned by this process.
    let _ = unsafe { SetWindowPos(HWND(hwnd.0 as _), None, x, y, w, h, SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE) };
}

#[cfg(not(windows))]
fn set_bounds(win: &WebviewWindow, x: i32, y: i32, w: i32, h: i32) {
    let _ = win.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

pub fn show(app: &AppHandle, rect: Option<Rect>) {
    let state = app.state::<AppState>();
    if let Some(r) = rect {
        let scale = app.primary_monitor().ok().flatten().map(|m| m.scale_factor()).unwrap_or(1.0);
        *state.anchor.lock().unwrap() = Some(Anchor { pos: r.position.to_physical(scale), size: r.size.to_physical(scale) });
    }
    state.popup_generation.fetch_add(1, Ordering::SeqCst);
    let anchor = *state.anchor.lock().unwrap();
    let height = *state.popup_height.lock().unwrap();
    let present = move |win: WebviewWindow| {
        place(&win, anchor, height);
        let _ = win.show();
        let _ = win.set_focus();
    };
    match app.get_webview_window(LABEL) {
        Some(w) => present(w),
        // Building a webview from an event handler on the main thread deadlocks on
        // Windows (tauri#4121), so create it from a worker thread.
        None => {
            let app = app.clone();
            std::thread::spawn(move || match create(&app) {
                Ok(w) => present(w),
                Err(e) => eprintln!("juicebar: could not create popup: {e}"),
            });
        }
    }
}

pub fn hide(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else { return };
    if !win.is_visible().unwrap_or(false) {
        return;
    }
    let _ = win.hide();
    let state = app.state::<AppState>();
    *state.last_hidden.lock().unwrap() = Some(Instant::now());
    let generation = state.popup_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(DESTROY_AFTER);
        let state = app.state::<AppState>();
        if state.popup_generation.load(Ordering::SeqCst) == generation {
            if let Some(w) = app.get_webview_window(LABEL) {
                if !w.is_visible().unwrap_or(true) {
                    let _ = w.destroy();
                }
            }
        }
    });
}

pub fn toggle(app: &AppHandle, rect: Option<Rect>) {
    let visible = app.get_webview_window(LABEL).is_some_and(|w| w.is_visible().unwrap_or(false));
    if visible {
        hide(app);
        return;
    }
    // Clicking the tray icon first blurs (and hides) an open popup: don't reopen it.
    let recently_hidden = app.state::<AppState>().last_hidden.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_millis(300));
    if !recently_hidden {
        show(app, rect);
    }
}

/// Resize to fit the content height reported by the UI, keeping the anchor edge.
pub fn fit(app: &AppHandle, height: f64) {
    let height = height.clamp(180.0, MAX_HEIGHT).round();
    let state = app.state::<AppState>();
    let previous = std::mem::replace(&mut *state.popup_height.lock().unwrap(), height);
    if previous == height {
        return;
    }
    if let Some(win) = app.get_webview_window(LABEL) {
        place(&win, *state.anchor.lock().unwrap(), height);
    }
}
