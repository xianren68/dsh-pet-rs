//! 设置面板：普通带标题栏的窗口 + 系统原生控件。
//!
//! 走原生控件（STATIC / BUTTON / CHECKBOX / trackbar）而不是自绘——省掉一整套
//! 控件逻辑，行为也符合系统习惯。所有改动即时作用到桌宠并写盘。
//!
//! 布局：三组滑块（大小 / 不透明度 / 三档权重）+ 两个开关
//! + 环境感知开关组 + 三个按钮。

use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetStockObject, SetBkMode, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    COLOR_WINDOW, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, HFONT, OUT_DEFAULT_PRECIS,
    TRANSPARENT, WHITE_BRUSH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::SS_LEFT;
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, TBS_AUTOTICKS, TBS_HORZ,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{EventToggles, Toggle};
use crate::util::to_wide;

// 控件 id
const ID_SIZE: i32 = 101;
const ID_SIZE_LBL: i32 = 102;
const ID_ALPHA: i32 = 103;
const ID_ALPHA_LBL: i32 = 104;
const ID_W_IDLE: i32 = 105;
const ID_W_IDLE_LBL: i32 = 106;
const ID_W_TURN: i32 = 107;
const ID_W_TURN_LBL: i32 = 108;
const ID_W_MOVE: i32 = 109;
const ID_W_MOVE_LBL: i32 = 110;
const ID_CHK_FIXED: i32 = 111;
const ID_CHK_AUTORUN: i32 = 112;
const ID_CHK_EVT_BATTERY: i32 = 113;
const ID_CHK_EVT_NETWORK: i32 = 114;
const ID_CHK_EVT_DOWNLOAD: i32 = 115;
const ID_CHK_EVT_IDLE: i32 = 116;
const ID_CHK_EVT_LOCK: i32 = 117;
const ID_CHK_EVT_FULLSCREEN: i32 = 118;
const ID_CHK_GREET: i32 = 119;
const ID_CHK_GREET_STARTUP: i32 = 120;
const ID_CHK_BUBBLE: i32 = 121;
const ID_CHK_BUBBLE_NAMES: i32 = 122;
const ID_CHK_SALARY: i32 = 130;
const ID_ED_WORK_START: i32 = 131;
const ID_ED_WORK_END: i32 = 132;
const ID_ED_LUNCH_START: i32 = 133;
const ID_ED_LUNCH_END: i32 = 134;
const ID_ED_SALARY_MONTH: i32 = 135;
const ID_HOME: i32 = 201;
const ID_RELOAD: i32 = 202;
const ID_CLOSE: i32 = 203;

pub const MIN_W: u32 = 200;
pub const MAX_W: u32 = 900;
const MIN_ALPHA: u32 = 40;

// 滑块与复选框消息（windows-sys 没导出，按 winuser.h / commctrl.h 手写）
const TBM_GETPOS: u32 = 0x0400;
const TBM_SETPOS: u32 = 0x0405;
const TBM_SETRANGE: u32 = 0x0406;
const TBM_SETTICFREQ: u32 = 0x0414;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: usize = 1;

struct Panel {
    hwnd: HWND,
    font: HFONT,
    size: HWND,
    size_lbl: HWND,
    alpha: HWND,
    alpha_lbl: HWND,
    w_idle: HWND,
    w_idle_lbl: HWND,
    w_turn: HWND,
    w_turn_lbl: HWND,
    w_move: HWND,
    w_move_lbl: HWND,
    chk_fixed: HWND,
    chk_autorun: HWND,
    chk_evt_battery: HWND,
    chk_evt_network: HWND,
    chk_evt_download: HWND,
    chk_evt_idle: HWND,
    chk_evt_lock: HWND,
    chk_evt_fullscreen: HWND,
    chk_greet: HWND,
    chk_greet_startup: HWND,
    chk_bubble: HWND,
    chk_bubble_names: HWND,
    chk_salary: HWND,
    ed_work_start: HWND,
    ed_work_end: HWND,
    ed_lunch_start: HWND,
    ed_lunch_end: HWND,
    ed_salary_month: HWND,
}

static mut PANEL: Option<Panel> = None;

fn panel() -> Option<&'static mut Panel> {
    unsafe { (*(&raw mut PANEL)).as_mut() }
}

fn size_text(w: u32) -> String {
    let h = ((w as f32) * 9.0 / 16.0).round() as u32;
    format!("{w} × {h} 像素")
}

