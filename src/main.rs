//! dsh-pet-rs —— 极简原生桌宠。
//!
//! 一个 WS_EX_LAYERED 弹出窗 + 32 位 alpha 位图，靠 `UpdateLayeredWindow`
//! 每帧上屏。透明像素由系统合成器直接放过，不需要轮询光标做穿透。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autorun;
mod behavior;
mod clip;
mod icon;
mod scale;
mod settings;
mod surface;
mod tray;
mod util;

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse as km;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, LoadCursorW,
    PostQuitMessage, RegisterClassExW, SetProcessDPIAware, SetTimer, ShowWindow, TranslateMessage,
    IDC_HAND, MA_NOACTIVATE, MSG, SW_SHOWNOACTIVATE, WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use app::App;
use tray::Tray;
use util::to_wide;

const TIMER_FRAME: usize = 1;
const FRAME_MS: u32 = 16;

thread_local! {
    static APP: RefCell<Option<App>> = RefCell::new(None);
}

/// 生成好的托盘/面板图标，设置面板创建窗口时要用。
static mut APP_ICON: windows_sys::Win32::UI::WindowsAndMessaging::HICON = std::ptr::null_mut();

pub(crate) fn app_icon() -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON> {
    let h = unsafe { APP_ICON };
    if h.is_null() { None } else { Some(h) }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[panic] {info}");
    }));
    unsafe {
        // Per-Monitor V2：拿物理像素坐标，跨屏拖动能自适应缩放
        let dpi_ok = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        eprintln!("[dpi] per-monitor-v2 -> {dpi_ok}");
        if dpi_ok == 0 {
            let old = SetProcessDPIAware();
            eprintln!("[dpi] system-aware -> {old}");
        }

        let hinstance = GetModuleHandleW(null());
        let class_name = to_wide("DshPetRsWindow");

        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_HAND),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: null_mut(),
        };
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            eprintln!(
                "[启动] RegisterClassExW 失败, GetLastError={}",
                windows_sys::Win32::Foundation::GetLastError()
            );
        }

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class_name.as_ptr(),
            to_wide("dsh-pet-rs").as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            hinstance,
            null_mut(),
        );
        if hwnd.is_null() {
            eprintln!(
                "[启动] CreateWindowExW 失败, GetLastError={}",
                windows_sys::Win32::Foundation::GetLastError()
            );
            return;
        }

        let Some(assets) = asset_dir() else {
            let msg = "找不到素材目录。\n\n把打包里的 assets 文件夹放在 exe 同级，\n\
                       或放到 %APPDATA%\\dsh-pet-rs\\assets 下。";
            eprintln!("[启动] {msg}");
            show_error(msg);
            return;
        };
        let cfg = config_dir();
        match App::new(hwnd, assets.clone(), cfg.clone()) {
            Ok(pet) => APP.with(|c| *c.borrow_mut() = Some(pet)),
            Err(e) => {
                eprintln!("[启动] {e}");
                show_error(&e);
                return;
            }
        }

        // 后台预热时长表缓存：首次播放某段动画就不会卡在整段解码上
        if let Some(list) = with_app(|app| app.anim_list()) {
            std::thread::spawn(move || {
                for (name, path) in list {
                    let _ = clip::Clip::probe(&path, &name);
                }
            });
        }

        // 托盘图标从待机动画第一帧生成，没有就退回系统默认
        let hicon = icon::ensure_icon(&assets, &cfg)
            .as_ref()
            .and_then(|p| icon::load_hicon(p));
        if let Some(h) = hicon {
            APP_ICON = h;
        }
        let mut tray = Tray::new(hwnd, hicon);

        // 先显示再起定时器：paint 已在 App::new 里做过一次，首帧就是完整画面
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        // --play "<动作名>"：启动即播某段，调试用
        if let Some(i) = std::env::args().position(|a| a == "--play") {
            if let Some(name) = std::env::args().nth(i + 1) {
                with_app(|app| app.play_name(&name));
            }
        }

        // 带 --settings 启动时直接开设置面板（给快捷方式用）
        if std::env::args().any(|a| a == "--settings") {
            settings::show();
        }
        SetTimer(hwnd, TIMER_FRAME, FRAME_MS, None);

        let mut msg: MSG = zeroed();
        loop {
            let r = GetMessageW(&mut msg, null_mut(), 0, 0);
            if r <= 0 {
                eprintln!("[loop] exit: GetMessageW={r} last_msg=0x{:x}", msg.message);
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if let Some(t) = tray.as_mut() {
            t.destroy();
        }
    }
}

