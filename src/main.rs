//! dsh-pet-rs —— 极简原生桌宠。
//!
//! 一个 WS_EX_LAYERED 弹出窗 + 32 位 alpha 位图，靠 `UpdateLayeredWindow`
//! 每帧上屏。透明像素由系统合成器直接放过，不需要轮询光标做穿透。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autorun;
mod behavior;
mod bubble;
mod clip;
mod event;
mod holiday;
mod icon;
mod salary;
mod scale;
mod sense;
mod settings;
mod surface;
mod tray;
mod util;

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Power::{
    RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY,
    POWERBROADCAST_SETTING,
};
use windows_sys::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows_sys::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;
use windows_sys::Win32::UI::Input::KeyboardAndMouse as km;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetMessageW,
    LoadCursorW, PostQuitMessage, RegisterClassExW, SetProcessDPIAware, SetTimer, ShowWindow,
    TranslateMessage, DEVICE_NOTIFY_WINDOW_HANDLE, IDC_HAND, MA_NOACTIVATE, MSG,
    PBT_APMPOWERSTATUSCHANGE, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    PBT_POWERSETTINGCHANGE, SW_SHOWNOACTIVATE, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_ENDSESSION, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_POWERBROADCAST, WM_RBUTTONUP, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

use app::App;
use tray::Tray;
use util::to_wide;

const TIMER_FRAME: usize = 1;
const FRAME_MS: u32 = 16;
const TIMER_SENSE: usize = 2;
const SENSE_MS: u32 = 1_000;

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

    // --autorun-selftest：不开窗口，只跑一遍自启写入再撤销，用来诊断命令行
    if std::env::args().any(|a| a == "--autorun-selftest") {
        let was = autorun::is_enabled();
        println!("[selftest] 当前已登记自启 = {was}");
        match autorun::set_enabled(true) {
            Ok(()) => println!("[selftest] 写入成功，值 = {:?}", autorun::current_command()),
            Err(e) => {
                println!("[selftest] 写入失败: {e}");
                std::process::exit(1);
            }
        }
        if !was {
            match autorun::set_enabled(false) {
                Ok(()) => println!("[selftest] 已撤销，恢复原状"),
                Err(e) => println!("[selftest] 撤销失败: {e}"),
            }
        }
        return;
    }

    unsafe {
        // Per-Monitor V2：拿物理像素坐标，跨屏拖动能自适应缩放。
        // SetProcessDpiAwarenessContext 是 Win10 1703 才有的 API，必须运行时取：
        // 静态导入会让 Win7/8 的加载器在启动那一刻就失败（0xc0000005）。
        // 拿不到（老系统）就退回 Vista 时代就有的 SetProcessDPIAware。
        type SetDpiCtx = unsafe extern "system" fn(*mut std::ffi::c_void) -> i32;
        let mut dpi_ok = 0;
        let user32 = GetModuleHandleW(to_wide("user32.dll").as_ptr());
        if !user32.is_null() {
            if let Some(p) = GetProcAddress(user32, b"SetProcessDpiAwarenessContext\0".as_ptr()) {
                let f: SetDpiCtx = std::mem::transmute(p);
                dpi_ok = f(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            }
        }
        eprintln!("[dpi] per-monitor-v2 -> {dpi_ok}");
        if dpi_ok == 0 {
            let old = SetProcessDPIAware();
            eprintln!("[dpi] system-aware -> {old}");
        }

        let hinstance = GetModuleHandleW(null());
        let class_name = to_wide("DshPetRsWindow");

        if !FindWindowW(class_name.as_ptr(), null()).is_null() {
            show_error("dsh-pet-rs 已经在运行了。\n\n双击托盘图标可以打开设置。");
            return;
        }

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

        // 屏幕亮灭（GUID_CONSOLE_DISPLAY_STATE）走 PowerSetting 通知；
        // 锁屏/解锁走 WTS 会话通知。都失败也不影响桌宠本体。
        let power_notify: HPOWERNOTIFY = RegisterPowerSettingNotification(
            hwnd as HANDLE,
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        if power_notify == 0 {
            eprintln!("[power] RegisterPowerSettingNotification 失败");
        }
        let session_notify =
            WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) != 0;
        if !session_notify {
            eprintln!("[session] WTSRegisterSessionNotification 失败");
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

        // 调试/测试参数
        let has_play = std::env::args().any(|a| a == "--play");
        let wants_settings = std::env::args().any(|a| a == "--settings");
        let simulate = arg_value("--simulate");
        let debug_events = std::env::args().any(|a| a == "--debug-events");
        if has_play || wants_settings || simulate.is_some() {
            with_app(|app| app.skip_startup_greeting());
        }
        if debug_events {
            with_app(|app| app.set_debug(true));
        }
        if let Some(v) = arg_value("--greet-at") {
            if let Some(minutes) = event::parse_hm(&v) {
                with_app(|app| app.set_test_minutes(Some(minutes)));
            }
        }

        // --play "<动作名>"：启动即播某段，调试用
        if let Some(i) = std::env::args().position(|a| a == "--play") {
            if let Some(name) = std::env::args().nth(i + 1) {
                with_app(|app| app.play_name(&name));
            }
        }

        if let Some(spec) = simulate {
            with_app(|app| app.simulate(&spec));
        }

        // 带 --settings 启动时直接开设置面板（给快捷方式用）
        if wants_settings {
            settings::show();
        }
        SetTimer(hwnd, TIMER_FRAME, FRAME_MS, None);
        SetTimer(hwnd, TIMER_SENSE, SENSE_MS, None);

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

        with_app(|app| app.save_on_exit());
        bubble::destroy();
        if power_notify != 0 {
            UnregisterPowerSettingNotification(power_notify);
        }
        if session_notify {
            WTSUnRegisterSessionNotification(hwnd);
        }
        if let Some(t) = tray.as_mut() {
            t.destroy();
        }
    }
}

/// 取 `--foo value` 里的 value。
fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
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
            // 这个框不只用于启动失败，勾自启失败之类也走这里，标题别写死"启动失败"
            to_wide("dsh-pet-rs").as_ptr(),
            MB_ICONERROR,
        );
    }
}

