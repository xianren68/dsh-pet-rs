//! 头顶气泡：独立的分层点击穿透窗口 + GDI 文字。
//!
//! 气泡窗自己也是一个 `WS_EX_LAYERED` 窗口，但它整窗 `WS_EX_TRANSPARENT`，
//! 永远不吃鼠标。文字用 GDI 画进 32 位 DIB，再把非透明像素的 alpha 补上。

use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, GetDC,
    GetDeviceCaps, GetStockObject, Rectangle, ReleaseDC, RoundRect, SelectObject, SetBkMode,
    SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    DT_CALCRECT, DT_CENTER, DT_NOPREFIX, DT_WORDBREAK, FF_DONTCARE, HGDIOBJ, LOGPIXELSY, NULL_PEN,
    OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IsWindowVisible, LoadCursorW, RegisterClassExW,
    ShowWindow, IDC_ARROW, SW_HIDE, SW_SHOWNOACTIVATE, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::event::BubbleStyle;
use crate::surface::Surface;
use crate::util::{to_wide, work_area_at};

const CLASS_NAME: &str = "DshPetBubble";
const DEFAULT_TEXT_COLOR: u32 = 0x00FF_FFFF;
const DEFAULT_BG_COLOR: u32 = 0x002B2B2B;

/// 把 JSON 里的逻辑尺寸换算成当前 DPI 下的像素。
struct RenderStyle {
    font_family: String,
    font_px: i32,
    font_weight: i32,
    text_color: u32,
    background_color: u32,
    alpha: u32,
    radius: i32,
    pad: i32,
    max_w: i32,
}

impl RenderStyle {
    fn new(style: &BubbleStyle, dpi: u32) -> Self {
        let dpi = dpi.max(96) as i64;
        let scale = |v: i32| ((v.max(0) as i64 * dpi + 48) / 96) as i32;

        let pad = scale(style.padding).clamp(0, 64);
        let font_px = ((style.font_size.max(8) as i64 * dpi + 48) / 96).clamp(8, 96) as i32;
        let max_w = scale(style.max_width.max(96))
            .clamp(96, 1_600)
            .max(pad * 2 + font_px * 4);

        let font_family = style.font_family.trim();
        let font_family = if font_family.is_empty() {
            "Microsoft YaHei UI".to_string()
        } else {
            font_family.to_string()
        };

        Self {
            font_family,
            font_px,
            font_weight: style.font_weight.clamp(100, 900) as i32,
            text_color: parse_color(&style.text_color).unwrap_or(DEFAULT_TEXT_COLOR),
            background_color: parse_color(&style.background_color).unwrap_or(DEFAULT_BG_COLOR),
            alpha: style.alpha.min(255),
            radius: scale(style.corner_radius).clamp(0, 64),
            pad,
            max_w,
        }
    }
}

struct Bubble {
    hwnd: HWND,
    surface: Surface,
}

static mut BUBBLE: Option<Bubble> = None;

fn slot() -> &'static mut Option<Bubble> {
    unsafe { &mut *(&raw mut BUBBLE) }
}

pub fn show(text: &str, pet_x: i32, pet_y: i32, pet_w: i32, pet_h: i32, style: &BubbleStyle) {
    let text = text.trim();
    if text.is_empty() {
        hide();
        return;
    }
    let style = RenderStyle::new(style, screen_dpi());
    let (w, h) = metrics(text, &style);

    let s = slot();
    if s.is_none() {
        *s = create();
    }
    let Some(b) = s.as_mut() else {
        return;
    };
    if b.surface.w != w || b.surface.h != h {
        let Some(surface) = Surface::new(w, h) else {
            return;
        };
        b.surface = surface;
    }
    draw(&mut b.surface, text, &style);
    let (x, y) = place(pet_x, pet_y, pet_w, pet_h, w, h);
    b.surface.present(b.hwnd, x, y, 255);
    unsafe {
        ShowWindow(b.hwnd, SW_SHOWNOACTIVATE);
    }
}

/// 宠物移动时让气泡跟着走。
pub fn follow(pet_x: i32, pet_y: i32, pet_w: i32, pet_h: i32) {
    let Some(b) = slot().as_mut() else {
        return;
    };
    if unsafe { IsWindowVisible(b.hwnd) } == 0 {
        return;
    }
    let (x, y) = place(pet_x, pet_y, pet_w, pet_h, b.surface.w, b.surface.h);
    b.surface.present(b.hwnd, x, y, 255);
}

pub fn hide() {
    if let Some(b) = slot().as_mut() {
        unsafe {
            ShowWindow(b.hwnd, SW_HIDE);
        }
    }
}

pub fn destroy() {
    if let Some(b) = slot().take() {
        unsafe {
            DestroyWindow(b.hwnd);
        }
    }
}

