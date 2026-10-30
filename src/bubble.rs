//! 头顶气泡：独立的分层点击穿透窗口 + GDI 文字。
//!
//! 气泡窗自己也是一个 `WS_EX_LAYERED` 窗口，但它整窗 `WS_EX_TRANSPARENT`，
//! 永远不吃鼠标。文字用 GDI 画进 32 位 DIB，再把非透明像素的 alpha 补上。
//!
//! **形状**：圆角主体 + 一条指向角色的三角尾巴，整体在 4 倍画布上画完再缩回——
//! GDI 的 `RoundRect` / `Polygon` 不做抗锯齿，直接 1× 画出来边缘是阶梯状的。
//! 超采样后走 `scale::scale` 的双线性缩，边缘才有柔和过渡。

use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, GetDC,
    GetDeviceCaps, GetStockObject, Polygon, Rectangle, ReleaseDC, RoundRect, SelectObject,
    SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_PITCH, DT_CALCRECT, DT_CENTER, DT_NOPREFIX, DT_WORDBREAK, FF_DONTCARE, HGDIOBJ,
    LOGPIXELSY, NULL_PEN, OUT_DEFAULT_PRECIS, TRANSPARENT,
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
const DEFAULT_TEXT_COLOR: u32 = 0x0057_4A_3D;
const DEFAULT_BG_COLOR: u32 = 0x00FF_FFFF;
const DEFAULT_BORDER_COLOR: u32 = 0x00C0_B8_A8;

/// 超采样倍数。4 倍是体积/画质的折中：再高收益递减，内存翻倍。
const SS: i32 = 4;
/// 尾巴底边宽、高度（逻辑像素，96 DPI 基准）
const TAIL_W: i32 = 24;
const TAIL_H: i32 = 13;

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
    /// 尾巴高度（已按 DPI 缩放），0 表示不带尾巴
    tail_h: i32,
    border_color: u32,
    /// 描边宽度（已按 DPI 缩放），0 = 不描边
    border_w: i32,
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
            "YouYuan".to_string()
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
            tail_h: scale(TAIL_H).clamp(0, 48),
            border_color: parse_color(&style.border_color).unwrap_or(DEFAULT_BORDER_COLOR),
            border_w: scale(style.border_width).clamp(0, 12),
        }
    }
}

struct Bubble {
    hwnd: HWND,
    surface: Surface,
}

static mut BUBBLE: Option<Bubble> = None;

fn slot() -> &'static mut Option<Bubble> {
    // 本分支要能过 Rust 1.77：`&raw mut` 是 1.82 的语法，得用 addr_of_mut!
    unsafe { &mut *std::ptr::addr_of_mut!(BUBBLE) }
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
    let (x, y, tail_top) = place(pet_x, pet_y, pet_w, pet_h, w, h);
    draw(&mut b.surface, text, &style, tail_top);
    // 调试：设了环境变量就把这一帧存到临时目录，用来眼睛看渲染结果
    if std::env::var_os("DSH_DUMP_BUBBLE").is_some() {
        let p = std::env::temp_dir().join("bubble_dump.png");
        dump_surface(&mut b.surface, &p);
        eprintln!("[bubble] dump -> {}", p.display());
    }
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
    let (x, y, _) = place(pet_x, pet_y, pet_w, pet_h, b.surface.w, b.surface.h);
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

/// 把当前表面存成 PNG，调试用。像素是 0xAARRGGBB，直接拆成 RGBA。
fn dump_surface(surface: &mut Surface, path: &std::path::Path) {
    let (w, h) = (surface.w.max(0) as u32, surface.h.max(0) as u32);
    if w == 0 || h == 0 {
        return;
    }
    let px = surface.pixels();
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = px[(y * w + x) as usize];
            img.put_pixel(
                x,
                y,
                image::Rgba([(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]),
            );
        }
    }
    let _ = img.save(path);
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
    // 多行文案（数工资那种两行）要按最长的一行算宽度，否则 \n 会被当成字符撑宽
    let chars = text
        .lines()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(1)
        .clamp(1, 10_000) as i32;
    let min_w = (style.pad * 2 + style.font_px * 4).max(96);
    let w = (chars * style.font_px + style.pad * 2).clamp(min_w, style.max_w);
    let h = measure_height(text, w, style) + style.pad * 2 + style.tail_h;
    (w, h.max(style.font_px + style.pad * 2 + style.tail_h))
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

