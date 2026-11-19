//! Floating transparent taskbar widget + modern Windows 11-style hover tooltip.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::{mem, ptr};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER,
    BLENDFUNCTION, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CALCRECT, DT_CENTER, DT_LEFT, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE, FW_NORMAL, GetDC,
    OUT_DEFAULT_PRECIS, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, FindWindowExW, FindWindowW, GW_HWNDPREV, GetWindow,
    GetWindowRect, HWND_TOPMOST, IDC_ARROW, LoadCursorW, RegisterClassW, SW_HIDE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, ShowWindow,
    ULW_ALPHA, UpdateLayeredWindow, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const WM_MOUSEHOVER: u32 = 0x02A1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WIDGET_WIDTH: i32 = 64;

static PERCENT_STATE: Mutex<Option<u8>> = Mutex::new(None);
static TOOLTIP_TEXT: Mutex<String> = Mutex::new(String::new());
static TOOLTIP_HWND: AtomicIsize = AtomicIsize::new(0);
static MENU_REQUESTED: AtomicBool = AtomicBool::new(false);
static HOVERED: AtomicBool = AtomicBool::new(false);

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Returns true if the taskbar window is currently above `hwnd` in Z-order.
fn is_behind_taskbar(hwnd: HWND, taskbar: HWND) -> bool {
    unsafe {
        let mut cur = GetWindow(hwnd, GW_HWNDPREV);
        while !cur.is_null() {
            if cur == taskbar {
                return true;
            }
            cur = GetWindow(cur, GW_HWNDPREV);
        }
        false
    }
}

/// Checks if the Windows taskbar is currently using Light Mode.
fn is_light_taskbar() -> bool {
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("SystemUsesLightTheme");
    let mut data: u32 = 0;
    let mut size = mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    status == 0 && data == 1
}

/// Loads the saved "Show on Taskbar" preference from the Registry.
pub fn is_mode_enabled() -> bool {
    let subkey = wide(r"Software\Logitech-G435");
    let value = wide("ShowOnTaskbar");
    let mut data: u32 = 0;
    let mut size = mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    status == 0 && data == 1
}

/// Saves the "Show on Taskbar" preference to the Registry.
pub fn set_mode_enabled(enabled: bool) {
    let subkey = wide(r"Software\Logitech-G435");
    let value = wide("ShowOnTaskbar");
    let data: u32 = if enabled { 1 } else { 0 };
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            REG_DWORD,
            &data as *const u32 as *const _,
            mem::size_of::<u32>() as u32,
        );
    }
}

/// Returns true if (x, y) is inside a rounded rectangle with corner radius `r`.
fn in_rounded_rect(x: i32, y: i32, left: i32, top: i32, right: i32, bottom: i32, r: i32) -> bool {
    if x < left || x >= right || y < top || y >= bottom {
        return false;
    }
    let dx = if x < left + r {
        (left + r) - x
    } else if x >= right - r {
        x - (right - r - 1)
    } else {
        0
    };
    let dy = if y < top + r {
        (top + r) - y
    } else if y >= bottom - r {
        y - (bottom - r - 1)
    } else {
        0
    };
    dx * dx + dy * dy <= r * r
}

fn hide_tooltip() {
    let tip = TOOLTIP_HWND.load(Ordering::SeqCst) as HWND;
    if !tip.is_null() {
        unsafe { ShowWindow(tip, SW_HIDE) };
    }
}