fn create() -> Option<Bubble> {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let cls = to_wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: cls.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
            cls.as_ptr(),
            to_wide("").as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            hinst,
            null_mut(),
        );
        if hwnd.is_null() {
            return None;
        }
        let surface = Surface::new(1, 1)?;
        Some(Bubble { hwnd, surface })
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn screen_dpi() -> u32 {
    unsafe {
        let dc = GetDC(null_mut());
        if dc.is_null() {
            return 96;
        }
        let dpi = GetDeviceCaps(dc, LOGPIXELSY as i32);
        ReleaseDC(null_mut(), dc);
        if dpi <= 0 {
            96
        } else {
            dpi as u32
        }
    }
}

fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(s, 16).ok()?;
    Some(((rgb & 0xFF) << 16) | (rgb & 0x00FF00) | ((rgb >> 16) & 0xFF))
}

fn make_font(px: i32, family: &str, weight: i32) -> HGDIOBJ {
    let family = to_wide(family);
    unsafe {
        CreateFontW(
            -px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            family.as_ptr(),
        ) as HGDIOBJ
    }
}

fn metrics(text: &str, style: &RenderStyle) -> (i32, i32) {
    let chars = text.chars().count().min(10_000).max(1) as i32;
    let min_w = (style.pad * 2 + style.font_px * 4).max(96);
    let w = (chars * style.font_px + style.pad * 2).clamp(min_w, style.max_w);
    let h = measure_height(text, w, style) + style.pad * 2;
    (w, h.max(style.font_px + style.pad * 2))
}

fn measure_height(text: &str, w: i32, style: &RenderStyle) -> i32 {
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return style.font_px * 2;
        }
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(null_mut(), screen);
        if dc.is_null() {
            return style.font_px * 2;
        }
        let font = make_font(style.font_px, &style.font_family, style.font_weight);
        let old = SelectObject(dc, font);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (w - style.pad * 2).max(20),
            bottom: 0,
        };
        let wide = to_wide(text);
        DrawTextW(
            dc,
            wide.as_ptr(),
            -1,
            &mut rect,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        DeleteObject(font);
        DeleteDC(dc);
        (rect.bottom - rect.top).max(style.font_px)
    }
}

fn draw(surface: &mut Surface, text: &str, style: &RenderStyle) {
    surface.pixels().fill(0);
    let hdc = surface.hdc();
    let radius = style.radius.min(surface.w / 2).min(surface.h / 2).max(0);
    unsafe {
        let font = make_font(style.font_px, &style.font_family, style.font_weight);
        let old_font = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, style.text_color);

        let brush = CreateSolidBrush(style.background_color);
        let old_brush = SelectObject(hdc, brush as HGDIOBJ);
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        if radius > 0 {
            RoundRect(hdc, 0, 0, surface.w, surface.h, radius * 2, radius * 2);
        } else {
            Rectangle(hdc, 0, 0, surface.w, surface.h);
        }
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);

        let mut rect = RECT {
            left: style.pad,
            top: style.pad,
            right: surface.w - style.pad,
            bottom: surface.h - style.pad,
        };
        let wide = to_wide(text);
        DrawTextW(
            hdc,
            wide.as_ptr(),
            -1,
            &mut rect,
            DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
        );

        SelectObject(hdc, old_font);
        DeleteObject(brush as HGDIOBJ);
        DeleteObject(font);
    }

    apply_alpha(surface, style.alpha, radius);
}

/// GDI 只写 RGB。按圆角矩形的几何形状补 alpha 并预乘，避免背景色是纯黑时
/// 被误判成“没画过的像素”。圆角外的像素保持完全透明。
fn apply_alpha(surface: &mut Surface, alpha: u32, radius: i32) {
    let alpha = alpha.min(255);
    let w = surface.w;
    let h = surface.h;
    let radius = radius.min(w / 2).min(h / 2).max(0);
    let pixels = surface.pixels();
    for y in 0..h {
        for x in 0..w {
            let p = &mut pixels[(y * w + x) as usize];
            if alpha == 0 || !inside_round_rect(x, y, w, h, radius) {
                *p = 0;
                continue;
            }
            let b = (*p & 0xFF) * alpha / 255;
            let g = ((*p >> 8) & 0xFF) * alpha / 255;
            let r = ((*p >> 16) & 0xFF) * alpha / 255;
            *p = (alpha << 24) | (r << 16) | (g << 8) | b;
        }
    }
}

fn inside_round_rect(x: i32, y: i32, w: i32, h: i32, radius: i32) -> bool {
    if radius <= 0 {
        return true;
    }
    let cx = x.min(w - 1 - x);
    let cy = y.min(h - 1 - y);
    if cx >= radius || cy >= radius {
        return true;
    }
    let dx = radius - cx;
    let dy = radius - cy;
    dx * dx + dy * dy <= radius * radius
}

fn place(pet_x: i32, pet_y: i32, pet_w: i32, pet_h: i32, w: i32, h: i32) -> (i32, i32) {
    let wa = work_area_at(pet_x + pet_w / 2, pet_y + pet_h / 2);
    let mut x = pet_x + pet_w / 2 - w / 2;
    let mut y = pet_y - h - 8;
    if y < wa.top {
        y = pet_y + pet_h + 8;
    }
    x = x.clamp(wa.left, (wa.right - w).max(wa.left));
    y = y.clamp(wa.top, (wa.bottom - h).max(wa.top));
    (x, y)
}
