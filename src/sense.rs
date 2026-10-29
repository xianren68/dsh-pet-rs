//! 系统传感器：电量、网络、下载、空闲、全屏、屏幕/会话状态。
//!
//! 所有阈值和轮询间隔来自 `assets/events.json` 的 `sensors` 段。这里只负责
//! “把系统状态翻译成事件”，不做任何动画或文案决定。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::Networking::WinInet::InternetGetConnectedState;
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    REG_EXPAND_SZ, REG_SZ,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowRect, IsIconic,
};

use crate::event::{BatteryCfg, DownloadCfg, FullscreenCfg, IdleCfg, NetworkCfg, SensorsConfig};
use crate::util::to_wide;

/// 轮询类传感器产生的事件。
#[derive(Debug, Clone)]
pub enum Signal {
    BatteryLow { pct: u8 },
    BatteryCritical { pct: u8 },
    BatteryCharging,
    BatteryFull,
    NetworkDown,
    NetworkUp,
    DownloadStart { name: String },
    DownloadDone { name: String, size: u64 },
    DownloadFailed { name: String },
    IdleSleep,
    IdleWake,
    FullscreenEnter,
    FullscreenExit,
}

pub struct Sense {
    cfg: SensorsConfig,
    battery: BatterySensor,
    network: NetworkSensor,
    download: DownloadSensor,
    idle: IdleSensor,
    fullscreen: FullscreenSensor,
    screen: ScreenState,
}

impl Sense {
    pub fn new(cfg: &SensorsConfig) -> Self {
        Self {
            cfg: cfg.clone(),
            battery: BatterySensor {
                first: true,
                ..BatterySensor::default()
            },
            network: NetworkSensor::default(),
            download: DownloadSensor::new(&cfg.download),
            idle: IdleSensor::default(),
            fullscreen: FullscreenSensor::default(),
            screen: ScreenState::new(),
        }
    }

    pub fn update_config(&mut self, cfg: &SensorsConfig) {
        self.cfg = cfg.clone();
        self.download.update_dir(&cfg.download);
    }

    pub fn poll(&mut self, now: u32) -> Vec<Signal> {
        let mut out = Vec::new();
        out.extend(self.battery.poll(&self.cfg.battery, now, false));
        if let Some(s) = self.network.poll(&self.cfg.network, now) {
            out.push(s);
        }
        out.extend(self.download.poll(&self.cfg.download, now));
        if let Some(s) = self.idle.poll(&self.cfg.idle, now) {
            out.push(s);
        }
        if let Some(s) = self.fullscreen.poll(&self.cfg.fullscreen, now) {
            out.push(s);
        }
        out
    }

    pub fn poll_battery(&mut self, now: u32) -> Vec<Signal> {
        self.battery.poll(&self.cfg.battery, now, true)
    }

    pub fn is_locked(&self) -> bool {
        self.screen.locked
    }

    pub fn display_on(&self) -> bool {
        self.screen.display_on
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen.on
    }

    pub fn set_display(&mut self, on: bool, now: u32) -> Option<u32> {
        if on {
            self.screen.display_on = true;
            if self.screen.locked {
                None
            } else {
                self.screen.take_away(now)
            }
        } else {
            self.screen.display_on = false;
            self.screen.mark_away(now);
            None
        }
    }

    pub fn set_locked(&mut self, locked: bool, now: u32) -> Option<u32> {
        if locked {
            self.screen.locked = true;
            self.screen.mark_away(now);
            None
        } else {
            self.screen.locked = false;
            if self.screen.display_on {
                self.screen.take_away(now)
            } else {
                None
            }
        }
    }

    pub fn on_suspend(&mut self, now: u32) {
        self.screen.display_on = false;
        self.screen.mark_away(now);
        self.idle.reset();
    }

    /// 返回离开时长；恢复时即使没收到息屏消息也按 0 秒处理，交给 App 决定是否问好。
    pub fn on_resume(&mut self, now: u32) -> Option<u32> {
        self.screen.display_on = true;
        self.idle.reset();
        if self.screen.locked {
            None
        } else {
            Some(self.screen.take_away(now).unwrap_or(0))
        }
    }
}

struct ScreenState {
    display_on: bool,
    locked: bool,
    away_since: Option<u32>,
}