/// Draws and positions the modern rounded dark/light tooltip above the taskbar widget.
fn show_tooltip(widget_hwnd: HWND) {
    let tip_hwnd = TOOLTIP_HWND.load(Ordering::SeqCst) as HWND;
    if tip_hwnd.is_null() {
        return;
    }
    let text = TOOLTIP_TEXT
        .lock()
        .ok()
        .map(|s| s.clone())
        .unwrap_or_default();
    if text.is_empty() {
        return;
    }

    unsafe {
        let screen_dc = GetDC(ptr::null_mut());
        let mem_dc = CreateCompatibleDC(screen_dc);

        let font_name = wide("Segoe UI");
        let font = CreateFontW(
            -14,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            font_name.as_ptr(),
        );
        let old_font = SelectObject(mem_dc, font as _);

        // Measure text size
        let text_w = wide(&text);
        let mut calc_rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        DrawTextW(
            mem_dc,
            text_w.as_ptr(),
            -1,
            &mut calc_rect,
            DT_CALCRECT | DT_SINGLELINE,
        );

        let width = (calc_rect.right - calc_rect.left + 24).max(60);
        let height = (calc_rect.bottom - calc_rect.top + 14).max(30);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..mem::zeroed()
            },
            bmiColors: [mem::zeroed()],
        };

        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let hbm = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if hbm.is_null() || bits.is_null() {
            SelectObject(mem_dc, old_font);
            DeleteObject(font as _);
            DeleteDC(mem_dc);
            ReleaseDC(ptr::null_mut(), screen_dc);
            return;
        }

        let old_bm = SelectObject(mem_dc, hbm as _);
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
        pixels.fill(0);

        SetBkMode(mem_dc, TRANSPARENT as i32);
        SetTextColor(mem_dc, 0x00FF_FFFF);
        let mut draw_rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        DrawTextW(
            mem_dc,
            text_w.as_ptr(),
            -1,
            &mut draw_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        SelectObject(mem_dc, old_font);
        DeleteObject(font as _);

        let light = is_light_taskbar();
        // Modern Windows 11 dark / light tooltip palette
        let (fill_r, fill_g, fill_b, fill_a): (u32, u32, u32, u32) = if light {
            (249, 249, 249, 245)
        } else {
            (36, 40, 46, 245)
        };
        let (brd_r, brd_g, brd_b, brd_a): (u32, u32, u32, u32) = if light {
            (210, 210, 210, 255)
        } else {
            (62, 66, 74, 255)
        };
        let (tr, tg, tb): (u32, u32, u32) = if light {
            (24, 24, 24)
        } else {
            (255, 255, 255)
        };

        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) as usize;
                let in_outer = in_rounded_rect(x, y, 0, 0, width, height, 6);
                if !in_outer {
                    pixels[idx] = 0;
                    continue;
                }
                let in_inner = in_rounded_rect(x, y, 1, 1, width - 1, height - 1, 5);
                let (br, bg, bb, ba) = if in_inner {
                    (fill_r, fill_g, fill_b, fill_a)
                } else {
                    (brd_r, brd_g, brd_b, brd_a)
                };

                let cov = (pixels[idx] >> 16) & 0xFF;
                let bg_r = (br * ba) / 255;
                let bg_g = (bg * ba) / 255;
                let bg_b = (bb * ba) / 255;

                let fg_r = (tr * cov) / 255;
                let fg_g = (tg * cov) / 255;
                let fg_b = (tb * cov) / 255;
                let inv = 255 - cov;

                let out_r = fg_r + (bg_r * inv) / 255;
                let out_g = fg_g + (bg_g * inv) / 255;
                let out_b = fg_b + (bg_b * inv) / 255;
                let out_a = cov + (ba * inv) / 255;

                pixels[idx] = (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b;
            }
        }

        // Position centered directly above the taskbar widget
        let mut w_rect: RECT = mem::zeroed();
        GetWindowRect(widget_hwnd, &mut w_rect);
        let tip_x = w_rect.left + ((w_rect.right - w_rect.left) - width) / 2;
        let tip_y = w_rect.top - height - 8;

        let pt_src = POINT { x: 0, y: 0 };
        let pt_dst = POINT { x: tip_x, y: tip_y };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        UpdateLayeredWindow(
            tip_hwnd,
            screen_dc,
            &pt_dst,
            &size,
            mem_dc,
            &pt_src,
            0,
            &blend,
            ULW_ALPHA,
        );
        SetWindowPos(
            tip_hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );

        SelectObject(mem_dc, old_bm);
        DeleteObject(hbm as _);
        DeleteDC(mem_dc);
        ReleaseDC(ptr::null_mut(), screen_dc);
    }
}

