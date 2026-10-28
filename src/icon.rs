//! 托盘图标：从待机动画第一帧生成 `.ico`。
//!
//! 第一次运行时生成并缓存到 `assets/icon.ico`，之后直接读文件。
//! 用「PNG 塞进 ICO 容器」的写法——Vista 以后都认，省掉 BMP + mask 那套。

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

use crate::clip::Stream;
use crate::scale::scale;

const SIZE: usize = 32;

pub fn ensure_icon(assets: &Path, cfg: &Path) -> Option<PathBuf> {
    let ico = cfg.join("icon.ico");
    if ico.is_file() {
        return Some(ico);
    }
    let src = match pick_source(assets) {
        Some(p) => p,
        None => {
            eprintln!("[icon] 找不到素材");
            return None;
        }
    };
    eprintln!("[icon] 源 {}", src.display());
    let mut s = match Stream::open(&src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[icon] 打开失败 {e}");
            return None;
        }
    };
    let (sw, sh) = (s.w as usize, s.h as usize);
    let frame = match s.next_frame() {
        Some(f) => f,
        None => {
            eprintln!("[icon] 取帧失败");
            return None;
        }
    };
    if sw == 0 || sh == 0 {
        return None;
    }

    // 640×360 直接压成方形会把人压扁，先按人物 alpha 包围盒裁成正方形再缩
    let (bx0, by0, bx1, by1) = alpha_bbox(&frame, sw, sh);
    let side = (bx1.saturating_sub(bx0)).max(by1.saturating_sub(by0)).max(8);
    let cx = (bx0 + bx1) / 2;
    let cy = (by0 + by1) / 2;
    let ox = cx.saturating_sub(side / 2);
    let oy = cy.saturating_sub(side / 2);

    let mut sq = vec![0u32; side * side];
    for y in 0..side {
        for x in 0..side {
            let sx = ox + x;
            let sy = oy + y;
            if sx < sw && sy < sh {
                sq[y * side + x] = frame[sy * sw + sx];
            }
        }
    }
    let small = scale(&sq, side, side, SIZE, SIZE);

    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    for p in &small {
        // 存的是小端 0xAARRGGBB，展开成 R,G,B,A
        rgba.push((p >> 16) as u8);
        rgba.push((p >> 8) as u8);
        rgba.push(*p as u8);
        rgba.push((p >> 24) as u8);
    }

    let mut png = Vec::new();
    {
        let enc = PngEncoder::new(&mut png);
        enc.write_image(&rgba, SIZE as u32, SIZE as u32, ExtendedColorType::Rgba8)
            .ok()?;
    }

    // ICONDIR + ICONDIRENTRY + PNG
    let mut out = Vec::with_capacity(png.len() + 22);
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    out.extend_from_slice(&1u16.to_le_bytes()); // count
    out.push(SIZE as u8); // width
    out.push(SIZE as u8); // height
    out.push(0); // color count
    out.push(0); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&(png.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes()); // offset
    out.extend_from_slice(&png);

    match std::fs::write(&ico, &out) {
        Ok(_) => eprintln!("[icon] 写入 {} ({} 字节)", ico.display(), out.len()),
        Err(e) => eprintln!("[icon] 写入失败 {e}"),
    }
    Some(ico)
}

/// 优先用待机那张，没有就随便挑一张。
fn pick_source(dir: &Path) -> Option<PathBuf> {
    let anims = dir.join("anims");
    let base = if anims.is_dir() { anims } else { dir.to_path_buf() };
    for name in ["待机呼吸休闲", "点击回应-元气挥手"] {
        let p = base.join(format!("{name}.webp"));
        if p.is_file() {
            return Some(p);
        }
    }
    let rd = std::fs::read_dir(&base).ok()?;
    rd.flatten()
        .map(|e| e.path())
        .find(|p| {
            matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("webp") | Some("apng") | Some("gif")
            )
        })
}

/// 把 ICO 文件读成 HICON。
pub fn load_hicon(path: &Path) -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON> {
    use std::ptr::null_mut;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        LoadImageW, IMAGE_ICON, LR_LOADFROMFILE,
    };

    let wide = crate::util::to_wide(&path.to_string_lossy());
    let h = unsafe {
        LoadImageW(
            null_mut(),
            wide.as_ptr(),
            IMAGE_ICON,
            SIZE as i32,
            SIZE as i32,
            LR_LOADFROMFILE,
        )
    };
    if h.is_null() {
        None
    } else {
        Some(h as _)
    }
}

#[allow(dead_code)]
fn _unused(_: Cursor<Vec<u8>>) {}


/// 找出 alpha > 8 的像素范围（人物实际占的区域）。
fn alpha_bbox(px: &[u32], w: usize, h: usize) -> (usize, usize, usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if (px[y * w + x] >> 24) > 8 {
                if x < x0 {
                    x0 = x;
                }
                if y < y0 {
                    y0 = y;
                }
                if x + 1 > x1 {
                    x1 = x + 1;
                }
                if y + 1 > y1 {
                    y1 = y + 1;
                }
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        (0, 0, w, h)
    } else {
        (x0, y0, x1, y1)
    }
}
