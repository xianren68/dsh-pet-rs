//! 中国节假日：拉当年的法定假日 / 调休安排，用来算当月实际工作日数。
//!
//! **这是项目里唯一会联网的模块**，而且一年只请求一次：结果落到
//! `%APPDATA%\dsh-pet-rs\holidays-<年>.json`，之后全部走本地缓存。
//!
//! 数据源 `timor.tech/api/holiday/year/<年>`，免费、无需 key，只返回
//! 「放假」和「调休补班」两类特殊日子，普通周末由本地星期推算。
//!
//! 拿不到数据时**不影响主流程** —— 调用方回落到 21.75 的固定月计薪天数。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::Networking::WinInet::{
    InternetCloseHandle, InternetOpenUrlW, InternetOpenW, InternetReadFile,
    INTERNET_FLAG_NO_CACHE_WRITE, INTERNET_FLAG_RELOAD, INTERNET_OPEN_TYPE_PRECONFIG,
};

/// 请求失败时的兜底：法定月计薪天数 `(365 - 104) / 12`。
pub const FALLBACK_WORKDAYS: f32 = 21.75;

const UA: &str = "dsh-pet-rs";

/// 一年的特殊日子：`"MM-DD" -> true 放假 / false 调休上班`。
pub struct Holidays {
    pub year: i32,
    map: HashMap<String, bool>,
}

pub fn cache_path(dir: &Path, year: i32) -> PathBuf {
    dir.join(format!("holidays-{year}.json"))
}

/// 读本地缓存。没有文件或解析失败都返回 None（不是错误，只是没得用）。
pub fn load(dir: &Path, year: i32) -> Option<Holidays> {
    let text = std::fs::read_to_string(cache_path(dir, year)).ok()?;
    parse(&text, year)
}

fn parse(body: &str, year: i32) -> Option<Holidays> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let mut map = HashMap::new();

    // 格式一 timor.tech：{"code":0,"holiday":{"01-01":{"holiday":true},...}}
    if let Some(obj) = v.get("holiday").and_then(|h| h.as_object()) {
        for (k, item) in obj {
            if k.len() != 5 {
                continue;
            }
            if let Some(off) = item.get("holiday").and_then(|h| h.as_bool()) {
                map.insert(k.clone(), off);
            }
        }
    }

    // 格式二 holiday-cn：{"year":2026,"days":[{"date":"2026-01-01","isOffDay":true},...]}
    if let Some(days) = v.get("days").and_then(|d| d.as_array()) {
        for d in days {
            let date = d.get("date").and_then(|x| x.as_str()).unwrap_or("");
            let off = d.get("isOffDay").and_then(|x| x.as_bool());
            if date.len() == 10 {
                if let Some(off) = off {
                    map.insert(date[5..].to_string(), off);
                }
            }
        }
    }

    if map.is_empty() {
        return None;
    }
    Some(Holidays { year, map })
}

/// 后台拉一次并写缓存。失败就算了，下次启动再试。
///
/// 放线程里是因为 WinInet 的阻塞式读取没有可靠的超时控制，
/// 网络卡住时不能把消息循环一起拖死。
///
/// 两个源：jsDelivr 上的 holiday-cn 只要 0.2 秒，timor.tech 要 8 秒左右，
/// 所以先试快的，不通再回落。
pub fn fetch_async(dir: PathBuf, year: i32) {
    std::thread::spawn(move || {
        for url in [
            format!("https://cdn.jsdelivr.net/gh/NateScarlet/holiday-cn@master/{year}.json"),
            format!("https://timor.tech/api/holiday/year/{year}"),
        ] {
            if let Some(body) = http_get(&url) {
                if parse(&body, year).is_some() {
                    let _ = std::fs::write(cache_path(&dir, year), body);
                    return;
                }
            }
        }
    });
}

fn http_get(url: &str) -> Option<String> {
    let to_wide = |s: &str| -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    };
    let agent = to_wide(UA);
    let url_w = to_wide(url);
    unsafe {
        let h = InternetOpenW(
            agent.as_ptr(),
            INTERNET_OPEN_TYPE_PRECONFIG,
            std::ptr::null(),
            std::ptr::null(),
            0,
        );
        if h.is_null() {
            return None;
        }
        let f = InternetOpenUrlW(
            h,
            url_w.as_ptr(),
            std::ptr::null(),
            0,
            INTERNET_FLAG_RELOAD | INTERNET_FLAG_NO_CACHE_WRITE,
            0,
        );
        if f.is_null() {
            InternetCloseHandle(h);
            return None;
        }
        let mut out: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let mut read: u32 = 0;
            let ok = InternetReadFile(
                f,
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
                &mut read,
            );
            if ok == 0 || read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
            if out.len() > 2 * 1024 * 1024 {
                break;
            }
        }
        InternetCloseHandle(f);
        InternetCloseHandle(h);
        if out.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&out).into_owned())
        }
    }
}

/// 当天的年月日与星期（0 = 周日）。
pub fn today() -> SYSTEMTIME {
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st);
    }
    st
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

impl Holidays {
    /// 当月实际工作日数：普通周末去掉，法定假日去掉，调休补班加回来。
    ///
    /// 星期以 `today` 为锚点反推，省掉一套日期库。
    pub fn workdays_in_month(&self, year: i32, month: u32, today: &SYSTEMTIME) -> u32 {
        if year != self.year || month == 0 || month > 12 {
            return 0;
        }
        let days = days_in_month(year, month);
        let day = today.wDay.clamp(1, 31) as i32;
        let wd = today.wDayOfWeek as i32; // 0 = 周日
        // 本月 1 号是星期几
        let first_wd = ((wd - ((day - 1) % 7)) % 7 + 7) % 7;

        let mut n = 0;
        for d in 1..=days {
            let w = ((first_wd + d as i32 - 1) % 7 + 7) % 7;
            let key = format!("{month:02}-{d:02}");
            let off = match self.map.get(&key) {
                Some(&holiday) => holiday,        // true = 放假
                None => w == 0 || w == 6,         // 普通周末
            };
            if !off {
                n += 1;
            }
        }
        n
    }

    /// 今天是否工作日。
    pub fn is_workday(&self, today: &SYSTEMTIME) -> bool {
        let key = format!("{:02}-{:02}", today.wMonth, today.wDay);
        match self.map.get(&key) {
            Some(&holiday) => !holiday,
            None => {
                let w = today.wDayOfWeek;
                w != 0 && w != 6
            }
        }
    }
}
