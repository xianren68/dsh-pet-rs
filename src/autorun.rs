//! 开机自启：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。
//!
//! 用 HKCU 而不是 HKLM，不需要管理员权限，也不影响其他用户。
//! 值名固定 `dsh-pet-rs`，值为带引号的 exe 绝对路径。

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

use crate::util::to_wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "dsh-pet-rs";
const EXPECTED_TYPE: u32 = REG_SZ;

fn open_key(access: u32) -> Option<HKEY> {
    let sub = to_wide(RUN_KEY);
    let mut hkey: HKEY = null_mut();
    let rc = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            access,
            &mut hkey,
        )
    };
    if rc == ERROR_SUCCESS {
        Some(hkey)
    } else {
        None
    }
}

/// 当前是否已登记自启（只看值存不存在，不校验路径是否过期）。
pub fn is_enabled() -> bool {
    let Some(hkey) = open_key(KEY_QUERY_VALUE) else {
        return false;
    };
    let name = to_wide(VALUE_NAME);
    let mut ty: u32 = 0;
    let mut size: u32 = 0;
    let rc = unsafe {
        RegQueryValueExW(
            hkey,
            name.as_ptr(),
            null(),
            &mut ty,
            null_mut(),
            &mut size,
        )
    };
    unsafe { RegCloseKey(hkey) };
    rc != ERROR_FILE_NOT_FOUND && rc == ERROR_SUCCESS
}

/// 开 / 关自启。
pub fn set_enabled(on: bool) -> Result<(), String> {
    let Some(hkey) = open_key(KEY_SET_VALUE) else {
        return Err("打不开注册表 Run 项".into());
    };
    let name = to_wide(VALUE_NAME);

    let rc = if on {
        let exe = std::env::current_exe().map_err(|e| format!("取 exe 路径失败: {e}"))?;
        // 带引号，路径有空格也不会被拆开
        let cmd = format!("\"{}\"", exe.display());
        let data = to_wide(&cmd);
        let bytes = (data.len() * 2) as u32;
        unsafe {
            RegSetValueExW(
                hkey,
                name.as_ptr(),
                0,
                EXPECTED_TYPE,
                data.as_ptr() as *const u8,
                bytes,
            )
        }
    } else {
        unsafe { RegDeleteValueW(hkey, name.as_ptr()) }
    };

    unsafe { RegCloseKey(hkey) };

    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("注册表写入失败，错误码 {rc}"))
    }
}

/// 读一下当前登记的命令行（调试用）。
#[allow(dead_code)]
pub fn current_command() -> Option<String> {
    let hkey = open_key(KEY_QUERY_VALUE)?;
    let name = to_wide(VALUE_NAME);
    let mut buf = vec![0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    let mut ty: u32 = 0;
    let rc = unsafe {
        RegQueryValueExW(
            hkey,
            name.as_ptr(),
            null(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        )
    };
    unsafe { RegCloseKey(hkey) };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let n = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n]))
}
