//! 动画片段：元数据探测 + 按需流式解码。
//!
//! 关键取舍：**不预解码全部帧**。一段 640×360 / 12fps / 10s 的动画全量解码要
//! 110MB 内存，而桌宠常年只播一条。所以这里只探测时长表（结果缓存在
//! `.clipcache`），像素按帧现解现用，常驻内存降到两帧。
//!
//! 三种素材格式：
//! - `.webp` 动画 —— 首选。同样 RGBA，体积约为 APNG 的 1/6
//! - `.apng` —— 无损，体积大
//! - `.gif`  —— 只有 1-bit alpha，边缘会毛，兼容用

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::{AnimationDecoder, Frames, ImageDecoder};
use image_webp::WebPDecoder;

#[allow(dead_code)]
pub struct Clip {
    pub name: String,
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    /// 每帧停留毫秒
    pub delays: Vec<u32>,
}

impl Clip {
    /// 只读元数据，不保留像素。
    pub fn probe(path: &Path, name: &str) -> Result<Clip, String> {
        let (w, h, delays) = match read_cache(path) {
            Some(v) => v,
            None => {
                let v = scan(path)?;
                write_cache(path, &v);
                v
            }
        };
        if delays.is_empty() {
            return Err(format!("{}: 没有任何帧", path.display()));
        }
        Ok(Clip {
            name: name.to_string(),
            path: path.to_path_buf(),
            w,
            h,
            delays,
        })
    }

    pub fn total_ms(&self) -> u32 {
        self.delays.iter().sum()
    }
}

/// 从头解一遍拿尺寸和每帧延时，像素用完即弃。
fn scan(path: &Path) -> Result<(u32, u32, Vec<u32>), String> {
    let f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let r = BufReader::new(f);
    let ext = ext_of(path);

    let mut delays = Vec::new();
    let (w, h);

    if ext == "webp" {
        let mut dec = WebPDecoder::new(r).map_err(|e| format!("webp: {e}"))?;
        (w, h) = dec.dimensions();
        let n = dec.num_frames();
        let size = dec.output_buffer_size().unwrap_or((w * h * 4) as usize);
        let mut buf = vec![0u8; size];
        for _ in 0..n {
            match dec.read_frame(&mut buf) {
                Ok(ms) => delays.push(normalize(ms)),
                Err(_) => break,
            }
        }
    } else if ext == "gif" {
        let dec = GifDecoder::new(r).map_err(|e| format!("gif: {e}"))?;
        (w, h) = dec.dimensions();
        for fr in dec.into_frames() {
            let fr = fr.map_err(|e| format!("gif frame: {e}"))?;
            delays.push(ratio_ms(fr.delay().numer_denom_ms()));
        }
    } else {
        let dec = PngDecoder::new(r).map_err(|e| format!("png: {e}"))?;
        (w, h) = dec.dimensions();
        let apng = dec.apng().map_err(|e| format!("apng: {e}"))?;
        for fr in apng.into_frames() {
            let fr = fr.map_err(|e| format!("apng frame: {e}"))?;
            delays.push(ratio_ms(fr.delay().numer_denom_ms()));
        }
    }

    Ok((w, h, delays))
}

fn ratio_ms((num, den): (u32, u32)) -> u32 {
    let ms = if den == 0 { 0 } else { num / den };
    normalize(ms)
}

fn normalize(ms: u32) -> u32 {
    if ms == 0 || ms > 1000 {
        80
    } else {
        ms
    }
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

// ---- 元数据缓存 ----
// 一行：`<w> <h> <d1,d2,d3...>`；素材比缓存新就失效重扫。

fn cache_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".clipcache");
    PathBuf::from(s)
}

fn read_cache(p: &Path) -> Option<(u32, u32, Vec<u32>)> {
    let meta = std::fs::metadata(p).ok()?;
    let cache = cache_path(p);
    let cm = std::fs::metadata(&cache).ok()?;
    if cm.modified().ok()? < meta.modified().ok()? {
        return None;
    }
    let text = std::fs::read_to_string(&cache).ok()?;
    let mut it = text.split_whitespace();
    let w: u32 = it.next()?.parse().ok()?;
    let h: u32 = it.next()?.parse().ok()?;
    let delays: Vec<u32> = it
        .next()?
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect();
    if delays.is_empty() {
        None
    } else {
        Some((w, h, delays))
    }
}

fn write_cache(p: &Path, v: &(u32, u32, Vec<u32>)) {
    let body = format!(
        "{} {} {}\n",
        v.0,
        v.1,
        v.2.iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let _ = std::fs::write(cache_path(p), body);
}

// ---- 流式解码 ----

pub struct Stream {
    inner: Inner,
    buf: Vec<u8>,
    alpha: bool,
    pub w: u32,
    pub h: u32,
}

enum Inner {
    Png(Frames<'static>),
    Gif(Frames<'static>),
    Webp(Box<WebPDecoder<BufReader<File>>>),
}

impl Stream {
    pub fn open(path: &Path) -> Result<Stream, String> {
        let f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let r = BufReader::new(f);
        let ext = ext_of(path);

        if ext == "webp" {
            let dec = WebPDecoder::new(r).map_err(|e| format!("webp: {e}"))?;
            let alpha = dec.has_alpha();
            let (w, h) = dec.dimensions();
            let size = dec
                .output_buffer_size()
                .unwrap_or((w * h * 4) as usize);
            Ok(Stream {
                inner: Inner::Webp(Box::new(dec)),
                buf: vec![0u8; size],
                alpha,
                w,
                h,
            })
        } else if ext == "gif" {
            let dec = GifDecoder::new(r).map_err(|e| format!("gif: {e}"))?;
            let (w, h) = dec.dimensions();
            Ok(Stream {
                inner: Inner::Gif(dec.into_frames()),
                buf: Vec::new(),
                alpha: true,
                w,
                h,
            })
        } else {
            let dec = PngDecoder::new(r).map_err(|e| format!("png: {e}"))?;
            let (w, h) = dec.dimensions();
            let apng = dec.apng().map_err(|e| format!("apng: {e}"))?;
            Ok(Stream {
                inner: Inner::Png(apng.into_frames()),
                buf: Vec::new(),
                alpha: true,
                w,
                h,
            })
        }
    }

    /// 下一帧；到结尾返回 None。
    pub fn next_frame(&mut self) -> Option<Vec<u32>> {
        let alpha = self.alpha;
        match (&mut self.inner, &mut self.buf) {
            (Inner::Webp(d), buf) => {
                d.read_frame(buf).ok()?;
                Some(webp_buf_to_bgra(buf, alpha))
            }
            (Inner::Png(f), _) => {
                let fr = f.next()?.ok()?;
                Some(rgba_to_bgra(fr.buffer()))
            }
            (Inner::Gif(f), _) => {
                let fr = f.next()?.ok()?;
                Some(rgba_to_bgra(fr.buffer()))
            }
        }
    }
}

fn webp_buf_to_bgra(buf: &[u8], alpha: bool) -> Vec<u32> {
    if alpha {
        buf.chunks_exact(4)
            .map(|p| u32::from_le_bytes([p[2], p[1], p[0], p[3]]))
            .collect()
    } else {
        buf.chunks_exact(3)
            .map(|p| u32::from_le_bytes([p[2], p[1], p[0], 255]))
            .collect()
    }
}

fn rgba_to_bgra(img: &image::RgbaImage) -> Vec<u32> {
    let mut out = Vec::with_capacity(img.len() / 4);
    for p in img.as_raw().chunks_exact(4) {
        out.push(u32::from_le_bytes([p[2], p[1], p[0], p[3]]));
    }
    out
}
