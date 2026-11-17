//! Background thread that owns the [`Monitor`], schedules polls, reacts to device
//! changes and persists history. The UI talks to it through [`Handle`].

use crate::alerts::Alerts;
use crate::settings::write_atomic;
use crate::{on_devices, AppState};
use juicebar_core::history::History;
use juicebar_core::monitor::KnownDevice;
use juicebar_core::{Monitor, MonitorOptions, ProviderOptions, Sample};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const SAVE_EVERY: Duration = Duration::from_secs(10 * 60);
const DEVICE_CHANGE_DEBOUNCE: Duration = Duration::from_millis(2500);

pub enum Cmd {
    Refresh,
    DeviceChange,
    Forget(String),
    History(String, Sender<Vec<Sample>>),
    Configure(ProviderOptions, Duration),
    Shutdown(Sender<()>),
}

#[derive(Clone)]
pub struct Handle {
    tx: Sender<Cmd>,
}

impl Handle {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn history(&self, key: String) -> Vec<Sample> {
        let (tx, rx) = channel();
        self.send(Cmd::History(key, tx));
        rx.recv_timeout(Duration::from_secs(15)).unwrap_or_default()
    }

    /// Persist history and stop the thread (bounded wait).
    pub fn shutdown(&self) {
        let (tx, rx) = channel();
        self.send(Cmd::Shutdown(tx));
        let _ = rx.recv_timeout(Duration::from_secs(3));
    }
}

pub fn load_history(path: &Path) -> History {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn devices_path(history_path: &Path) -> PathBuf {
    history_path.with_file_name("devices.json")
}

/// Persist history and the last known devices (so asleep devices still show after a restart).
fn save_state(history_path: &Path, monitor: &Monitor) {
    if let Some(dir) = history_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec(&monitor.history) {
        write_atomic(history_path, &bytes);
    }
    if let Ok(bytes) = serde_json::to_vec(&monitor.known()) {
        write_atomic(&devices_path(history_path), &bytes);
    }
}

pub fn spawn(app: AppHandle, providers: ProviderOptions, interval: Duration, history_path: PathBuf) -> Handle {
    let (tx, rx) = channel();
    let handle = Handle { tx: tx.clone() };
    std::thread::Builder::new()
        .name("juicebar-monitor".into())
        .spawn(move || run(app, rx, tx, providers, interval, history_path))
        .expect("spawn monitor thread");
    handle
}

fn run(app: AppHandle, rx: Receiver<Cmd>, tx: Sender<Cmd>, providers: ProviderOptions, mut interval: Duration, history_path: PathBuf) {
    // Setup registers AppState right after spawning us; don't poll before it exists.
    while app.try_state::<AppState>().is_none() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut monitor = Monitor::new(MonitorOptions { providers, ..Default::default() });
    monitor.history = load_history(&history_path);
    let known: Vec<KnownDevice> =
        std::fs::read(devices_path(&history_path)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    monitor.restore(known);
    let mut alerts = Alerts::default();
    // Show last known devices right away, before the first (slower) poll completes.
    if let Some(state) = app.try_state::<AppState>() {
        on_devices(&app, &state, monitor.snapshot(), &mut alerts);
    }
    let mut known_keys: Vec<String> = Vec::new();

    #[cfg(windows)]
    let _watcher = {
        let tx = std::sync::Mutex::new(tx.clone());
        juicebar_core::watch::watch(move || {
            if let Ok(tx) = tx.lock() {
                let _ = tx.send(Cmd::DeviceChange);
            }
        })
    };
    drop(tx);

    let mut full_scan = true;
    let mut next_poll = Instant::now();
    let mut last_save = Instant::now();
    let mut dirty = false;

    loop {
        if Instant::now() >= next_poll {
            let devices = monitor.poll(full_scan);
            full_scan = false;
            dirty = true;
            // Persist right away when a device appears or disappears.
            let keys: Vec<String> = devices.iter().map(|d| d.key.clone()).collect();
            if keys != known_keys {
                save_state(&history_path, &monitor);
                known_keys = keys;
            }
            if let Some(state) = app.try_state::<AppState>() {
                on_devices(&app, &state, devices, &mut alerts);
            }
            next_poll = Instant::now() + interval;
        }
        if dirty && last_save.elapsed() >= SAVE_EVERY {
            save_state(&history_path, &monitor);
            last_save = Instant::now();
            dirty = false;
        }

        let wait = next_poll.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Cmd::Refresh) => {
                full_scan = true;
                next_poll = Instant::now();
            }
            Ok(Cmd::DeviceChange) => {
                full_scan = true;
                next_poll = next_poll.min(Instant::now() + DEVICE_CHANGE_DEBOUNCE);
            }
            Ok(Cmd::Forget(key)) => {
                monitor.forget(&key);
                if let Some(state) = app.try_state::<AppState>() {
                    on_devices(&app, &state, monitor.snapshot(), &mut alerts);
                }
            }
            Ok(Cmd::History(key, reply)) => {
                let _ = reply.send(monitor.history.series(&key).to_vec());
            }
            Ok(Cmd::Configure(p, i)) => {
                let history = std::mem::take(&mut monitor.history);
                monitor = Monitor::new(MonitorOptions { providers: p, ..Default::default() });
                monitor.history = history;
                interval = i;
                full_scan = true;
                next_poll = Instant::now();
            }
            Ok(Cmd::Shutdown(ack)) => {
                save_state(&history_path, &monitor);
                let _ = ack.send(());
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