/// Renders the widget with per-pixel alpha so the entire box catches clicks and hover events.
fn render_widget(hwnd: HWND) {
    unsafe {
        let mut rect: RECT = mem::zeroed();
        GetWindowRect(hwnd, &mut rect);
        let width = (rect.right - rect.left).max(1);
        let height = (rect.bottom - rect.top).max(1);

        let screen_dc = GetDC(ptr::null_mut());
        let mem_dc = CreateCompatibleDC(screen_dc);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..mem::zeroed()
            },
            bmiColors: [mem::zeroed()],
        };

        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let hbm = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if hbm.is_null() || bits.is_null() {
            DeleteDC(mem_dc);
            ReleaseDC(ptr::null_mut(), screen_dc);
            return;
        }

        let old_bm = SelectObject(mem_dc, hbm as _);
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
        pixels.fill(0);

        SetBkMode(mem_dc, TRANSPARENT as i32);
        SetTextColor(mem_dc, 0x00FF_FFFF);

        let icon_font_name = wide("Segoe Fluent Icons");
        let icon_font = CreateFontW(
            -16,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            icon_font_name.as_ptr(),
        );
        let old_font = SelectObject(mem_dc, icon_font as _);
        let mut icon_rect = RECT {
            left: 6,
            top: 0,
            right: width,
            bottom: height,
        };
        let glyph = wide("\u{E7F6}");
        DrawTextW(
            mem_dc,
            glyph.as_ptr(),
            -1,
            &mut icon_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        let text_font_name = wide("Segoe UI");
        let text_font = CreateFontW(
            -13,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            text_font_name.as_ptr(),
        );
        SelectObject(mem_dc, text_font as _);
        let percent = PERCENT_STATE.lock().ok().and_then(|s| *s);
        let label = match percent {
            Some(p) => format!("{p}%"),
            None => "--%".to_string(),
        };
        let label_w = wide(&label);
        let mut text_rect = RECT {
            left: 26,
            top: 0,
            right: width,
            bottom: height,
        };
        DrawTextW(
            mem_dc,
            label_w.as_ptr(),
            -1,
            &mut text_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        SelectObject(mem_dc, old_font);
        DeleteObject(icon_font as _);
        DeleteObject(text_font as _);

        let light = is_light_taskbar();
        let hovered = HOVERED.load(Ordering::SeqCst);

        let (tr, tg, tb): (u32, u32, u32) = match percent {
            Some(p) if p <= 15 => (255, 68, 68),
            _ if light => (0, 0, 0),
            _ => (255, 255, 255),
        };

        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) as usize;
                let cov = (pixels[idx] >> 16) & 0xFF;

                let in_pill = hovered && in_rounded_rect(x, y, 0, 4, width, height - 4, 5);
                let (bg_r, bg_g, bg_b, bg_a): (u32, u32, u32, u32) = if in_pill {
                    if light {
                        (0, 0, 0, 28)
                    } else {
                        (42, 42, 42, 42)
                    }
                } else {
                    (0, 0, 0, 1)
                };

                let fg_r = (tr * cov) / 255;
                let fg_g = (tg * cov) / 255;
                let fg_b = (tb * cov) / 255;
                let inv = 255 - cov;

                let out_r = fg_r + (bg_r * inv) / 255;
                let out_g = fg_g + (bg_g * inv) / 255;
                let out_b = fg_b + (bg_b * inv) / 255;
                let out_a = cov + (bg_a * inv) / 255;

                pixels[idx] = (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b;
            }
        }

        let pt_src = POINT { x: 0, y: 0 };
        let pt_dst = POINT {
            x: rect.left,
            y: rect.top,
        };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        UpdateLayeredWindow(
            hwnd,
            screen_dc,
            &pt_dst,
            &size,
            mem_dc,
            &pt_src,
            0,
            &blend,
            ULW_ALPHA,
        );

        SelectObject(mem_dc, old_bm);
        DeleteObject(hbm as _);
        DeleteDC(mem_dc);
        ReleaseDC(ptr::null_mut(), screen_dc);
    }
}

pub struct TaskbarWidget {
    hwnd: HWND,
    visible: bool,
    last_x: i32,
    last_y: i32,
    last_height: i32,
}

impl TaskbarWidget {
    pub fn new() -> Self {
        unsafe {
            let hinstance = GetModuleHandleW(ptr::null());

            let class_name = wide("G435TaskbarWidget");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance,
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                lpszClassName: class_name.as_ptr(),
                ..mem::zeroed()
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                ptr::null(),
                WS_POPUP,
                0,
                0,
                WIDGET_WIDTH,
                40,
                ptr::null_mut(),
                ptr::null_mut(),
                hinstance,
                ptr::null(),
            );

            // Create the custom modern tooltip window
            let tip_class = wide("G435TaskbarTooltip");
            let tip_wc = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: hinstance,
                lpszClassName: tip_class.as_ptr(),
                ..mem::zeroed()
            };
            RegisterClassW(&tip_wc);

