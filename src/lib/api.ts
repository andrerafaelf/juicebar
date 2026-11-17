import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Sample, Settings, Snapshot } from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  refresh: () => invoke<void>("refresh"),
  history: (key: string) => invoke<Sample[]>("get_history", { key }),
  forget: (key: string) => invoke<void>("forget_device", { key }),
  fit: (height: number) => invoke<void>("fit_popup", { height }),
  hide: () => invoke<void>("hide_popup"),
  autostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  quit: () => invoke<void>("quit_app"),
  version: () => invoke<string>("app_version"),
  takeFocus: () => invoke<string | null>("take_focus"),
  onFocus: (cb: (key: string) => void): Promise<UnlistenFn> => listen<string>("focus-device", (e) => cb(e.payload)),
  onSnapshot: (cb: (s: Snapshot) => void): Promise<UnlistenFn> => listen<Snapshot>("snapshot", (e) => cb(e.payload)),
};
