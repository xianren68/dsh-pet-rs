//! 32 位 DIB 表面 + UpdateLayeredWindow 上屏。
//!
//! `UpdateLayeredWindow` 带 `AC_SRC_ALPHA` 时，Windows 会按位图的 alpha 通道
//! 做逐像素合成，**alpha = 0 的像素天然不吃鼠标**——这是整个方案里
//! 「点透明处穿透到桌面」不需要任何轮询/钩子的原因。

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows_sys::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows_sys::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

/// 一块与屏幕兼容的 32 位 top-down DIB。
pub struct Surface {
    pub w: i32,
    pub h: i32,
    hdc: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Option<Self> {
        unsafe {
            let screen = GetDC(null_mut());
            if screen.is_null() {
                return None;
            }
            let hdc = CreateCompatibleDC(screen);
            ReleaseDC(null_mut(), screen);
            if hdc.is_null() {
                return None;
            }

            let mut bmi: BITMAPINFO = zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 负高度 = top-down，行序与图像一致
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..zeroed()
            };

            let mut bits: *mut c_void = null_mut();
            let dib = CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if dib.is_null() || bits.is_null() {
                DeleteDC(hdc);
                return None;
            }
            let old = SelectObject(hdc, dib as HGDIOBJ);

            Some(Self {
                w,
                h,
                hdc,
                dib,
                old,
                bits: bits as *mut u32,
            })
        }
    }

    /// 供气泡等自绘模块使用的 GDI 画布。
    pub fn hdc(&self) -> HDC {
        self.hdc
    }

    /// 像素缓冲（0xAARRGGBB）。GDI 只写 RGB，alpha 需要调用方补。
    pub fn pixels(&mut self) -> &mut [u32] {
        let n = (self.w * self.h).max(0) as usize;
        unsafe { std::slice::from_raw_parts_mut(self.bits, n) }
    }

    /// 把一整帧 BGRA（`0xAARRGGBB`，小端）拷进 DIB。
    ///
    /// **做 alpha 预乘**。`UpdateLayeredWindow` 配 `AC_SRC_ALPHA` 要求位图是预乘的，
    /// 而 WebP 解出来的是非预乘——直接把非预乘数据贴上去，半透明边缘的 RGB 没乘
    /// alpha 却被当成已乘，边缘会偏亮发白（深色桌面上就是一圈白斑/白块）。
    pub fn upload(&mut self, frame: &[u32]) {
        let n = (self.w * self.h).max(0) as usize;
        let n = n.min(frame.len());
        unsafe {
            for i in 0..n {
                let p = frame[i];
                let a = p >> 24;
                *self.bits.add(i) = if a == 255 {
                    p
                } else if a == 0 {
                    0
                } else {
                    let r = ((p >> 16) & 0xFF) * a / 255;
                    let g = ((p >> 8) & 0xFF) * a / 255;
                    let b = (p & 0xFF) * a / 255;
                    (a << 24) | (r << 16) | (g << 8) | b
                };
            }
        }
    }

    /// 水平镜像后写入 DIB（朝右时用；素材原生朝左）。同样做预乘。
    ///
    /// 只动像素不做预翻转副本：640×360 一行 2.5KB，整帧不到 1ms。
    pub fn upload_flipped(&mut self, frame: &[u32]) {
        let w = self.w as usize;
        let h = self.h as usize;
        if w == 0 || h == 0 {
            return;
        }
        let total = w * h;
        let n = total.min(frame.len());
        unsafe {
            for y in 0..h {
                let row = y * w;
                if row >= n {
                    break;
                }
                let end = (row + w).min(n);
                let len = end - row;
                let dst = self.bits.add(row);
                for x in 0..len {
                    let p = frame[row + len - 1 - x];
                    let a = p >> 24;
                    *dst.add(x) = if a == 255 {
                        p
                    } else if a == 0 {
                        0
                    } else {
                        let r = ((p >> 16) & 0xFF) * a / 255;
                        let g = ((p >> 8) & 0xFF) * a / 255;
                        let b = (p & 0xFF) * a / 255;
                        (a << 24) | (r << 16) | (g << 8) | b
                    };
                }
            }
        }
    }

    /// 把 DIB 贴到屏幕，同时把窗口挪到 `(x, y)`。
    ///
    /// 位置走这里而不是 `SetWindowPos`：一次调用同时完成「移动 + 换帧」，
    /// 不会出现窗口先动、画面后到的撕裂。
    pub fn present(&self, hwnd: HWND, x: i32, y: i32, alpha: u8) {
        unsafe {
            let dst = windows_sys::Win32::Foundation::POINT { x, y };
            let size = windows_sys::Win32::Foundation::SIZE {
                cx: self.w,
                cy: self.h,
            };
            let src = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                null_mut(),
                &dst,
                &size,
                self.hdc,
                &src,
                0,
                &blend,
                ULW_ALPHA,
            );
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.old);
            DeleteObject(self.dib as HGDIOBJ);
            DeleteDC(self.hdc);
        }
    }
}

// 单线程使用，裸指针本身不跨线程。
unsafe impl Send for Surface {}