/// 尾巴在 `tail_h` 处的半宽。按 TAIL_W:TAIL_H 的比例从高度推出来，
/// 这样超采样倍数变了也不用跟着改。
fn tail_half(tail_h: i32) -> i32 {
    (tail_h as f32 * TAIL_W as f32 / TAIL_H as f32 / 2.0).round() as i32
}

/// 画一帧气泡。
///
/// 全程在 `SS` 倍画布上做，最后缩回 1×。GDI 的 RoundRect / Polygon 不带抗锯齿，
/// 1× 直接画出来圆角和尾巴斜边都是阶梯状的。
fn draw(surface: &mut Surface, text: &str, style: &RenderStyle, tail_on_top: bool) {
    let w = surface.w;
    let h = surface.h;
    if w <= 0 || h <= 0 {
        return;
    }
    let tail = style.tail_h.min(h / 2);
    // 主体在逻辑坐标里的上下边界（尾巴占掉的那一条不算）
    let (body_top, body_bottom) = if tail_on_top {
        (tail, h)
    } else {
        (0, h - tail)
    };

    let hw = w * SS;
    let hh = h * SS;
    let Some(mut hi) = Surface::new(hw, hh) else {
        return;
    };
    hi.pixels().fill(0);
    let hdc = hi.hdc();

    unsafe {
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        let (bt, bb) = (body_top * SS, body_bottom * SS);
        let tail_hi = tail * SS;
        let radius = (style.radius * SS).min(hw / 2).min(hh / 2).max(0);
        let border_w = style.border_w * SS;

        // 两层填充：外层描边色打满，内层背景色内缩覆盖，露出来的一圈就是描边。
        // 浅色底在浅色桌面上没有描边等于隐形，GDI 又画不了阴影，这是最稳的办法。
        if border_w > 0 {
            let outer = CreateSolidBrush(style.border_color);
            let old_b = SelectObject(hdc, outer as HGDIOBJ);
            fill_shape(hdc, hw, hh, bt, bb, radius, tail_hi, tail_on_top, 0);
            SelectObject(hdc, old_b);
            DeleteObject(outer as HGDIOBJ);
        }

        let brush = CreateSolidBrush(style.background_color);
        let old_brush = SelectObject(hdc, brush as HGDIOBJ);
        fill_shape(hdc, hw, hh, bt, bb, radius, tail_hi, tail_on_top, border_w);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(brush as HGDIOBJ);

        // 文字：字号跟着 ×SS，垂直居中在主体内（尾巴不参与）
        let font = make_font(style.font_px * SS, &style.font_family, style.font_weight);
        let old_font = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, style.text_color);

        let pad = style.pad * SS;
        let wide = to_wide(text);
        let mut probe = RECT {
            left: 0,
            top: 0,
            right: (hw - pad * 2).max(1),
            bottom: 0,
        };
        DrawTextW(
            hdc,
            wide.as_ptr(),
            -1,
            &mut probe,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        let text_h = (probe.bottom - probe.top).max(1);
        let body_h = bb - bt;
        let mut rect = RECT {
            left: pad,
            top: bt + (body_h - text_h) / 2,
            right: hw - pad,
            bottom: bb,
        };
        DrawTextW(
            hdc,
            wide.as_ptr(),
            -1,
            &mut rect,
            DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
        );

        SelectObject(hdc, old_font);
        DeleteObject(font);
    }

    // 补 alpha（GDI 只写 RGB），然后缩回 1×
    apply_alpha(&mut hi, style.alpha, style.radius * SS, tail * SS, tail_on_top);
    let small = crate::scale::scale(
        hi.pixels(),
        hw as usize,
        hh as usize,
        w as usize,
        h as usize,
    );
    surface.upload(&small);
}