fn guid_eq(a: &windows_sys::core::GUID, b: &windows_sys::core::GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // 点宠物不给焦点，其他程序保持前台
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,

        WM_TIMER => {
            match wparam {
                TIMER_FRAME => {
                    with_app(|app| app.tick());
                }
                TIMER_SENSE => {
                    with_app(|app| app.poll_senses());
                }
                _ => {}
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
            let (cats, salary) =
                with_app(|app| (app.categories(), app.salary_on())).unwrap_or_default();
            let cmd = tray::popup_pet(hwnd, &cats, salary);
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

        WM_POWERBROADCAST => {
            match wparam as u32 {
                PBT_APMPOWERSTATUSCHANGE => {
                    with_app(|app| app.poll_power());
                }
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                    with_app(|app| app.on_resume());
                }
                PBT_APMSUSPEND => {
                    with_app(|app| app.on_suspend());
                }
                PBT_POWERSETTINGCHANGE => {
                    let p = lparam as *const POWERBROADCAST_SETTING;
                    if !p.is_null() {
                        let setting = &*p;
                        if guid_eq(&setting.PowerSetting, &GUID_CONSOLE_DISPLAY_STATE)
                            && setting.DataLength >= 1
                        {
                            // 0=关屏 1=开屏 2=变暗；只有真正开屏才问好
                            let on = setting.Data[0] == 1;
                            with_app(|app| app.on_display(on));
                        }
                    }
                }
                _ => {}
            }
            0
        }

        WM_WTSSESSION_CHANGE => {
            match wparam as u32 {
                WTS_SESSION_LOCK => {
                    with_app(|app| app.on_session(true));
                }
                WTS_SESSION_UNLOCK => {
                    with_app(|app| app.on_session(false));
                }
                _ => {}
            }
            0
        }

        WM_DISPLAYCHANGE => {
            with_app(|app| app.on_display_change());
            0
        }

        WM_ENDSESSION => {
            if wparam != 0 {
                with_app(|app| app.save_on_exit());
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
                app.toggle_visible();
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
        tray::CMD_SALARY => {
            with_app(|app| app.count_salary());
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