            let tip_hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT,
                tip_class.as_ptr(),
                ptr::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                ptr::null_mut(),
                ptr::null_mut(),
                hinstance,
                ptr::null(),
            );
            TOOLTIP_HWND.store(tip_hwnd as isize, Ordering::SeqCst);

            Self {
                hwnd,
                visible: false,
                last_x: i32::MIN,
                last_y: i32::MIN,
                last_height: 0,
            }
        }
    }

    pub fn hwnd_isize(&self) -> isize {
        self.hwnd as isize
    }

    pub fn set_percent(&self, percent: Option<u8>) {
        if let Ok(mut state) = PERCENT_STATE.lock() {
            *state = percent;
        }
        if self.visible {
            render_widget(self.hwnd);
        }
    }

    pub fn set_tooltip(&self, text: &str) {
        if let Ok(mut state) = TOOLTIP_TEXT.lock() {
            *state = text.to_string();
        }
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if visible {
            self.last_x = i32::MIN;
            self.reposition();
            render_widget(self.hwnd);
        } else {
            HOVERED.store(false, Ordering::SeqCst);
            hide_tooltip();
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }

    /// Keeps the widget aligned immediately to the left of the system tray chevron.
    pub fn reposition(&mut self) {
        if !self.visible {
            return;
        }
        unsafe {
            let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), ptr::null());
            if taskbar.is_null() {
                return;
            }
            let tray = FindWindowExW(
                taskbar,
                ptr::null_mut(),
                wide("TrayNotifyWnd").as_ptr(),
                ptr::null(),
            );
            if tray.is_null() {
                return;
            }

            let mut tb_rect: RECT = mem::zeroed();
            let mut tray_rect: RECT = mem::zeroed();
            GetWindowRect(taskbar, &mut tb_rect);
            GetWindowRect(tray, &mut tray_rect);

            let height = tb_rect.bottom - tb_rect.top;
            let x = tray_rect.left - WIDGET_WIDTH + 2;
            let y = tb_rect.top;

            let geometry_changed =
                x != self.last_x || y != self.last_y || height != self.last_height;
            let covered = is_behind_taskbar(self.hwnd, taskbar);

            if geometry_changed || covered {
                let height_changed = height != self.last_height;
                self.last_x = x;
                self.last_y = y;
                self.last_height = height;

                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    x,
                    y,
                    WIDGET_WIDTH,
                    height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );

                if height_changed {
                    render_widget(self.hwnd);
                }
            }
        }
    }

    /// Returns true once whenever the user clicks the taskbar widget.
    pub fn take_menu_request(&self) -> bool {
        if MENU_REQUESTED.swap(false, Ordering::SeqCst) {
            hide_tooltip();
            unsafe { SetForegroundWindow(self.hwnd) };
            true
        } else {
            false
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            if !HOVERED.swap(true, Ordering::SeqCst) {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_HOVER,
                    hwndTrack: hwnd,
                    dwHoverTime: 400,
                };
                unsafe { TrackMouseEvent(&mut tme) };
                render_widget(hwnd);
            }
            0
        }
        WM_MOUSEHOVER => {
            if HOVERED.load(Ordering::SeqCst) {
                show_tooltip(hwnd);
            }
            0
        }
        WM_MOUSELEAVE => {
            if HOVERED.swap(false, Ordering::SeqCst) {
                hide_tooltip();
                render_widget(hwnd);
            }
            0
        }
        WM_RBUTTONUP | WM_LBUTTONUP => {
            hide_tooltip();
            MENU_REQUESTED.store(true, Ordering::SeqCst);
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Renders a 32x32 RGBA headphone glyph for the system tray icon.
pub fn render_headphone_rgba(size: i32, percent: Option<u8>) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    unsafe {
        let screen_dc = GetDC(ptr::null_mut());
        let mem_dc = CreateCompatibleDC(screen_dc);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..mem::zeroed()
            },
            bmiColors: [mem::zeroed()],
        };

        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let hbm = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if hbm.is_null() || bits.is_null() {
            DeleteDC(mem_dc);
            ReleaseDC(ptr::null_mut(), screen_dc);
            return rgba;
        }

        let old_bm = SelectObject(mem_dc, hbm as _);
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        pixels.fill(0);

        SetBkMode(mem_dc, TRANSPARENT as i32);
        SetTextColor(mem_dc, 0x00FF_FFFF);

        let font_name = wide("Segoe Fluent Icons");
        let font = CreateFontW(
            -(size - 4),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            font_name.as_ptr(),
        );
        let old_font = SelectObject(mem_dc, font as _);

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        let glyph = wide("\u{E7F6}");
        DrawTextW(
            mem_dc,
            glyph.as_ptr(),
            -1,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        let light = is_light_taskbar();
        let (r, g, b): (u8, u8, u8) = match percent {
            Some(p) if p <= 15 => (255, 68, 68),
            _ if light => (0, 0, 0),
            _ => (255, 255, 255),
        };

        for (i, &px) in pixels.iter().enumerate() {
            let alpha = ((px >> 16) & 0xFF) as u8;
            let o = i * 4;
            rgba[o] = r;
            rgba[o + 1] = g;
            rgba[o + 2] = b;
            rgba[o + 3] = alpha;
        }

        SelectObject(mem_dc, old_font);
        DeleteObject(font as _);
        SelectObject(mem_dc, old_bm);
        DeleteObject(hbm as _);
        DeleteDC(mem_dc);
        ReleaseDC(ptr::null_mut(), screen_dc);
    }
    rgba
}