import type { Device, Transport } from "./types";

export type Tone = "ok" | "warn" | "low" | "charging" | "idle";

export function tone(d: Pick<Device, "level" | "charge" | "online">, threshold: number): Tone {
  if (!d.online) return "idle";
  if (d.charge === "charging" || d.charge === "full") return "charging";
  if (d.level <= threshold) return "low";
  if (d.level <= threshold + 15) return "warn";
  return "ok";
}

export function duration(minutes: number): string {
  if (minutes < 1) return "<1m";
  if (minutes < 60) return `${Math.round(minutes)}m`;
  const h = Math.floor(minutes / 60);
  const m = Math.round(minutes % 60);
  if (h >= 48) return `${Math.round(h / 24)}d`;
  return m && h < 10 ? `${h}h ${m}m` : `${h}h`;
}

export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 10) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

export const transportLabel: Record<Transport, string> = {
  usb: "USB",
  receiver: "Receiver",
  bluetooth: "Bluetooth",
  ble: "Bluetooth LE",
  unknown: "Wireless",
};

export const sourceLabel: Record<string, string> = {
  "logitech-hidpp": "Logitech HID++",
  g435: "G435 dongle bridge",
  razer: "Razer protocol",
  playstation: "PlayStation report",
  sonix: "SONiX dongle",
  headsetcontrol: "HeadsetControl",
  xinput: "XInput",
  bluetooth: "Windows Bluetooth",
  airpods: "Apple proximity data",
  "hid-battery": "HID battery usage",
  sysfs: "Linux power_supply",
  demo: "Demo",
};

/** One-line status under the device name. */
export function status(d: Device): string {
  if (!d.online) return `Asleep · seen ${ago(d.lastSeen)}`;
  if (d.charge === "full") return "Fully charged";
  if (d.charge === "charging") {
    return d.estimate?.toFull ? `Charging · full in ${duration(d.estimate.minutes)}` : "Charging";
  }
  if (d.estimate && !d.estimate.toFull) return `${duration(d.estimate.minutes)} left`;
  return transportLabel[d.transport];
}

export function levelText(level: number, step: number): string {
  return `${step > 1 ? "~" : ""}${level}%`;
}
