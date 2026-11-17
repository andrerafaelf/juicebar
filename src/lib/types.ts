// Mirrors juicebar-core's serialized model (serde camelCase).

export type DeviceKind =
  | "headset"
  | "earbuds"
  | "mouse"
  | "keyboard"
  | "gamepad"
  | "speaker"
  | "phone"
  | "watch"
  | "pen"
  | "other";

export type Transport = "usb" | "receiver" | "bluetooth" | "ble" | "unknown";
export type Charge = "discharging" | "charging" | "full" | "unknown";

export interface Battery {
  label: string | null;
  level: number;
  charge: Charge;
  step: number;
}

export interface Estimate {
  minutes: number;
  toFull: boolean;
  ratePerHour: number;
}

export interface Device {
  key: string;
  name: string;
  vendor: string | null;
  kind: DeviceKind;
  transport: Transport;
  batteries: Battery[];
  level: number;
  charge: Charge;
  step: number;
  source: string;
  online: boolean;
  lastSeen: number;
  estimate: Estimate | null;
}

export interface Snapshot {
  devices: Device[];
  updatedAt: number;
}

export interface Sample {
  t: number;
  level: number;
  charging: boolean;
}

export type TrayMode = "off" | "single" | "per-device";

export interface Settings {
  lowThreshold: number;
  notifyLow: boolean;
  notifyFull: boolean;
  pollSeconds: number;
  trayMode: TrayMode;
  taskbarWidget: boolean;
  widgetStyle: "detailed" | "compact";
  showOffline: boolean;
  hidden: string[];
  aliases: [string, string][];
  headsetcontrolPath: string | null;
}
