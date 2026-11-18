mod alerts;
mod engine;
mod icon;
mod popup;
mod settings;
mod tray;
#[cfg(windows)]
mod widget;

use alerts::Alerts;
use juicebar_core::util::now_ms;
use juicebar_core::{DeviceView, ProviderOptions, Sample};
use serde::Serialize;
use settings::Settings;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_notification::NotificationExt;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub devices: Vec<DeviceView>,
    /// Unix ms of the last completed poll; 0 before the first one.
    pub updated_at: u64,
}

pub struct AppState {
    pub snapshot: Mutex<Snapshot>,
    pub settings: Mutex<Settings>,
    pub engine: engine::Handle,
    pub settings_path: PathBuf,
    pub device_trays: Mutex<HashSet<String>>,
    pub anchor: Mutex<Option<popup::Anchor>>,
    pub last_hidden: Mutex<Option<Instant>>,
    pub popup_generation: AtomicU64,
    pub popup_height: Mutex<f64>,
    pub focus: Mutex<Option<String>>,
}

fn provider_options(s: &Settings) -> ProviderOptions {
    ProviderOptions {
        headsetcontrol_path: s.headsetcontrol_path.as_ref().map(PathBuf::from),
        demo: std::env::var_os("JUICEBAR_DEMO").is_some(),
        g435_interval: (s.g435_minutes > 0).then(|| Duration::from_secs(s.g435_minutes as u64 * 60)),
    }
}

fn apply_aliases(devices: &mut [DeviceView], s: &Settings) {
    for d in devices.iter_mut() {
        if let Some(alias) = s.alias(&d.key) {
            d.name = alias.to_string();
        }
    }
}

fn publish(app: &AppHandle, snapshot: Snapshot, settings: Settings) {
    let app2 = app.clone();
    let devices = snapshot.devices.clone();
    // Tray APIs must run on the main thread.
    let _ = app.run_on_main_thread(move || tray::update(&app2, &devices, &settings));
    let _ = app.emit("snapshot", snapshot);
}

/// Called by the engine after every poll.
pub(crate) fn on_devices(app: &AppHandle, state: &AppState, mut devices: Vec<DeviceView>, alerts: &mut Alerts) {
    let settings = state.settings.lock().unwrap().clone();
    apply_aliases(&mut devices, &settings);
    let notes = alerts.check(&devices, &settings);
    // Simulated devices shouldn't raise real notifications.
    if std::env::var_os("JUICEBAR_DEMO").is_none() {
        for n in notes {
            let _ = app.notification().builder().title(n.title).body(n.body).show();
        }
    }
    let snapshot = Snapshot { devices, updated_at: now_ms() };
    *state.snapshot.lock().unwrap() = snapshot.clone();
    publish(app, snapshot, settings);
}

pub(crate) fn request_refresh(app: &AppHandle) {
    app.state::<AppState>().engine.send(engine::Cmd::Refresh);
}

pub(crate) fn quit(app: &AppHandle) {
    #[cfg(windows)]
    widget::stop();
    app.state::<AppState>().engine.shutdown();
    app.exit(0);
}

/// Re-render tray + widget from the last snapshot (e.g. after the widget attaches).
pub(crate) fn republish(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut snapshot = state.snapshot.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    apply_aliases(&mut snapshot.devices, &settings);
    publish(app, snapshot, settings);
}

#[cfg(windows)]
fn start_widget(app: &AppHandle) {
    use tauri::{PhysicalPosition, PhysicalSize, Rect};
    use widget::{Event, MenuCommand};
    configure_widget(&app.state::<AppState>().settings.lock().unwrap());
    let app = app.clone();
    widget::start(move |event| match event {
        Event::Click(r, key) => {
            let rect = Rect {
                position: PhysicalPosition::new(r.left, r.top).into(),
                size: PhysicalSize::new((r.right - r.left) as u32, (r.bottom - r.top) as u32).into(),
            };
            // Clicking a specific device opens the flyout with that device expanded.
            if let Some(key) = key {
                *app.state::<AppState>().focus.lock().unwrap() = Some(key.clone());
                let _ = app.emit("focus-device", key);
            }
            popup::toggle(&app, Some(rect));
        }
        Event::Menu(MenuCommand::Open) => popup::show(&app, None),
        Event::Menu(MenuCommand::Refresh) => request_refresh(&app),
        Event::Menu(MenuCommand::Quit) => quit(&app),
        // Tray icon visibility depends on whether the widget is showing.
        Event::AttachmentChanged => republish(&app),
    });
}