fn pct_text(a: u32) -> String {
    format!("{}%", (a as f32 / 255.0 * 100.0).round() as u32)
}

/// 打开（或前置）设置面板，同时把控件状态与当前值同步。
pub fn show() {
    if let Some(p) = panel() {
        unsafe {
            if let Some((w, a, (wi, wt, wm), fixed, tg)) = crate::with_app(|app| {
                (
                    app.width(),
                    app.alpha(),
                    app.weights(),
                    app.fixed(),
                    app.toggles(),
                )
            }) {
                set_slider(p.size, w.clamp(MIN_W, MAX_W));
                SetWindowTextW(p.size_lbl, to_wide(&size_text(w)).as_ptr());
                set_slider(p.alpha, a as u32);
                SetWindowTextW(p.alpha_lbl, to_wide(&pct_text(a as u32)).as_ptr());
                set_slider(p.w_idle, wi);
                SetWindowTextW(p.w_idle_lbl, to_wide(&wi.to_string()).as_ptr());
                set_slider(p.w_turn, wt);
                SetWindowTextW(p.w_turn_lbl, to_wide(&wt.to_string()).as_ptr());
                set_slider(p.w_move, wm);
                SetWindowTextW(p.w_move_lbl, to_wide(&wm.to_string()).as_ptr());
                set_check(p.chk_fixed, fixed);
                set_check(p.chk_evt_battery, tg.battery);
                set_check(p.chk_evt_network, tg.network);
                set_check(p.chk_evt_download, tg.download);
                set_check(p.chk_evt_idle, tg.idle);
                set_check(p.chk_evt_lock, tg.lock);
                set_check(p.chk_evt_fullscreen, tg.fullscreen);
                set_check(p.chk_greet, tg.greet);
                set_check(p.chk_greet_startup, tg.greet_startup);
                set_check(p.chk_bubble, tg.bubble);
                set_check(p.chk_bubble_names, tg.bubble_names);
            }
            SendMessageW(
                p.chk_autorun,
                BM_SETCHECK,
                if crate::autorun::is_enabled() {
                    BST_CHECKED
                } else {
                    0
                },
                0,
            );
            ShowWindow(p.hwnd, SW_SHOW);
            SetForegroundWindow(p.hwnd);
        }
        return;
    }
    create();
}

fn set_slider(h: HWND, v: u32) {
    unsafe {
        SendMessageW(h, TBM_SETPOS, 1, v as isize);
    }
}

fn set_check(h: HWND, on: bool) {
    unsafe {
        SendMessageW(h, BM_SETCHECK, if on { BST_CHECKED } else { 0 }, 0);
    }
}

fn checked(h: HWND) -> bool {
    unsafe { SendMessageW(h, BM_GETCHECK, 0, 0) as usize == BST_CHECKED }
}

/// 读面板上五个输入框，写回配置。输入框失焦（EN_KILLFOCUS）时调用。
///
/// 月薪解析失败就当没改（保留 8000 的兜底），不弹错 —— 输到一半的 `9` 也算合法。
fn save_salary_edits() {
    let Some(p) = panel() else {
        return;
    };
    let get = |h: HWND| -> String {
        let mut buf = [0u16; 32];
        let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    };
    let on = unsafe { SendMessageW(p.chk_salary, BM_GETCHECK, 0, 0) as usize == BST_CHECKED };
    let ws = get(p.ed_work_start).trim().to_string();
    let we = get(p.ed_work_end).trim().to_string();
    let ls = get(p.ed_lunch_start).trim().to_string();
    let le = get(p.ed_lunch_end).trim().to_string();
    let month = get(p.ed_salary_month)
        .trim()
        .parse::<u32>()
        .unwrap_or_else(|_| crate::app::UserSettings::default().salary_month);
    crate::with_app(|app| app.set_salary(on, ws, we, ls, le, month));
}

fn apply_toggle(h: HWND, which: Toggle) {
    let on = checked(h);
    crate::with_app(|app| app.set_toggle(which, on));
}

