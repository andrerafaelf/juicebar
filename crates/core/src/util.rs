use crate::model::DeviceKind;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn now_secs() -> u64 {
    now_ms() / 1000
}

/// Lower-case alphanumerics only; used to match the same physical device seen
/// through two different providers (e.g. HID++ and the Windows Bluetooth stack).
pub fn normalize_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// Remove transport noise from product strings ("Razer Viper Ultimate Dongle" -> "Razer Viper Ultimate").
pub fn clean_name(name: &str) -> String {
    const SUFFIXES: [&str; 6] = [" Dongle", " Receiver", " Wireless Receiver", " USB Receiver", " Hands-Free AG", " Hands-Free"];
    let mut s = name.trim().to_string();
    for suffix in SUFFIXES {
        if s.len() > suffix.len() && s.to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase()) {
            s.truncate(s.len() - suffix.len());
        }
    }
    s.trim().to_string()
}

/// Best-effort device classification from a product name.
pub fn guess_kind(name: &str) -> DeviceKind {
    let n = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if n.contains("airpods max") {
        DeviceKind::Headset
    } else if has(&[
        "airpods pro",
        "buds",
        "earbuds",
        "airpods",
        "freebuds",
        "earfun",
        "galaxy buds",
        "pixel buds",
        "jabra elite",
        "wf-",
        "beats fit",
        "powerbeats",
        "studio buds",
    ]) {
        DeviceKind::Earbuds
    } else if has(&[
        "headset",
        "headphone",
        "arctis",
        "blackshark",
        "kraken",
        "barracuda",
        "cloud",
        "void",
        "virtuoso",
        "hs80",
        "hs70",
        "g533",
        "g535",
        "g733",
        "g933",
        "g935",
        "inzone",
        "wh-",
        "airpods max",
        "quantum",
        "nari",
        "astro",
        "maxwell",
        "audeze",
        "qc45",
        "quietcomfort",
        "bose 700",
        "momentum",
        "jbl live",
        "jbl tune",
        "evolve",
        "h9 ii",
    ]) {
        DeviceKind::Headset
    } else if has(&[
        "controller",
        "gamepad",
        "dualsense",
        "dualshock",
        "xbox",
        "joy-con",
        "pro controller",
        "8bitdo",
        "wolverine",
        "kishi",
        "gamesir",
        "flydigi",
        "steam deck",
        "stadia",
    ]) {
        DeviceKind::Gamepad
    } else if has(&[
        "keyboard",
        "keys",
        "blackwidow",
        "huntsman",
        "apex",
        "k70",
        "k100",
        "k65",
        "mx keys",
        "craft",
        "keychron",
        "ornata",
        "deathstalker",
        "azoth",
        "falchion",
        "pop keys",
        "g915",
        "g913",
        "g715",
        "magic keyboard",
        "aula",
        "ajazz",
        "nuphy",
    ]) {
        DeviceKind::Keyboard
    } else if has(&[
        "mouse",
        "mice",
        "mx master",
        "mx anywhere",
        "mx ergo",
        "trackball",
        "trackpad",
        "viper",
        "deathadder",
        "basilisk",
        "naga",
        "pulsefire",
        "aerox",
        "rival",
        "prime",
        "g pro",
        "g502",
        "g305",
        "g703",
        "g903",
        "superlight",
        "lift",
        "pebble",
        "model o",
        "model d",
        "xm2",
        "op1",
        "zowie",
        "lamzu",
        "pulsar",
        "vaxee",
        "finalmouse",
        "magic mouse",
        "harpe",
        "gladius",
        "keris",
        "chakram",
    ]) {
        DeviceKind::Mouse
    } else if has(&["speaker", "soundlink", "boom", "flip", "charge 5", "sonos", "soundcore motion"]) {
        DeviceKind::Speaker
    } else if has(&["watch", "band", "fitbit", "garmin"]) {
        DeviceKind::Watch
    } else if has(&["pen", "stylus", "pencil"]) {
        DeviceKind::Pen
    } else if has(&["phone", "iphone", "galaxy s", "pixel ", "oneplus"]) {
        DeviceKind::Phone
    } else {
        DeviceKind::Other
    }
}

/// Approximate state of charge of a single Li-ion / Li-po cell from its voltage.
/// Piecewise-linear over a typical 3.5–4.2 V discharge curve.
pub fn lipo_percent(millivolts: u16) -> u8 {
    const CURVE: [(u16, u8); 13] = [
        (4186, 100),
        (4067, 90),
        (3989, 80),
        (3922, 70),
        (3859, 60),
        (3811, 50),
        (3778, 40),
        (3751, 30),
        (3717, 20),
        (3671, 10),
        (3646, 5),
        (3579, 2),
        (3500, 0),
    ];
    let mv = millivolts;
    if mv >= CURVE[0].0 {
        return 100;
    }
    for pair in CURVE.windows(2) {
        let (hi_v, hi_p) = pair[0];
        let (lo_v, lo_p) = pair[1];
        if mv >= lo_v {
            let span = (hi_v - lo_v) as f32;
            let t = (mv - lo_v) as f32 / span;
            return (lo_p as f32 + t * (hi_p - lo_p) as f32).round() as u8;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voltage_curve_is_monotonic_and_clamped() {
        assert_eq!(lipo_percent(4300), 100);
        assert_eq!(lipo_percent(3400), 0);
        assert_eq!(lipo_percent(3811), 50);
        let mut last = 0;
        for mv in (3400..4300).step_by(5) {
            let p = lipo_percent(mv);
            assert!(p >= last, "{mv} mV gave {p} < {last}");
            last = p;
        }
    }

    #[test]
    fn names() {
        assert_eq!(clean_name("Razer Viper Ultimate Dongle"), "Razer Viper Ultimate");
        assert_eq!(clean_name("WH-1000XM4 Hands-Free AG"), "WH-1000XM4");
        assert_eq!(normalize_name("MX Master 3S"), "mxmaster3s");
        assert_eq!(guess_kind("Galaxy Buds2 Pro"), DeviceKind::Earbuds);
        assert_eq!(guess_kind("Arctis Nova 7"), DeviceKind::Headset);
        assert_eq!(guess_kind("DualSense Wireless Controller"), DeviceKind::Gamepad);
        assert_eq!(guess_kind("MX Keys S"), DeviceKind::Keyboard);
        assert_eq!(guess_kind("PRO X SUPERLIGHT 2"), DeviceKind::Mouse);
    }
}