/// 素材目录（只读）：exe 同级 `assets` → `%APPDATA%\dsh-pet-rs\assets` → 开发期工程目录。
fn asset_dir() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let p = d.join("assets");
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    if let Some(base) = appdata() {
        let p = base.join("dsh-pet-rs").join("assets");
        if p.is_dir() {
            return Some(p);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    if dev.is_dir() {
        return Some(dev);
    }
    None
}

/// 配置目录（可写）：`%APPDATA%\dsh-pet-rs`。
///
/// 装到 Program Files 这类只读位置时，素材目录写不进去，
/// 所以 settings.json / icon.ico 一律落到这里。
fn config_dir() -> PathBuf {
    let base = appdata().unwrap_or_else(std::env::temp_dir);
    let dir = base.join("dsh-pet-rs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn appdata() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
}

fn show_error(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR};
    let msg = to_wide(text);
    unsafe {
        MessageBoxW(
            null_mut(),
            msg.as_ptr(),
            to_wide("dsh-pet-rs 启动失败").as_ptr(),
            MB_ICONERROR,
        );
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // 点宠物不给焦点，其他程序保持前台
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,

        WM_TIMER => {
            if wparam == TIMER_FRAME {
                with_app(|app| app.tick());
            }
            0
        }

        WM_LBUTTONDOWN => {
            with_app(|app| {
                app.begin_drag();
                km::SetCapture(hwnd);
            });
            0
        }

        WM_MOUSEMOVE => {
            with_app(|app| {
                if app.is_dragging() {
                    app.drag_move();
                }
            });
            0
        }

        WM_LBUTTONUP => {
            with_app(|app| {
                app.end_drag();
                km::ReleaseCapture();
            });
            0
        }

        WM_RBUTTONUP => {
            let cats = with_app(|app| app.categories()).unwrap_or_default();
            let cmd = tray::popup_pet(hwnd, &cats);
            if cmd > 0 {
                let flat: Vec<String> = cats.iter().flat_map(|(_, v)| v.clone()).collect();
                handle_command(hwnd, cmd, &flat);
            }
            0
        }

        tray::WM_TRAY => {
            let ev = (lparam as u32) & 0xffff;
            if ev == WM_LBUTTONDBLCLK {
                // 双击托盘直接开设置
                settings::show();
            } else if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                let cmd = tray::popup_tray(hwnd);
                if cmd > 0 {
                    handle_command(hwnd, cmd, &[]);
                }
            }
            0
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }

        // 桌宠只从托盘/菜单退出，任何 WM_CLOSE（Alt+F4、系统广播、误触）一律吞掉
        WM_CLOSE => 0,

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn handle_command(hwnd: HWND, cmd: u32, flat: &[String]) {
    // 1..=N 是分类子菜单里平铺后的动画下标
    if cmd >= 1 && (cmd as usize) <= flat.len() {
        let name = flat[cmd as usize - 1].clone();
        with_app(|app| app.play_name(&name));
        return;
    }
    match cmd {
        tray::CMD_SHOW => {
            with_app(|app| {
                let vis = app.toggle_visible();
                unsafe {
                    if vis {
                        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        app.repaint();
                    } else {
                        ShowWindow(hwnd, 0); // SW_HIDE
                    }
                }
            });
        }
        tray::CMD_HOME => {
            with_app(|app| app.home());
        }
        tray::CMD_RELOAD => {
            let r = with_app(|app| app.reload());
            if let Some(Err(e)) = r {
                show_error(&e);
            }
        }
        tray::CMD_SETTINGS => {
            settings::show();
        }
        tray::CMD_EXIT => unsafe {
            DestroyWindow(hwnd);
        },
        _ => {}
    }
}

/// 借出 App 跑一段闭包；重入（如弹菜单期间进来的定时器）直接跳过。
pub(crate) fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        let app = guard.as_mut()?;
        Some(f(app))
    })
}
