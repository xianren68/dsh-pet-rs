//! 系统托盘：显示/隐藏、重载素材、退出。

use std::mem::size_of;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{HICON,
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, PostMessageW,
    SetForegroundWindow, TrackPopupMenuEx, IDI_APPLICATION, MF_POPUP, MF_SEPARATOR, MF_STRING, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
};

use crate::util::to_wide;

pub const WM_TRAY: u32 = 0x8000 + 1; // WM_APP + 1

pub const CMD_SHOW: u32 = 10000;
pub const CMD_HOME: u32 = 10001;
pub const CMD_RELOAD: u32 = 10002;
pub const CMD_EXIT: u32 = 10003;
pub const CMD_SETTINGS: u32 = 10004;
pub const CMD_SALARY: u32 = 10005;

pub struct Tray {
    data: NOTIFYICONDATAW,
    alive: bool,
}

impl Tray {
    pub fn new(hwnd: HWND, hicon: Option<HICON>) -> Option<Self> {
        unsafe {
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = hwnd;
            data.uID = 1;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = WM_TRAY;
            data.hIcon = match hicon {
                Some(h) => h,
                None => LoadIconW(null_mut(), IDI_APPLICATION),
            };

            let tip = to_wide("dsh-pet-rs");
            let n = tip.len().min(data.szTip.len());
            data.szTip[..n].copy_from_slice(&tip[..n]);

            if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                return None;
            }
            Some(Self { data, alive: true })
        }
    }

    pub fn destroy(&mut self) {
        if !self.alive {
            return;
        }
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.data);
        }
        self.alive = false;
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// 弹出托盘菜单，返回被选中的命令 id（0 = 没选）。
pub fn popup_tray(hwnd: HWND) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        let items: [(&str, u32); 5] = [
            ("设置…", CMD_SETTINGS),
            ("显示 / 隐藏", CMD_SHOW),
            ("回到角落", CMD_HOME),
            ("重载素材", CMD_RELOAD),
            ("退出", CMD_EXIT),
        ];
        for (label, id) in items {
            let w = to_wide(label);
            AppendMenuW(menu, MF_STRING, id as usize, w.as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

        let pt = cursor();
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            pt.0,
            pt.1,
            hwnd,
            null_mut(),
        );
        DestroyMenu(menu);
        eprintln!("[menu] at {pt:?} -> cmd={cmd}");
        PostMessageW(hwnd, WM_NULL, 0, 0);
        cmd as u32
    }
}

/// 悬浮菜单：分类子菜单 + 固定项。返回值的 1..=N 对应 `cats` 按序平铺后的下标+1。
///
/// `salary` 为真时在固定项上方插一条「数工资」—— 开关没开就不显示，
/// 免得让人点了一个什么都不做的菜单项。
pub fn popup_pet(hwnd: HWND, cats: &[(String, Vec<String>)], salary: bool) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        let mut flat = 0u32;
        for (cat, items) in cats {
            if items.is_empty() {
                continue;
            }
            let sub = CreatePopupMenu();
            for it in items {
                let w = to_wide(it);
                AppendMenuW(sub, MF_STRING, (flat + 1) as usize, w.as_ptr());
                flat += 1;
            }
            let w = to_wide(&format!("{cat}（{}）", items.len()));
            AppendMenuW(menu, MF_POPUP, sub as usize, w.as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        if salary {
            let w = to_wide("数工资");
            AppendMenuW(menu, MF_STRING, CMD_SALARY as usize, w.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        }
        for (label, id) in [
            ("设置…", CMD_SETTINGS),
            ("回到角落", CMD_HOME),
            ("重载素材", CMD_RELOAD),
            ("退出", CMD_EXIT),
        ] {
            let w = to_wide(label);
            AppendMenuW(menu, MF_STRING, id as usize, w.as_ptr());
        }

        let pt = cursor();
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            pt.0,
            pt.1,
            hwnd,
            null_mut(),
        );
        DestroyMenu(menu);
        eprintln!("[menu] at {pt:?} -> cmd={cmd}");
        PostMessageW(hwnd, WM_NULL, 0, 0);
        cmd as u32
    }
}

fn cursor() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut p);
    }
    (p.x, p.y)
}