/// 在超采样画布上填一块「圆角主体 + 尾巴」，整体向内缩 `inset` 像素。
///
/// 画两遍就得到描边：先 `inset = 0` 用描边色打满，再 `inset = border_w` 用背景色覆盖。
unsafe fn fill_shape(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    w: i32,
    h: i32,
    bt: i32,
    bb: i32,
    radius: i32,
    tail_hi: i32,
    tail_on_top: bool,
    inset: i32,
) {
    let (bt, bb) = (bt + inset, bb - inset);
    if bb <= bt || w - inset * 2 <= 0 {
        return;
    }
    let r = (radius - inset).max(0).min((bb - bt) / 2);
    if r > 0 {
        RoundRect(hdc, inset, bt, w - inset, bb, r * 2, r * 2);
    } else {
        Rectangle(hdc, inset, bt, w - inset, bb);
    }

    let th = tail_hi - inset;
    if th > 0 {
        let half = (tail_half(tail_hi) - inset).max(1).min(w / 2);
        let cx = w / 2;
        let mut pts = if tail_on_top {
            [
                POINT { x: cx - half, y: bt },
                POINT { x: cx + half, y: bt },
                POINT { x: cx, y: inset },
            ]
        } else {
            [
                POINT { x: cx - half, y: bb },
                POINT { x: cx + half, y: bb },
                POINT { x: cx, y: h - inset },
            ]
        };
        Polygon(hdc, pts.as_mut_ptr(), 3);
    }
}

/// GDI 只写 RGB。按「圆角主体 + 三角尾巴」的几何形状补 alpha 并预乘，
/// 形状外的像素保持完全透明。在超采样画布上做，缩小时边缘自然过渡。
fn apply_alpha(surface: &mut Surface, alpha: u32, radius: i32, tail_h: i32, tail_on_top: bool) {
    let alpha = alpha.min(255);
    let w = surface.w;
    let h = surface.h;
    let radius = radius.min(w / 2).min(h / 2).max(0);
    let tail_h = tail_h.min(h / 2).max(0);
    let (bt, bb) = if tail_on_top {
        (tail_h, h)
    } else {
        (0, h - tail_h)
    };
    let half = tail_half(tail_h);
    let cx = w / 2;

    let pixels = surface.pixels();
    for y in 0..h {
        let in_body = y >= bt && y < bb;
        for x in 0..w {
            let p = &mut pixels[(y * w + x) as usize];
            let solid = if in_body {
                inside_round_rect(x, y - bt, w, bb - bt, radius)
            } else if tail_h > 0 {
                // 尾巴区间：按离底边的距离线性收窄
                let t = if tail_on_top {
                    y as f32 / tail_h as f32
                } else {
                    (h - y) as f32 / tail_h as f32
                };
                let t = t.clamp(0.0, 1.0);
                let half_w = half as f32 * t;
                ((x - cx) as f32).abs() <= half_w
            } else {
                false
            };
            if alpha == 0 || !solid {
                *p = 0;
                continue;
            }
            // 只写 alpha，**不预乘** —— 预乘统一由 Surface::upload 做。
            // 这里如果乘了，scale 会再乘一次、upload 再乘第三次，气泡会一路发暗。
            *p = (alpha << 24) | (*p & 0x00FF_FFFF);
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

/// 气泡位置：尾巴尖贴着角色头顶（素材里角色头顶在画面 15.5% 高度处）。
/// 上方放不下就翻到脚下，此时尾巴朝上，尖仍指向角色。
fn place(pet_x: i32, pet_y: i32, pet_w: i32, pet_h: i32, w: i32, h: i32) -> (i32, i32, bool) {
    let wa = work_area_at(pet_x + pet_w / 2, pet_y + pet_h / 2);
    let mut x = pet_x + pet_w / 2 - w / 2;
    let head_top = pet_y + (pet_h as f32 * 0.155) as i32;
    let foot = pet_y + (pet_h as f32 * 0.92) as i32;

    let (mut y, tail_on_top) = if head_top - h - 2 >= wa.top {
        (head_top - h - 2, false)
    } else {
        (foot + 2, true)
    };
    x = x.clamp(wa.left, (wa.right - w).max(wa.left));
    y = y.clamp(wa.top, (wa.bottom - h).max(wa.top));
    (x, y, tail_on_top)
}