impl ScreenState {
    fn new() -> Self {
        Self {
            display_on: true,
            locked: false,
            away_since: None,
        }
    }

    fn mark_away(&mut self, now: u32) {
        if self.away_since.is_none() {
            self.away_since = Some(now);
        }
    }

    fn take_away(&mut self, now: u32) -> Option<u32> {
        self.away_since.take().map(|t| now.wrapping_sub(t) / 1000)
    }
}

#[derive(Default)]
struct BatterySensor {
    last_poll: u32,
    first: bool,
    last_ac: Option<bool>,
    low_fired: bool,
    critical_fired: bool,
    full_fired: bool,
}

impl BatterySensor {
    fn poll(&mut self, cfg: &BatteryCfg, now: u32, force: bool) -> Vec<Signal> {
        if !force && now.wrapping_sub(self.last_poll) < cfg.poll_sec.max(1) * 1000 {
            return Vec::new();
        }
        self.last_poll = now;

        let mut st: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        if unsafe { GetSystemPowerStatus(&mut st) } == 0 {
            return Vec::new();
        }
        // 128 = NoSystemBattery（台式机）；255 = 状态未知
        if st.BatteryFlag & 128 != 0 || st.BatteryFlag == 255 {
            return Vec::new();
        }
        if st.ACLineStatus == 255 {
            return Vec::new();
        }

        let pct = if st.BatteryLifePercent == 255 {
            0
        } else {
            st.BatteryLifePercent
        };
        let ac = st.ACLineStatus == 1;
        let mut out = Vec::new();

        // 第一次只记基线。插着电启动时别急着喊“充满”，
        // 但用电池启动且电量已经很低仍然要提醒。
        if self.first {
            self.first = false;
            self.last_ac = Some(ac);
            self.full_fired = ac && pct >= 99;
            if ac {
                return Vec::new();
            }
        }

        if self.last_ac != Some(ac) {
            if self.last_ac.is_some() && ac {
                if pct >= 99 {
                    out.push(Signal::BatteryFull);
                    self.full_fired = true;
                } else {
                    out.push(Signal::BatteryCharging);
                }
            }
            if !ac {
                self.low_fired = false;
                self.critical_fired = false;
                self.full_fired = false;
            }
            self.last_ac = Some(ac);
        }

        if ac {
            if pct >= 99 && !self.full_fired {
                out.push(Signal::BatteryFull);
                self.full_fired = true;
            }
            if pct < 99 {
                self.full_fired = false;
            }
        } else {
            // 电量回升（拔插/校准）时允许再次提醒
            if pct > cfg.low_pct.saturating_add(5) {
                self.low_fired = false;
                self.critical_fired = false;
            }
            if pct <= cfg.critical_pct && !self.critical_fired {
                out.push(Signal::BatteryCritical { pct });
                self.critical_fired = true;
                self.low_fired = true;
            } else if pct <= cfg.low_pct && !self.low_fired {
                out.push(Signal::BatteryLow { pct });
                self.low_fired = true;
            }
        }
        out
    }
}

#[derive(Default)]
struct NetworkSensor {
    last_poll: u32,
    last_up: Option<bool>,
    pending: u32,
    pending_up: bool,
}

impl NetworkSensor {
    fn poll(&mut self, cfg: &NetworkCfg, now: u32) -> Option<Signal> {
        if now.wrapping_sub(self.last_poll) < cfg.poll_sec.max(1) * 1000 {
            return None;
        }
        self.last_poll = now;

        let mut flags: u32 = 0;
        let up = unsafe { InternetGetConnectedState(&mut flags, 0) } != 0;
        let Some(last) = self.last_up else {
            // 启动时只记基线，不上报“网络断开”
            self.last_up = Some(up);
            return None;
        };
        if up == last {
            self.pending = 0;
            return None;
        }
        if self.pending == 0 {
            self.pending_up = up;
        }
        if self.pending_up != up {
            self.pending = 0;
            self.pending_up = up;
        }
        self.pending += 1;
        if self.pending < cfg.confirm_ticks.max(1) {
            return None;
        }

        self.last_up = Some(up);
        self.pending = 0;
        Some(if up {
            Signal::NetworkUp
        } else {
            Signal::NetworkDown
        })
    }
}

#[derive(Clone)]
struct FileEntry {
    size: u64,
    mtime: u64,
    temp: bool,
}

