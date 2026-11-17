//! Tray icon rasteriser: the battery percentage drawn as crisp pixel digits, sized to
//! the exact small-icon size of the current DPI so Windows never rescales it.

use juicebar_core::Charge;

/// 3×5 pixel font for digits 0-9 (rows top to bottom, 3 bits each, MSB = left).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconState {
    pub level: Option<u8>,
    pub charge: Charge,
    pub online: bool,
    pub low_threshold: u8,
}

type Rgba = [u8; 4];

fn palette(s: &IconState, light_taskbar: bool) -> Rgba {
    let pick = |dark: Rgba, light: Rgba| if light_taskbar { light } else { dark };
    let Some(level) = s.level else { return pick([156, 163, 175, 255], [107, 114, 128, 255]) };
    if !s.online {
        pick([156, 163, 175, 200], [107, 114, 128, 200])
    } else if matches!(s.charge, Charge::Charging | Charge::Full) {
        pick([74, 222, 128, 255], [21, 128, 61, 255])
    } else if level <= s.low_threshold {
        pick([248, 113, 113, 255], [220, 38, 38, 255])
    } else if level <= s.low_threshold.saturating_add(15) {
        pick([251, 191, 36, 255], [180, 83, 9, 255])
    } else {
        pick([255, 255, 255, 255], [17, 24, 39, 255])
    }
}

struct Canvas {
    size: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(size: u32) -> Self {
        Self { size, px: vec![0; (size * size * 4) as usize] }
    }
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        for yy in y.max(0)..(y + h).min(self.size as i32) {
            for xx in x.max(0)..(x + w).min(self.size as i32) {
                let i = ((yy as u32 * self.size + xx as u32) * 4) as usize;
                self.px[i..i + 4].copy_from_slice(&c);
            }
        }
    }
}

/// Battery outline with a fill bar: used for 100% and "no data".
fn draw_battery(c: &mut Canvas, fill: Option<u8>, color: Rgba) {
    let s = c.size as i32;
    let t = (s / 16).max(1); // stroke
    let (w, h) = (s - 2 * t - t * 2, s / 2);
    let (x, y) = (t, (s - h) / 2);
    c.fill(x, y, w, t, color);
    c.fill(x, y + h - t, w, t, color);
    c.fill(x, y, t, h, color);
    c.fill(x + w - t, y, t, h, color);
    c.fill(x + w, y + h / 3, t * 2, h - 2 * (h / 3), color); // nub
    if let Some(level) = fill {
        let inner = w - 4 * t;
        let fw = (inner * level as i32 / 100).max(if level > 0 { t } else { 0 });
        c.fill(x + 2 * t, y + 2 * t, fw, h - 4 * t, color);
    }
}

fn draw_number(c: &mut Canvas, n: u8, color: Rgba) {
    let digits: Vec<usize> = n.to_string().bytes().map(|b| (b - b'0') as usize).collect();
    let count = digits.len() as i32;
    let size = c.size as i32;
    // Largest integer scale where the digits (3px wide, 1px gaps) fit.
    let scale = (1..=size).rev().find(|&s| count * 3 * s + (count - 1) * s <= size && 5 * s <= size).unwrap_or(1);
    let gap = scale;
    let total_w = count * 3 * scale + (count - 1) * gap;
    let mut x = (size - total_w) / 2;
    let y = (size - 5 * scale) / 2;
    for d in digits {
        for (row, bits) in DIGITS[d].iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 == 1 {
                    c.fill(x + col * scale, y + row as i32 * scale, scale, scale, color);
                }
            }
        }
        x += 3 * scale + gap;
    }
}

/// Render an RGBA icon of `size`×`size` pixels.
pub fn render(state: &IconState, size: u32, light_taskbar: bool) -> Vec<u8> {
    let mut c = Canvas::new(size);
    let color = palette(state, light_taskbar);
    match state.level {
        None => draw_battery(&mut c, None, color),
        Some(l) if l >= 100 => draw_battery(&mut c, Some(100), color),
        Some(l) => draw_number(&mut c, l, color),
    }
    c.px
}

/// Small-icon size in pixels for the system DPI, and whether the taskbar uses a light theme.
/// True when the taskbar uses the light theme (dark text).
#[cfg(windows)]
pub fn light_taskbar() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value: u32 = 0;
    let mut len = std::mem::size_of::<u32>() as u32;
    // SAFETY: valid out-pointers sized for a DWORD.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut len),
        )
        .is_ok()
            && value == 1
    }
}

#[cfg(windows)]
pub fn system_metrics() -> (u32, bool) {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;
    // SAFETY: plain Win32 metric query.
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.clamp(16, 64) as u32;
    (size, light_taskbar())
}

#[cfg(not(windows))]
pub fn system_metrics() -> (u32, bool) {
    (32, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `JUICEBAR_ICON_DUMP=dir cargo test -p juicebar dump_icons -- --ignored` writes raw RGBA previews.
    #[test]
    #[ignore]
    fn dump_icons() {
        let dir = std::env::var("JUICEBAR_ICON_DUMP").expect("set JUICEBAR_ICON_DUMP");
        for size in [16u32, 24, 32] {
            for (i, (level, charge)) in [
                (Some(7), Charge::Discharging),
                (Some(18), Charge::Discharging),
                (Some(42), Charge::Discharging),
                (Some(64), Charge::Charging),
                (Some(100), Charge::Full),
                (None, Charge::Unknown),
            ]
            .into_iter()
            .enumerate()
            {
                let s = IconState { level, charge, online: true, low_threshold: 20 };
                std::fs::write(format!("{dir}/icon_{size}_{i}.rgba"), render(&s, size, false)).unwrap();
            }
        }
    }

    #[test]
    fn renders_every_size() {
        for size in [16, 20, 24, 32, 48] {
            for level in [None, Some(0), Some(7), Some(42), Some(100)] {
                let s = IconState { level, charge: Charge::Discharging, online: true, low_threshold: 20 };
                let px = render(&s, size, false);
                assert_eq!(px.len(), (size * size * 4) as usize);
                assert!(px.chunks(4).any(|p| p[3] > 0), "blank icon {size} {level:?}");
            }
        }
    }
}