fn create() {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        };
        InitCommonControlsEx(&icc);

        let hinst = GetModuleHandleW(null());
        let cls = to_wide("DshPetSettings");

        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: crate::app_icon().unwrap_or_else(|| LoadIconW(null_mut(), IDI_APPLICATION)),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as usize as *mut _,
            lpszMenuName: null(),
            lpszClassName: cls.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);

        // 客户区 420×772
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 420,
            bottom: 772,
        };
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        AdjustWindowRect(&mut rect, style, 0);

        let hwnd = CreateWindowExW(
            0,
            cls.as_ptr(),
            to_wide("桌宠设置").as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            null_mut(),
            null_mut(),
            hinst,
            null_mut(),
        );
        if hwnd.is_null() {
            eprintln!(
                "[settings] CreateWindowExW failed: {}",
                GetLastError()
            );
            return;
        }

        let font = CreateFontW(
            -14,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            to_wide("Microsoft YaHei UI").as_ptr(),
        );

        let mk = |cls: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32| {
            let c = to_wide(cls);
            let t = to_wide(text);
            let h_ctl = CreateWindowExW(
                0,
                c.as_ptr(),
                t.as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                w,
                h,
                hwnd,
                id as usize as *mut _,
                hinst,
                null_mut(),
            );
            SendMessageW(h_ctl, WM_SETFONT, font as usize, 1);
            h_ctl
        };

        let slider = |x: i32, y: i32, w: i32, id: i32, lo: u32, hi: u32, tick: u32| {
            let h = mk(
                "msctls_trackbar32",
                "",
                TBS_HORZ | TBS_AUTOTICKS,
                x,
                y,
                w,
                32,
                id,
            );
            SendMessageW(
                h,
                TBM_SETRANGE,
                1,
                ((lo as isize) | ((hi as isize) << 16)) as isize,
            );
            SendMessageW(h, TBM_SETTICFREQ, tick as usize, 0);
            h
        };

        // 初始值从 App 拿
        let (w0, a0, (wi0, wt0, wm0), fixed0, tg0) = crate::with_app(|app| {
            (
                app.width(),
                app.alpha(),
                app.weights(),
                app.fixed(),
                app.toggles(),
            )
        })
        .unwrap_or((
            640,
            255,
            (10, 5, 5),
            false,
            EventToggles {
                battery: true,
                network: true,
                download: true,
                idle: true,
                lock: true,
                fullscreen: true,
                greet: true,
                greet_startup: true,
                bubble: true,
                bubble_names: true,
            },
        ));

        // ---- 大小 ----
        mk("STATIC", "宠物大小", SS_LEFT, 20, 12, 200, 20, 0);
        let size = slider(20, 34, 380, ID_SIZE, MIN_W, MAX_W, 50);
        let size_lbl = mk("STATIC", &size_text(w0), SS_LEFT, 20, 68, 200, 20, ID_SIZE_LBL);

        // ---- 不透明度 ----
        mk("STATIC", "不透明度", SS_LEFT, 20, 94, 200, 20, 0);
        let alpha = slider(20, 116, 380, ID_ALPHA, MIN_ALPHA, 255, 43);
        let alpha_lbl = mk(
            "STATIC",
            &pct_text(a0 as u32),
            SS_LEFT,
            20,
            150,
            200,
            20,
            ID_ALPHA_LBL,
        );

        // ---- 行为权重 ----
        mk(
            "STATIC",
            "行为倾向（相对权重，剩下的份额给随机小动作）",
            SS_LEFT,
            20,
            178,
            380,
            20,
            0,
        );
        mk("STATIC", "待机", SS_LEFT, 20, 206, 44, 20, 0);
        let w_idle = slider(70, 200, 240, ID_W_IDLE, 0, 100, 25);
        let w_idle_lbl = mk(
            "STATIC",
            &wi0.to_string(),
            SS_LEFT,
            322,
            206,
            60,
            20,
            ID_W_IDLE_LBL,
        );

        mk("STATIC", "转向", SS_LEFT, 20, 242, 44, 20, 0);
        let w_turn = slider(70, 236, 240, ID_W_TURN, 0, 100, 25);
        let w_turn_lbl = mk(
            "STATIC",
            &wt0.to_string(),
            SS_LEFT,
            322,
            242,
            60,
            20,
            ID_W_TURN_LBL,
        );

        mk("STATIC", "走动", SS_LEFT, 20, 278, 44, 20, 0);
        let w_move = slider(70, 272, 240, ID_W_MOVE, 0, 100, 25);
        let w_move_lbl = mk(
            "STATIC",
            &wm0.to_string(),
            SS_LEFT,
            322,
            278,
            60,
            20,
            ID_W_MOVE_LBL,
        );

        // ---- 开关 ----
        let chk_fixed = mk(
            "BUTTON",
            "原地待机（不自己走、不自己转朝向）",
            BS_AUTOCHECKBOX as u32,
            20,
            318,
            380,
            24,
            ID_CHK_FIXED,
        );
        let chk_autorun = mk(
            "BUTTON",
            "开机自动启动",
            BS_AUTOCHECKBOX as u32,
            20,
            348,
            380,
            24,
            ID_CHK_AUTORUN,
        );
        SendMessageW(
            chk_fixed,
            BM_SETCHECK,
            if fixed0 { BST_CHECKED } else { 0 },
            0,
        );
        if crate::autorun::is_enabled() {
            SendMessageW(chk_autorun, BM_SETCHECK, BST_CHECKED, 0);
        }

        // ---- 环境感知 ----
        mk(
            "BUTTON",
            "环境感知（全部本地，不联网）",
            BS_GROUPBOX as u32,
            12,
            382,
            396,
            168,
            0,
        );
        let chk_evt_battery = mk(
            "BUTTON",
            "电量低/充满",
            BS_AUTOCHECKBOX as u32,
            24,
            406,
            180,
            22,
            ID_CHK_EVT_BATTERY,
        );
        let chk_evt_network = mk(
            "BUTTON",
            "网络断开/恢复",
            BS_AUTOCHECKBOX as u32,
            216,
            406,
            180,
            22,
            ID_CHK_EVT_NETWORK,
        );
        let chk_evt_download = mk(
            "BUTTON",
            "下载开始/完成",
            BS_AUTOCHECKBOX as u32,
            24,
            432,
            180,
            22,
            ID_CHK_EVT_DOWNLOAD,
        );
        let chk_evt_idle = mk(
            "BUTTON",
            "空闲时睡觉",
            BS_AUTOCHECKBOX as u32,
            216,
            432,
            180,
            22,
            ID_CHK_EVT_IDLE,
        );
        let chk_evt_lock = mk(
            "BUTTON",
            "锁屏/恢复",
            BS_AUTOCHECKBOX as u32,
            24,
            458,
            180,
            22,
            ID_CHK_EVT_LOCK,
        );
        let chk_evt_fullscreen = mk(
            "BUTTON",
            "全屏时隐藏",
            BS_AUTOCHECKBOX as u32,
            216,
            458,
            180,
            22,
            ID_CHK_EVT_FULLSCREEN,
        );
        let chk_greet = mk(
            "BUTTON",
            "点亮屏幕问好",
            BS_AUTOCHECKBOX as u32,
            24,
            484,
            180,
            22,
            ID_CHK_GREET,
        );
        let chk_greet_startup = mk(
            "BUTTON",
            "启动时问好",
            BS_AUTOCHECKBOX as u32,
            216,
            484,
            180,
            22,
            ID_CHK_GREET_STARTUP,
        );
        let chk_bubble = mk(
            "BUTTON",
            "显示气泡",
            BS_AUTOCHECKBOX as u32,
            24,
            510,
            180,
            22,
            ID_CHK_BUBBLE,
        );
        let chk_bubble_names = mk(
            "BUTTON",
            "气泡显示文件名",
            BS_AUTOCHECKBOX as u32,
            216,
            510,
            180,
            22,
            ID_CHK_BUBBLE_NAMES,
        );
        set_check(chk_evt_battery, tg0.battery);
        set_check(chk_evt_network, tg0.network);
        set_check(chk_evt_download, tg0.download);
        set_check(chk_evt_idle, tg0.idle);
        set_check(chk_evt_lock, tg0.lock);
        set_check(chk_evt_fullscreen, tg0.fullscreen);
        set_check(chk_greet, tg0.greet);
        set_check(chk_greet_startup, tg0.greet_startup);
        set_check(chk_bubble, tg0.bubble);
        set_check(chk_bubble_names, tg0.bubble_names);

        // ---- 数工资 ----
        mk(
            "BUTTON",
            "数工资（按今天已工作比例折算）",
            BS_GROUPBOX as u32,
            12,
            558,
            396,
            150,
            0,
        );
        let chk_salary = mk(
            "BUTTON",
            "启用（右键菜单出现「数工资」）",
            BS_AUTOCHECKBOX as u32,
            24,
            582,
            240,
            22,
            ID_CHK_SALARY,
        );
        // 原生 EDIT：样式里 0x0080 是 ES_AUTOHSCROLL（windows-sys 没导出）
        let ed_style = WS_BORDER | WS_TABSTOP | 0x0080;
        mk("STATIC", "上班", SS_LEFT, 24, 612, 32, 20, 0);
        let ed_work_start = mk("EDIT", "", ed_style, 58, 610, 56, 22, ID_ED_WORK_START);
        mk("STATIC", "下班", SS_LEFT, 124, 612, 32, 20, 0);
        let ed_work_end = mk("EDIT", "", ed_style, 158, 610, 56, 22, ID_ED_WORK_END);
        mk("STATIC", "午休", SS_LEFT, 224, 612, 32, 20, 0);
        let ed_lunch_start = mk("EDIT", "", ed_style, 258, 610, 56, 22, ID_ED_LUNCH_START);
        mk("STATIC", "至", SS_LEFT, 318, 612, 16, 20, 0);
        let ed_lunch_end = mk("EDIT", "", ed_style, 336, 610, 56, 22, ID_ED_LUNCH_END);
        mk("STATIC", "月薪", SS_LEFT, 24, 644, 32, 20, 0);
        let ed_salary_month = mk("EDIT", "", ed_style, 58, 642, 68, 22, ID_ED_SALARY_MONTH);
        mk(
            "STATIC",
            "元/月，按 21.75 个计薪日折算",
            SS_LEFT,
            132,
            644,
            250,
            20,
            0,
        );

        // 数工资的初始值（控件建完才能填）
        let (sal_on, ws, we, ls, le, month) =
            crate::with_app(|app| app.salary_cfg()).unwrap_or_default();
        set_check(chk_salary, sal_on);
        let put = |h: HWND, s: &str| {
            SetWindowTextW(h, to_wide(s).as_ptr());
        };
        put(ed_work_start, &ws);
        put(ed_work_end, &we);
        put(ed_lunch_start, &ls);
        put(ed_lunch_end, &le);
        put(ed_salary_month, &month.to_string());

        // ---- 按钮 ----
        mk(
            "BUTTON",
            "回到角落",
            BS_PUSHBUTTON as u32,
            20,
            716,
            120,
            32,
            ID_HOME,
        );
        mk(
            "BUTTON",
            "重载素材",
            BS_PUSHBUTTON as u32,
            150,
            716,
            120,
            32,
            ID_RELOAD,
        );
        mk(
            "BUTTON",
            "关闭",
            BS_PUSHBUTTON as u32,
            280,
            716,
            120,
            32,
            ID_CLOSE,
        );

        *(&raw mut PANEL) = Some(Panel {
            hwnd,
            font,
            size,
            size_lbl,
            alpha,
            alpha_lbl,
            w_idle,
            w_idle_lbl,
            w_turn,
            w_turn_lbl,
            w_move,
            w_move_lbl,
            chk_fixed,
            chk_autorun,
            chk_evt_battery,
            chk_evt_network,
            chk_evt_download,
            chk_evt_idle,
            chk_evt_lock,
            chk_evt_fullscreen,
            chk_greet,
            chk_greet_startup,
            chk_bubble,
            chk_bubble_names,
            chk_salary,
            ed_work_start,
            ed_work_end,
            ed_lunch_start,
            ed_lunch_end,
            ed_salary_month,
        });

        set_slider(size, w0.clamp(MIN_W, MAX_W));
        set_slider(alpha, a0 as u32);
        set_slider(w_idle, wi0);
        set_slider(w_turn, wt0);
        set_slider(w_move, wm0);

        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(size);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_HSCROLL => {
            if let Some(p) = panel() {
                let who = lparam as HWND;
                let pos = SendMessageW(who, TBM_GETPOS, 0, 0) as u32;
                if who == p.size {
                    SetWindowTextW(p.size_lbl, to_wide(&size_text(pos)).as_ptr());
                    crate::with_app(|app| app.set_width(pos));
                } else if who == p.alpha {
                    SetWindowTextW(p.alpha_lbl, to_wide(&pct_text(pos)).as_ptr());
                    crate::with_app(|app| app.set_alpha(pos.min(255) as u8));
                } else if who == p.w_idle {
                    SetWindowTextW(p.w_idle_lbl, to_wide(&pos.to_string()).as_ptr());
                    crate::with_app(|app| app.set_weight(0, pos));
                } else if who == p.w_turn {
                    SetWindowTextW(p.w_turn_lbl, to_wide(&pos.to_string()).as_ptr());
                    crate::with_app(|app| app.set_weight(1, pos));
                } else if who == p.w_move {
                    SetWindowTextW(p.w_move_lbl, to_wide(&pos.to_string()).as_ptr());
                    crate::with_app(|app| app.set_weight(2, pos));
                }
            }
            0
        }

        // 静态文本用窗口底色：SetBkMode 透明 + 返回白刷子
        WM_CTLCOLORSTATIC => {
            let hdc = wparam as _;
            SetBkMode(hdc, TRANSPARENT as i32);
            GetStockObject(WHITE_BRUSH) as LRESULT
        }

        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            let code = ((wparam >> 16) & 0xffff) as u32;
            // EN_KILLFOCUS = 0x0200：离开输入框才写盘，不然每敲一个字都会落一次文件
            if code == 0x0200
                && matches!(
                    id,
                    ID_ED_WORK_START
                        | ID_ED_WORK_END
                        | ID_ED_LUNCH_START
                        | ID_ED_LUNCH_END
                        | ID_ED_SALARY_MONTH
                )
            {
                save_salary_edits();
                return 0;
            }
            match id {
                ID_CHK_FIXED => {
                    if let Some(p) = panel() {
                        let on =
                            SendMessageW(p.chk_fixed, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
                        crate::with_app(|app| app.set_fixed(on));
                    }
                }
                ID_CHK_AUTORUN => {
                    if let Some(p) = panel() {
                        let on =
                            SendMessageW(p.chk_autorun, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
                        if let Err(e) = crate::autorun::set_enabled(on) {
                            crate::show_error(&e);
                            // 失败就把勾去掉，别让界面骗人
                            SendMessageW(
                                p.chk_autorun,
                                BM_SETCHECK,
                                if on { 0 } else { BST_CHECKED },
                                0,
                            );
                        }
                    }
                }
                ID_CHK_SALARY => {
                    save_salary_edits();
                }
                ID_CHK_EVT_BATTERY
                | ID_CHK_EVT_NETWORK
                | ID_CHK_EVT_DOWNLOAD
                | ID_CHK_EVT_IDLE
                | ID_CHK_EVT_LOCK
                | ID_CHK_EVT_FULLSCREEN
                | ID_CHK_GREET
                | ID_CHK_GREET_STARTUP
                | ID_CHK_BUBBLE
                | ID_CHK_BUBBLE_NAMES => {
                    if let Some(p) = panel() {
                        let (h, which) = match id {
                            ID_CHK_EVT_BATTERY => (p.chk_evt_battery, Toggle::Battery),
                            ID_CHK_EVT_NETWORK => (p.chk_evt_network, Toggle::Network),
                            ID_CHK_EVT_DOWNLOAD => (p.chk_evt_download, Toggle::Download),
                            ID_CHK_EVT_IDLE => (p.chk_evt_idle, Toggle::Idle),
                            ID_CHK_EVT_LOCK => (p.chk_evt_lock, Toggle::Lock),
                            ID_CHK_EVT_FULLSCREEN => (p.chk_evt_fullscreen, Toggle::Fullscreen),
                            ID_CHK_GREET => (p.chk_greet, Toggle::Greet),
                            ID_CHK_GREET_STARTUP => (p.chk_greet_startup, Toggle::GreetStartup),
                            ID_CHK_BUBBLE => (p.chk_bubble, Toggle::Bubble),
                            _ => (p.chk_bubble_names, Toggle::BubbleNames),
                        };
                        apply_toggle(h, which);
                    }
                }
                ID_HOME => {
                    crate::with_app(|app| app.home());
                }
                ID_RELOAD => {
                    if let Some(Err(e)) = crate::with_app(|app| app.reload()) {
                        crate::show_error(&e);
                    }
                    show();
                }
                ID_CLOSE => {
                    ShowWindow(hwnd, SW_HIDE);
                }
                _ => {}
            }
            0
        }

        WM_CLOSE => {
            ShowWindow(hwnd, SW_HIDE);
            0
        }

        WM_DESTROY => {
            if let Some(p) = panel() {
                if !p.font.is_null() {
                    DeleteObject(p.font as _);
                }
            }
            *(&raw mut PANEL) = None;
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
