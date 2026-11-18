//! Taskbar widget: device icons and battery levels drawn *inside* the Windows taskbar,
//! just left of the notification area.
//!
//! It is a native layered child window of `Shell_TrayWnd` (no WebView), rendered with
//! Direct2D + DirectWrite into a premultiplied-alpha bitmap so text stays crisp over the
//! translucent taskbar. Runs on its own thread with its own message loop.
//!
//! Two styles:
//! - **Detailed** (default) mirrors the two-line clock: device name on top, a mini
//!   battery gauge and percentage below.
//! - **Compact**: icon + percentage on one line, for crowded taskbars.
//!
//! Everything is aligned on *ink* bounds measured with DirectWrite, not on font line
//! boxes, so icons and digits are optically centred whatever the font metrics.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    Normal,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Item {
    /// Device key, reported back on click.
    pub key: String,
    pub name: String,
    pub glyph: char,
    pub level: u8,
    /// Coarse readings show "~".
    pub approx: bool,
    pub tone: Tone,
    pub charging: bool,
    pub asleep: bool,
}

pub enum Event {
    /// Left click: the widget's screen rectangle (to anchor the flyout) and the device
    /// under the cursor, if any.
    Click(RECT, Option<String>),
    Menu(MenuCommand),
    /// The widget attached to or detached from the taskbar (see [`is_attached`]).
    AttachmentChanged,
}

#[derive(Debug, Clone, Copy)]
pub enum MenuCommand {
    Open,
    Refresh,
    Quit,
}

/// Defined in Win32_UI_Controls; declared here to avoid pulling in that feature.
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_APP_REDRAW: u32 = WM_APP + 1;
const WM_APP_STOP: u32 = WM_APP + 2;
const TIMER_ID: usize = 1;

type Callback = Box<dyn Fn(Event) + Send + Sync>;

