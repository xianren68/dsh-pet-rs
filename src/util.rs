//! Win32 小工具：宽字符串、光标、工作区、随机数。

use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, SystemParametersInfoW, SPI_GETWORKAREA,
};

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 光标屏幕坐标。
pub fn cursor() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// 主屏工作区（已排除任务栏）。
pub fn work_area() -> RECT {
    let mut r: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut _ as *mut _, 0);
    }
    if r.right <= r.left || r.bottom <= r.top {
        r = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
    }
    r
}

/// 够用的 xorshift，避免为挑动画引一个 rand 依赖。
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Self {
        let seed = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
        Self((seed as u64).wrapping_mul(6364136223846793005).max(1))
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
}