struct DownloadSensor {
    raw_dir: String,
    dir: Option<PathBuf>,
    last_poll: u32,
    initialized: bool,
    prev: HashMap<String, FileEntry>,
}

impl DownloadSensor {
    fn new(cfg: &DownloadCfg) -> Self {
        let mut s = Self {
            raw_dir: cfg.dir.clone(),
            dir: None,
            last_poll: 0,
            initialized: false,
            prev: HashMap::new(),
        };
        s.dir = resolve_download_dir(&cfg.dir);
        s
    }

    fn update_dir(&mut self, cfg: &DownloadCfg) {
        // 目录没变且已经解析成功时才跳过；首次启动时 Downloads 还不存在的话，
        // 重载素材时应该再试一次。
        if self.raw_dir == cfg.dir && self.dir.is_some() {
            return;
        }
        self.raw_dir = cfg.dir.clone();
        self.dir = resolve_download_dir(&cfg.dir);
        self.initialized = false;
        self.prev.clear();
    }

    fn poll(&mut self, cfg: &DownloadCfg, now: u32) -> Vec<Signal> {
        if now.wrapping_sub(self.last_poll) < cfg.poll_sec.max(1) * 1000 {
            return Vec::new();
        }
        self.last_poll = now;
        let Some(dir) = self.dir.as_deref() else {
            return Vec::new();
        };
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };

        let mut cur: HashMap<String, FileEntry> = HashMap::new();
        for e in rd.flatten().take(8000) {
            let Ok(md) = e.metadata() else { continue };
            if !md.is_file() {
                continue;
            }
            let Some(name) = e.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            let temp = is_temp(&name, &cfg.temp_exts);
            cur.insert(
                name,
                FileEntry {
                    size: md.len(),
                    mtime: md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    temp,
                },
            );
        }

        if !self.initialized {
            self.prev = cur;
            self.initialized = true;
            return Vec::new();
        }

        let mut out = Vec::new();
        for (name, e) in &cur {
            if e.temp && !self.prev.contains_key(name) {
                out.push(Signal::DownloadStart {
                    name: clean_name(name, true, &cfg.temp_exts),
                });
            }
        }

        // 临时文件消失：找同一时间窗口里新出现的成品文件，找不到就认为失败/取消。
        let candidates: Vec<(String, u64, u64)> = cur
            .iter()
            .filter(|(n, e)| {
                !e.temp
                    && match self.prev.get(*n) {
                        None => true,
                        Some(old) => old.size != e.size || old.mtime != e.mtime,
                    }
            })
            .map(|(n, e)| (n.clone(), e.size, e.mtime))
            .collect();
        let mut used: HashSet<String> = HashSet::new();
        for (name, old) in &self.prev {
            if !old.temp || cur.contains_key(name) {
                continue;
            }
            let best = candidates
                .iter()
                .filter(|(n, _, _)| !used.contains(n))
                .filter(|(_, _, mt)| mt.saturating_add(5) >= old.mtime)
                .max_by_key(|(_, _, mt)| *mt);
            match best {
                Some((n, size, _)) => {
                    used.insert(n.clone());
                    out.push(Signal::DownloadDone {
                        name: clean_name(n, false, &cfg.temp_exts),
                        size: *size,
                    });
                }
                None => out.push(Signal::DownloadFailed {
                    name: clean_name(name, true, &cfg.temp_exts),
                }),
            }
        }

        self.prev = cur;
        out
    }
}

fn is_temp(name: &str, exts: &[String]) -> bool {
    let lower = name.to_ascii_lowercase();
    exts.iter().any(|e| {
        let e = e.to_ascii_lowercase();
        !e.is_empty() && lower.ends_with(&e)
    })
}

fn clean_name(name: &str, temp: bool, exts: &[String]) -> String {
    if name.starts_with("Unconfirmed ") {
        return "文件".to_string();
    }
    let mut s = name.to_string();
    if temp {
        let lower = s.to_ascii_lowercase();
        for e in exts {
            let e = e.to_ascii_lowercase();
            if !e.is_empty() && lower.ends_with(&e) {
                s.truncate(s.len().saturating_sub(e.len()));
                break;
            }
        }
    }
    if s.is_empty() {
        "文件".to_string()
    } else {
        s
    }
}