struct Shared {
    items: Mutex<Vec<Item>>,
    callback: Callback,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
static HOST: AtomicIsize = AtomicIsize::new(0);
static ATTACHED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(true);
static COMPACT: AtomicBool = AtomicBool::new(false);

/// Whether the widget is currently visible inside the taskbar.
pub fn is_attached() -> bool {
    ATTACHED.load(Ordering::SeqCst)
}

/// Start the widget thread (once). `callback` runs on the widget thread.
pub fn start(callback: impl Fn(Event) + Send + Sync + 'static) {
    if SHARED.set(Shared { items: Mutex::new(Vec::new()), callback: Box::new(callback) }).is_err() {
        return;
    }
    std::thread::Builder::new().name("juicebar-widget".into()).spawn(run).expect("spawn widget thread");
}

pub fn configure(enabled: bool, compact: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
    COMPACT.store(compact, Ordering::SeqCst);
    post(WM_APP_REDRAW);
}

pub fn update(items: Vec<Item>) {
    if let Some(s) = SHARED.get() {
        *s.items.lock().unwrap() = items;
        post(WM_APP_REDRAW);
    }
}

pub fn stop() {
    post(WM_APP_STOP);
}

fn post(msg: u32) {
    let host = HOST.load(Ordering::SeqCst);
    if host != 0 {
        // SAFETY: posting to a window owned by our widget thread; harmless if it is gone.
        unsafe {
            let _ = PostMessageW(Some(HWND(host as _)), msg, WPARAM(0), LPARAM(0));
        }
    }
}

fn emit(e: Event) {
    if let Some(s) = SHARED.get() {
        (s.callback)(e);
    }
}

/// Segoe Fluent Icons (Windows 11) / Segoe MDL2 Assets (Windows 10) code points.
pub mod glyph {
    pub const HEADSET: char = '\u{E7F6}';
    pub const MOUSE: char = '\u{E962}';
    pub const KEYBOARD: char = '\u{E765}';
    pub const GAMEPAD: char = '\u{E7FC}';
    pub const PHONE: char = '\u{E8EA}';
    pub const SPEAKER: char = '\u{E7F5}';
    pub const PEN: char = '\u{EDC6}';
    pub const WATCH: char = '\u{E916}';
    pub const BLUETOOTH: char = '\u{E702}';
    pub const BOLT: char = '\u{E945}';
}

// ------------------------------------------------------------------ widget thread

#[derive(Default)]
struct State {
    widget: Option<HWND>,
    taskbar: Option<HWND>,
    hovered: Option<usize>,
    gfx: Option<Gfx>,
    /// Item boxes from the last paint (x, width), for hit-testing.
    boxes: Vec<(f32, f32)>,
    keys: Vec<String>,
    /// Hash of everything that affects the pixels, to skip redundant redraws.
    last: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn run() {
    // SAFETY: Win32 window + message loop owned by this thread.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let Ok(module) = GetModuleHandleW(None) else { return };
        let hinstance = module.into();
        let procs: [(PCWSTR, WNDPROC); 2] = [(w!("JuicebarHost"), Some(host_proc)), (w!("JuicebarTaskbarWidget"), Some(widget_proc))];
        for (name, proc) in procs {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: proc,
                hInstance: hinstance,
                lpszClassName: name,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassExW(&class);
        }
        let Ok(host) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("JuicebarHost"),
            w!("Juicebar"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        ) else {
            return;
        };
        HOST.store(host.0 as isize, Ordering::SeqCst);
        // Re-check the taskbar layout periodically: tray icons come and go, Explorer restarts.
        SetTimer(Some(host), TIMER_ID, 1000, None);
        refresh();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        HOST.store(0, Ordering::SeqCst);
    }
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER | WM_APP_REDRAW => {
            refresh();
            LRESULT(0)
        }
        WM_APP_STOP => {
            STATE.with_borrow_mut(|s| {
                if let Some(w) = s.widget.take() {
                    let _ = unsafe { DestroyWindow(w) };
                }
            });
            let _ = unsafe { DestroyWindow(hwnd) };
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn hit(s: &State, x: i32) -> Option<usize> {
    s.boxes.iter().position(|(bx, bw)| (x as f32) >= *bx && (x as f32) < bx + bw)
}

unsafe extern "system" fn widget_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let x = (lp.0 & 0xFFFF) as i16 as i32;
    match msg {
        WM_MOUSEMOVE => {
            let (changed, first) = STATE.with_borrow_mut(|s| {
                let now = hit(s, x);
                let first = s.hovered.is_none();
                let changed = now != s.hovered;
                s.hovered = now.or(Some(usize::MAX)); // MAX = over the strip but between items
                (changed, first)
            });
            if first {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = unsafe { TrackMouseEvent(&mut tme) };
            }
            if changed {
                refresh();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            STATE.with_borrow_mut(|s| s.hovered = None);
            refresh();
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let mut r = RECT::default();
            let _ = unsafe { GetWindowRect(hwnd, &mut r) };
            let key = STATE.with_borrow(|s| hit(s, x).and_then(|i| s.keys.get(i).cloned()));
            emit(Event::Click(r, key));
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            show_menu();
            LRESULT(0)
        }
        WM_DESTROY => {
            STATE.with_borrow_mut(|s| {
                if s.widget == Some(hwnd) {
                    s.widget = None;
                }
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn show_menu() {
    let host = HWND(HOST.load(Ordering::SeqCst) as _);
    // SAFETY: standard popup-menu dance; the host window is ours.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, 1, w!("Open Juicebar"));
        let _ = AppendMenuW(menu, MF_STRING, 2, w!("Refresh now"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, 3, w!("Quit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(host);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, host, None);
        let _ = PostMessageW(Some(host), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd.0 {
            1 => emit(Event::Menu(MenuCommand::Open)),
            2 => emit(Event::Menu(MenuCommand::Refresh)),
            3 => emit(Event::Menu(MenuCommand::Quit)),
            _ => {}
        }
    }
}

fn set_attached(v: bool) {
    if ATTACHED.swap(v, Ordering::SeqCst) != v {
        emit(Event::AttachmentChanged);
    }
}

fn hide(s: &mut State) {
    if let Some(w) = s.widget {
        // SAFETY: our window.
        unsafe {
            let _ = ShowWindow(w, SW_HIDE);
        }
    }
    s.last = 0;
    s.boxes.clear();
}

fn refresh() {
    STATE.with_borrow_mut(|s| {
        let items = SHARED.get().map(|x| x.items.lock().unwrap().clone()).unwrap_or_default();
        let compact = COMPACT.load(Ordering::SeqCst);
        // SAFETY: Win32 calls on windows we own or only query.
        unsafe {
            let Ok(taskbar) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) else {
                hide(s);
                set_attached(false);
                return;
            };
            let mut tb = RECT::default();
            let _ = GetWindowRect(taskbar, &mut tb);
            // Vertical taskbars have no room for a horizontal strip: fall back to the tray.
            if !ENABLED.load(Ordering::SeqCst) || (tb.bottom - tb.top) > (tb.right - tb.left) {
                hide(s);
                set_attached(false);
                return;
            }

            // (Re)create the child window if Explorer restarted or this is the first run.
            let alive = s.widget.is_some_and(|w| IsWindow(Some(w)).as_bool() && GetParent(w).ok() == Some(taskbar));
            if !alive {
                if let Some(w) = s.widget.take() {
                    let _ = DestroyWindow(w);
                }
                let hinstance = GetModuleHandleW(None).ok().map(Into::into);
                s.widget = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    w!("JuicebarTaskbarWidget"),
                    w!("Juicebar"),
                    WS_CHILD | WS_CLIPSIBLINGS,
                    0,
                    0,
                    1,
                    1,
                    Some(taskbar),
                    None,
                    hinstance,
                    None,
                )
                .ok();
                s.taskbar = Some(taskbar);
                s.last = 0;
            }
            let Some(widget) = s.widget else {
                set_attached(false);
                return;
            };
            if items.is_empty() {
                hide(s);
                set_attached(true);
                return;
            }

            let dpi = GetDpiForWindow(taskbar).max(96);
            let scale = dpi as f32 / 96.0;
            if s.gfx.as_ref().is_none_or(|g| (g.k - scale).abs() > f32::EPSILON) {
                s.gfx = Gfx::new(scale).ok();
            }
            let Some(gfx) = s.gfx.as_mut() else { return };

            // Notification area in taskbar client coordinates: we sit just left of it.
            let mut client = RECT::default();
            let _ = GetClientRect(taskbar, &mut client);
            let right = match FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), PCWSTR::null()) {
                Ok(n) => {
                    let mut r = RECT::default();
                    let _ = GetWindowRect(n, &mut r);
                    let mut pts = [POINT { x: r.left, y: r.top }];
                    MapWindowPoints(None, Some(taskbar), &mut pts);
                    pts[0].x
                }
                Err(_) => client.right - (200.0 * scale) as i32,
            };
            let light = crate::icon::light_taskbar();
            let height = client.bottom - client.top;
            let layout = gfx.layout(&items, compact);
            let width = layout.width.ceil() as i32;
            let x = right - width - (2.0 * scale) as i32;
            let hovered = s.hovered.filter(|i| *i < items.len());

            let sig = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (&items, compact, x, width, height, dpi, light, hovered).hash(&mut h);
                h.finish()
            };
            if sig != s.last {
                let _ = SetWindowPos(widget, Some(HWND_TOP), x, 0, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                if gfx.paint(widget, &items, &layout, width, height, light, hovered).is_ok() {
                    s.last = sig;
                }
                s.boxes = layout.items.iter().map(|b| (b.x, b.w)).collect();
                s.keys = items.iter().map(|i| i.key.clone()).collect();
            }
            set_attached(true);
        }
    });
}

// ------------------------------------------------------------------ drawing

/// Design sizes in DIPs (scaled by the taskbar DPI).
mod dip {
    pub const OUTER: f32 = 2.0; // strip padding
    pub const HPAD: f32 = 7.0; // padding inside an item's hover box
    pub const ITEM_GAP: f32 = 2.0; // between hover boxes
    pub const ICON: f32 = 20.0; // detailed glyph size
    pub const ICON_SMALL: f32 = 16.0; // compact glyph size
    pub const ICON_GAP: f32 = 8.0;
    pub const ICON_GAP_SMALL: f32 = 6.0;
    pub const TEXT: f32 = 12.0;
    pub const NAME_MAX: f32 = 116.0;
    pub const GAUGE_W: f32 = 17.0;
    pub const GAUGE_H: f32 = 8.0;
    pub const GAUGE_GAP: f32 = 5.0;
    pub const BOLT: f32 = 10.0;
    pub const LINE_GAP: f32 = 1.0; // extra space between the two lines
}

struct ItemBox {
    /// Hover box.
    x: f32,
    w: f32,
    icon_x: f32,
    text_x: f32,
    name: String,
    value: String,
    value_w: f32,
}

struct Layout {
    width: f32,
    items: Vec<ItemBox>,
}

/// Ink bounds of a string relative to the origin it is drawn at.
#[derive(Clone, Copy)]
struct Ink {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Ink {
    fn cx(&self) -> f32 {
        (self.left + self.right) / 2.0
    }
    fn cy(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }
}

struct Gfx {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    name: IDWriteTextFormat,
    value: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_small: IDWriteTextFormat,
    bolt: IDWriteTextFormat,
    /// Line height of the 12px text.
    line_h: f32,
    /// Ink of digits ("0") within one value line.
    digits: Ink,
    /// Ink of capitals + lowercase ("Ag") within one name line.
    caps: Ink,
    glyph_ink: HashMap<(char, bool), Ink>,
    k: f32,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Gfx {
    fn new(k: f32) -> windows::core::Result<Self> {
        // SAFETY: factory creation; formats are plain value objects.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let icon_family = if has_font(&dwrite, w!("Segoe Fluent Icons")) { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") };
            let text_family = if has_font(&dwrite, w!("Segoe UI Variable Text")) { w!("Segoe UI Variable Text") } else { w!("Segoe UI") };
            let fmt = |family: PCWSTR, size: f32, weight: DWRITE_FONT_WEIGHT| -> windows::core::Result<IDWriteTextFormat> {
                let f = dwrite.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size * k,
                    w!("en-us"),
                )?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                Ok(f)
            };
            let mut g = Self {
                name: fmt(text_family, dip::TEXT, DWRITE_FONT_WEIGHT_NORMAL)?,
                value: fmt(text_family, dip::TEXT, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
                icon: fmt(icon_family, dip::ICON, DWRITE_FONT_WEIGHT_NORMAL)?,
                icon_small: fmt(icon_family, dip::ICON_SMALL, DWRITE_FONT_WEIGHT_NORMAL)?,
                bolt: fmt(icon_family, dip::BOLT, DWRITE_FONT_WEIGHT_NORMAL)?,
                d2d,
                dwrite,
                line_h: dip::TEXT * k * 1.33,
                digits: Ink { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 },
                caps: Ink { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 },
                glyph_ink: HashMap::new(),
                k,
            };
            if let Ok(l) = g.dwrite.CreateTextLayout(&wide("Ag"), &g.name, 1000.0, 1000.0) {
                let mut lines = [DWRITE_LINE_METRICS::default(); 1];
                let mut count = 0;
                if l.GetLineMetrics(Some(&mut lines), &mut count).is_ok() {
                    g.line_h = lines[0].height;
                }
            }
            g.digits = g.ink(&g.value.clone(), "0");
            g.caps = g.ink(&g.name.clone(), "H");
            Ok(g)
        }
    }

    fn text_layout(&self, s: &str, f: &IDWriteTextFormat) -> windows::core::Result<IDWriteTextLayout> {
        // SAFETY: layout over a local UTF-16 buffer.
        unsafe { self.dwrite.CreateTextLayout(&wide(s), f, 1000.0, 1000.0) }
    }

    fn width(&self, s: &str, f: &IDWriteTextFormat) -> f32 {
        self.text_layout(s, f)
            .and_then(|l| {
                let mut m = DWRITE_TEXT_METRICS::default();
                // SAFETY: valid out-pointer.
                unsafe { l.GetMetrics(&mut m) }.map(|_| m.widthIncludingTrailingWhitespace)
            })
            .unwrap_or(s.chars().count() as f32 * 7.0 * self.k)
    }

    /// Ink bounds of `s` drawn with its layout origin at (0, 0).
    fn ink(&self, f: &IDWriteTextFormat, s: &str) -> Ink {
        let fallback = Ink { left: 0.0, top: 0.0, right: self.k * 12.0, bottom: self.k * 12.0 };
        let Ok(l) = self.text_layout(s, f) else { return fallback };
        // SAFETY: plain metrics query.
        let Ok(o) = (unsafe { l.GetOverhangMetrics() }) else { return fallback };
        // Overhangs are measured against the 1000×1000 layout box.
        Ink { left: -o.left, top: -o.top, right: 1000.0 + o.right, bottom: 1000.0 + o.bottom }
    }

    fn glyph_ink(&mut self, g: char, small: bool) -> Ink {
        if let Some(i) = self.glyph_ink.get(&(g, small)) {
            return *i;
        }
        let f = if small { self.icon_small.clone() } else { self.icon.clone() };
        let ink = self.ink(&f, &g.to_string());
        self.glyph_ink.insert((g, small), ink);
        ink
    }

    fn truncate(&self, name: &str) -> String {
        let max = dip::NAME_MAX * self.k;
        if self.width(name, &self.name) <= max {
            return name.to_string();
        }
        let chars: Vec<char> = name.chars().collect();
        for n in (1..chars.len()).rev() {
            let candidate: String = chars[..n].iter().collect::<String>().trim_end().to_string() + "…";
            if self.width(&candidate, &self.name) <= max {
                return candidate;
            }
        }
        "…".into()
    }

    fn layout(&mut self, items: &[Item], compact: bool) -> Layout {
        let k = self.k;
        let mut x = dip::OUTER * k;
        let mut out = Vec::with_capacity(items.len());
        for (i, it) in items.iter().enumerate() {
            if i > 0 {
                x += dip::ITEM_GAP * k;
            }
            let value = format!("{}{}%", if it.approx { "~" } else { "" }, it.level);
            let value_w = self.width(&value, if compact { &self.name } else { &self.value });
            let bolt_w = if it.charging { (dip::BOLT + 2.0) * k } else { 0.0 };
            let (icon_w, gap) = if compact { (dip::ICON_SMALL, dip::ICON_GAP_SMALL) } else { (dip::ICON, dip::ICON_GAP) };
            let icon_x = x + dip::HPAD * k;
            let text_x = icon_x + (icon_w + gap) * k;
            let (name, text_w) = if compact {
                (String::new(), value_w + bolt_w)
            } else {
                let name = self.truncate(&it.name);
                let name_w = self.width(&name, &self.name);
                let line2 = (dip::GAUGE_W + 1.5 + dip::GAUGE_GAP) * k + value_w + bolt_w;
                (name, name_w.max(line2))
            };
            let w = text_x + text_w + dip::HPAD * k - x;
            out.push(ItemBox { x, w, icon_x, text_x, name, value, value_w });
            x += w;
        }
        Layout { width: x + dip::OUTER * k, items: out }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        hwnd: HWND,
        items: &[Item],
        layout: &Layout,
        w: i32,
        h: i32,
        light: bool,
        hovered: Option<usize>,
    ) -> windows::core::Result<()> {
        let k = self.k;
        let compact = COMPACT.load(Ordering::SeqCst);
        let hf = h as f32;
        // Pre-compute glyph inks (needs &mut self) before borrowing formats immutably.
        let glyph_inks: Vec<Ink> = items.iter().map(|it| self.glyph_ink(it.glyph, compact)).collect();
        let bolt_ink = self.ink(&self.bolt.clone(), &glyph::BOLT.to_string());

        // SAFETY: GDI/D2D resources are created and released within this call.
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(mem, dib.into());

            let result = (|| -> windows::core::Result<()> {
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    ..Default::default()
                };
                let rt = self.d2d.CreateDCRenderTarget(&props)?;
                rt.BindDC(mem, &RECT { left: 0, top: 0, right: w, bottom: h })?;
                rt.BeginDraw();
                rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                // Alpha 1/255 everywhere keeps the whole strip clickable (fully transparent
                // pixels of a layered window are click-through).
                rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 / 255.0 }));

                type Rgba = (f32, f32, f32, f32);
                let fg: Rgba = if light { (0.0, 0.0, 0.0, 0.9) } else { (1.0, 1.0, 1.0, 1.0) };
                let low: Rgba = if light { (0.77, 0.17, 0.11, 1.0) } else { (1.0, 0.55, 0.58, 1.0) };
                let green: Rgba = if light { (0.06, 0.48, 0.27, 1.0) } else { (0.42, 0.8, 0.37, 1.0) };
                let brush = |c: Rgba, alpha: f32| rt.CreateSolidColorBrush(&D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 * alpha }, None);
                let text_at = |s: &str, f: &IDWriteTextFormat, x: f32, y: f32, b: &ID2D1SolidColorBrush| {
                    if let Ok(l) = self.text_layout(s, f) {
                        rt.DrawTextLayout(windows_numerics::Vector2 { X: x, Y: y }, &l, b, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                };

                // Two-line block centred vertically, like the clock.
                let gap = dip::LINE_GAP * k;
                let block = 2.0 * self.line_h + gap;
                let top = ((hf - block) / 2.0).round();
                let line2 = top + self.line_h + gap;

                for (i, ((it, b), ink)) in items.iter().zip(&layout.items).zip(&glyph_inks).enumerate() {
                    if hovered == Some(i) {
                        let bg = brush(if light { (0.0, 0.0, 0.0, 0.055) } else { (1.0, 1.0, 1.0, 0.075) }, 1.0)?;
                        let inset = 4.0 * k;
                        let rr = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F { left: b.x, top: inset, right: b.x + b.w, bottom: hf - inset },
                            radiusX: 4.0 * k,
                            radiusY: 4.0 * k,
                        };
                        rt.FillRoundedRectangle(&rr, &bg);
                    }

                    let alpha = if it.asleep { 0.45 } else { 1.0 };
                    let accent = if it.asleep {
                        fg
                    } else if it.charging {
                        green
                    } else if it.tone == Tone::Low {
                        low
                    } else {
                        fg
                    };
                    let fg_b = brush(fg, alpha)?;

                    // Icon, optically centred on its ink.
                    let icon_box = if compact { dip::ICON_SMALL } else { dip::ICON } * k;
                    let ix = (b.icon_x + icon_box / 2.0 - ink.cx()).round();
                    let iy = (hf / 2.0 - ink.cy()).round();
                    text_at(&it.glyph.to_string(), if compact { &self.icon_small } else { &self.icon }, ix, iy, &fg_b);

                    let value_b = brush(if it.tone == Tone::Low && !it.asleep { low } else { fg }, alpha)?;
                    if compact {
                        // Digits centred on the strip's midline.
                        let y = (hf / 2.0 - self.digits.cy()).round();
                        text_at(&b.value, &self.name, b.text_x, y, &value_b);
                        if it.charging {
                            let bx = b.text_x + b.value_w + 2.0 * k;
                            text_at(&glyph::BOLT.to_string(), &self.bolt, bx, (hf / 2.0 - bolt_ink.cy()).round(), &brush(green, alpha)?);
                        }
                        continue;
                    }

                    // Line 1: name.
                    text_at(&b.name, &self.name, b.text_x, top, &fg_b);

                    // Line 2: gauge + value (+ bolt), centred on the digits' ink.
                    let mid = line2 + self.digits.cy();
                    let (gw, gh) = (dip::GAUGE_W * k, dip::GAUGE_H * k);
                    let gx = b.text_x + 0.5;
                    let gy = (mid - gh / 2.0).round() + 0.5;
                    let stroke = k.max(1.0);
                    let outline = brush(fg, 0.55 * alpha)?;
                    let body = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F { left: gx, top: gy, right: gx + gw, bottom: gy + gh },
                        radiusX: 2.0 * k,
                        radiusY: 2.0 * k,
                    };
                    rt.DrawRoundedRectangle(&body, &outline, stroke, None);
                    let nub = D2D_RECT_F { left: gx + gw + 0.5, top: gy + gh * 0.3, right: gx + gw + 1.5 * k + 0.5, bottom: gy + gh * 0.7 };
                    rt.FillRectangle(&nub, &outline);
                    let inner = 1.5 * k + stroke / 2.0;
                    let full = gw - 2.0 * inner;
                    let fill_w = if it.level == 0 { 0.0 } else { (full * it.level as f32 / 100.0).max(1.5 * k) };
                    if fill_w > 0.0 {
                        let fill = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F { left: gx + inner, top: gy + inner, right: gx + inner + fill_w, bottom: gy + gh - inner },
                            radiusX: 1.0 * k,
                            radiusY: 1.0 * k,
                        };
                        rt.FillRoundedRectangle(&fill, &brush(accent, alpha)?);
                    }
                    let vx = gx + gw + (1.5 + dip::GAUGE_GAP) * k;
                    text_at(&b.value, &self.value, vx, line2, &value_b);
                    if it.charging {
                        let bx = vx + b.value_w + 2.0 * k;
                        text_at(&glyph::BOLT.to_string(), &self.bolt, bx, (mid - bolt_ink.cy()).round(), &brush(green, alpha)?);
                    }
                }
                rt.EndDraw(None, None)?;

                // Debug aid: JUICEBAR_WIDGET_DUMP=<file> writes the raw BGRA frame.
                #[cfg(debug_assertions)]
                if let Some(path) = std::env::var_os("JUICEBAR_WIDGET_DUMP") {
                    let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                    let _ = std::fs::write(path, [&w.to_le_bytes()[..], &h.to_le_bytes()[..], px].concat());
                }

                let size = SIZE { cx: w, cy: h };
                let src = POINT { x: 0, y: 0 };
                let blend =
                    BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
                UpdateLayeredWindow(hwnd, Some(screen), None, Some(&size), Some(mem), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA)
            })();

            SelectObject(mem, old);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            result
        }
    }
}

fn has_font(dwrite: &IDWriteFactory, family: PCWSTR) -> bool {
    // SAFETY: read-only query of the system font collection.
    unsafe {
        let mut collection = None;
        if dwrite.GetSystemFontCollection(&mut collection, false).is_err() {
            return false;
        }
        let Some(collection) = collection else { return false };
        let (mut index, mut exists) = (0u32, windows::core::BOOL(0));
        collection.FindFamilyName(family, &mut index, &mut exists).is_ok() && exists.as_bool()
    }
}
