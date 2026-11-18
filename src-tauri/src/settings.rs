use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrayMode {
    /// No tray icon (only when the taskbar widget is showing).
    Off,
    /// One tray icon showing the lowest battery.
    Single,
    /// One tray icon per device.
    PerDevice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WidgetStyle {
    /// Device name on top, gauge + percentage below (two lines, like the clock).
    Detailed,
    /// Icon + percentage on one line.
    Compact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub low_threshold: u8,
    pub notify_low: bool,
    pub notify_full: bool,
    pub poll_seconds: u32,
    pub tray_mode: TrayMode,
    /// Draw device levels inside the taskbar, next to the notification area.
    pub taskbar_widget: bool,
    pub widget_style: WidgetStyle,
    pub show_offline: bool,
    pub hidden: Vec<String>,
    /// Custom display names, keyed by device key.
    pub aliases: Vec<(String, String)>,
    pub headsetcontrol_path: Option<String>,
    /// Minutes between Logitech G435 reads (only while its audio is idle); 0 = off.
    pub g435_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            low_threshold: 20,
            notify_low: true,
            notify_full: true,
            poll_seconds: 60,
            tray_mode: TrayMode::Off,
            taskbar_widget: true,
            widget_style: WidgetStyle::Detailed,
            show_offline: true,
            hidden: Vec::new(),
            aliases: Vec::new(),
            headsetcontrol_path: None,
            g435_minutes: 10,
        }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        self.low_threshold = self.low_threshold.clamp(5, 50);
        self.poll_seconds = self.poll_seconds.clamp(15, 600);
        self.headsetcontrol_path = self.headsetcontrol_path.filter(|p| !p.trim().is_empty());
        if self.g435_minutes != 0 {
            self.g435_minutes = self.g435_minutes.clamp(2, 120);
        }
        self
    }

    pub fn alias(&self, key: &str) -> Option<&str> {
        self.aliases.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str::<Settings>(&s).ok()).unwrap_or_default().sanitized()
}

pub fn save(path: &Path, settings: &Settings) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        write_atomic(path, json.as_bytes());
    }
}

/// Write to a temp file then rename, so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp: PathBuf = path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}
