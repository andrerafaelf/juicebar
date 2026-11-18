//! Automatic updates. A background thread checks the GitHub release feed (`latest.json`,
//! signed with the key whose public half is in `tauri.conf.json`) shortly after start and
//! then a few times a day. A newer version is downloaded, verified and installed silently;
//! the installer closes Juicebar and starts the new version with the same arguments.

use crate::AppState;
use std::sync::mpsc::channel;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

/// Let startup (first poll, widget attach) settle before touching the network.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(6 * 3600);

pub fn spawn(app: AppHandle) {
    // Dev builds and demo sessions never replace the installed app.
    if cfg!(debug_assertions) || std::env::var_os("JUICEBAR_DEMO").is_some() {
        return;
    }
    let _ = std::thread::Builder::new().name("updater".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            if let Err(e) = tauri::async_runtime::block_on(check_and_install(&app)) {
                eprintln!("update check failed: {e}");
            }
            std::thread::sleep(EVERY);
        }
    });
}

async fn check_and_install(app: &AppHandle) -> tauri_plugin_updater::Result<()> {
    let handle = app.clone();
    let updater = app.updater_builder().on_before_exit(move || shut_down(&handle)).build()?;
    let Some(update) = updater.check().await? else { return Ok(()) };
    // On Windows this runs the installer and exits the process.
    update.download_and_install(|_, _| {}, || {}).await
}

/// The installer ends the process with `exit(0)`: save history and remove the widget and
/// tray icons first, as a normal quit would.
fn shut_down(app: &AppHandle) {
    #[cfg(windows)]
    crate::widget::stop();
    app.state::<AppState>().engine.shutdown();
    let (tx, rx) = channel();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        handle.cleanup_before_exit();
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(2));
}