fn resolve_download_dir(cfg: &str) -> Option<PathBuf> {
    let cfg = cfg.trim();
    if !cfg.is_empty() {
        let p = PathBuf::from(expand_env(cfg));
        return if p.is_dir() { Some(p) } else { None };
    }
    if let Some(p) = read_downloads_registry() {
        if p.is_dir() {
            return Some(p);
        }
    }
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    let p = home.join("Downloads");
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

fn read_downloads_registry() -> Option<PathBuf> {
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
    const VALUE: &str = r"{374DE290-123F-4565-9164-39C4925E467B}";
    unsafe {
        let mut hkey = std::ptr::null_mut();
        let key = to_wide(KEY);
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        ) != 0
        {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut cb = (buf.len() * 2) as u32;
        let mut ty = 0u32;
        let val = to_wide(VALUE);
        let ok = RegQueryValueExW(
            hkey,
            val.as_ptr(),
            std::ptr::null(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut cb,
        );
        RegCloseKey(hkey);
        if ok != 0 || (ty != REG_SZ && ty != REG_EXPAND_SZ) || cb < 2 {
            return None;
        }
        let len = (cb as usize / 2).saturating_sub(1).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..len]);
        let s = s.trim_matches('\0').trim();
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(expand_env(s)))
        }
    }
}

fn expand_env(s: &str) -> String {
    let mut out = s.to_string();
    for key in [
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        if let Ok(v) = std::env::var(key) {
            out = out.replace(&format!("%{key}%"), &v);
        }
    }
    out
}

#[derive(Default)]
struct IdleSensor {
    last_poll: u32,
    idle: bool,
}

impl IdleSensor {
    fn reset(&mut self) {
        self.idle = false;
    }

    fn poll(&mut self, cfg: &IdleCfg, now: u32) -> Option<Signal> {
        if now.wrapping_sub(self.last_poll) < 2_000 {
            return None;
        }
        self.last_poll = now;

        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if unsafe { GetLastInputInfo(&mut lii) } == 0 {
            return None;
        }
        let idle_ms = now.wrapping_sub(lii.dwTime);

        if !self.idle && idle_ms >= cfg.sleep_sec.max(1) * 1000 {
            self.idle = true;
            Some(Signal::IdleSleep)
        } else if self.idle && idle_ms < cfg.wake_sec.max(1) * 1000 {
            self.idle = false;
            Some(Signal::IdleWake)
        } else {
            None
        }
    }
}

#[derive(Default)]
struct FullscreenSensor {
    last_poll: u32,
    on: bool,
}

impl FullscreenSensor {
    fn poll(&mut self, cfg: &FullscreenCfg, now: u32) -> Option<Signal> {
        if now.wrapping_sub(self.last_poll) < cfg.poll_sec.max(1) * 1000 {
            return None;
        }
        self.last_poll = now;

        let mut state = 0i32;
        let ok = unsafe { SHQueryUserNotificationState(&mut state) } >= 0;
        let fs = ok
            && match state {
                QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE => true,
                // QUNS_BUSY 也会被“专注/演示”模式触发，必须再确认前台窗口
                // 真的盖住了整块屏幕，否则普通最大化窗口也会让桌宠消失。
                QUNS_BUSY => foreground_covers_monitor(),
                _ => false,
            };
        if fs == self.on {
            return None;
        }
        self.on = fs;
        Some(if fs {
            Signal::FullscreenEnter
        } else {
            Signal::FullscreenExit
        })
    }
}

/// QUNS_BUSY 的保守复核：前台窗口必须完整覆盖它所在的显示器。
fn foreground_covers_monitor() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() || IsIconic(hwnd) != 0 {
            return false;
        }

        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n > 0 {
            let class = String::from_utf16_lossy(&buf[..n as usize]);
            if matches!(
                class.as_str(),
                "DshPetRsWindow" | "DshPetSettings" | "DshPetBubble"
            ) {
                return false;
            }
        }

        let mut wr: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut wr) == 0 {
            return false;
        }
        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if hmon.is_null() {
            return false;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi) == 0 {
            return false;
        }
        let m = mi.rcMonitor;
        wr.left <= m.left + 2
            && wr.top <= m.top + 2
            && wr.right >= m.right - 2
            && wr.bottom >= m.bottom - 2
    }
}