#[cfg(windows)]
fn configure_widget(s: &Settings) {
    widget::configure(s.taskbar_widget, s.widget_style == settings::WidgetStyle::Compact);
}

// ---------------------------------------------------------------- commands

/// Device the user clicked in the taskbar widget; the flyout expands it once.
#[tauri::command]
fn take_focus(state: State<AppState>) -> Option<String> {
    state.focus.lock().unwrap().take()
}

#[tauri::command]
fn get_snapshot(state: State<AppState>) -> Snapshot {
    state.snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Settings {
    let settings = settings.sanitized();
    let previous = std::mem::replace(&mut *state.settings.lock().unwrap(), settings.clone());
    settings::save(&state.settings_path, &settings);
    #[cfg(windows)]
    configure_widget(&settings);
    if previous.poll_seconds != settings.poll_seconds
        || previous.headsetcontrol_path != settings.headsetcontrol_path
        || previous.g435_minutes != settings.g435_minutes
    {
        state.engine.send(engine::Cmd::Configure(provider_options(&settings), Duration::from_secs(settings.poll_seconds as u64)));
    }
    // Re-apply aliases / hidden devices / tray mode right away.
    let mut snapshot = state.snapshot.lock().unwrap().clone();
    apply_aliases(&mut snapshot.devices, &settings);
    publish(&app, snapshot, settings.clone());
    settings
}

#[tauri::command]
fn refresh(app: AppHandle) {
    request_refresh(&app);
}

#[tauri::command]
async fn get_history(state: State<'_, AppState>, key: String) -> Result<Vec<Sample>, ()> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.history(key)).await.map_err(|_| ())
}

#[tauri::command]
fn forget_device(state: State<AppState>, key: String) {
    state.engine.send(engine::Cmd::Forget(key));
}

#[tauri::command]
fn fit_popup(app: AppHandle, height: f64) {
    popup::fit(&app, height);
}

#[tauri::command]
fn hide_popup(app: AppHandle) {
    popup::hide(&app);
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> bool {
    let al = app.autolaunch();
    let _ = if enabled { al.enable() } else { al.disable() };
    al.is_enabled().unwrap_or(false)
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    quit(&app);
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

// ---------------------------------------------------------------- entry point

pub fn run() {
    let background = std::env::args().any(|a| a == "--background");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| popup::show(app, None)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            take_focus,
            get_settings,
            save_settings,
            refresh,
            get_history,
            forget_device,
            fit_popup,
            hide_popup,
            get_autostart,
            set_autostart,
            quit_app,
            app_version
        ])
        .setup(move |app| {
            // Demo mode keeps its own settings and state so it never touches real data.
            let sandbox = if std::env::var_os("JUICEBAR_DEMO").is_some() { "demo" } else { "" };
            let config_dir = app.path().app_config_dir()?.join(sandbox);
            let data_dir = app.path().app_data_dir()?.join(sandbox);
            let settings_path = config_dir.join("settings.json");
            let settings = settings::load(&settings_path);
            let engine = engine::spawn(
                app.handle().clone(),
                provider_options(&settings),
                Duration::from_secs(settings.poll_seconds as u64),
                data_dir.join("history.json"),
            );
            app.manage(AppState {
                snapshot: Mutex::new(Snapshot::default()),
                settings: Mutex::new(settings),
                engine,
                settings_path,
                device_trays: Mutex::new(HashSet::new()),
                anchor: Mutex::new(None),
                last_hidden: Mutex::new(None),
                popup_generation: AtomicU64::new(0),
                popup_height: Mutex::new(popup::DEFAULT_HEIGHT),
                focus: Mutex::new(None),
            });
            tray::init(app.handle())?;
            #[cfg(windows)]
            start_widget(app.handle());
            if !background {
                popup::show(app.handle(), None);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Juicebar")
        .run(|_app, event| {
            // The popup is destroyed while idle; keep running in the tray.
            if let RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        });
}
