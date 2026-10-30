//! 一次性小工具：alpha 侵蚀，用来削掉 AI 生成素材外圈发虚的白边。
//!
//! 用法：`cargo run --release --example edgefix -- <输入.png> <输出.png> [半径=1]`
//!
//! 原理：对每个像素取 (2r+1)² 邻域里 alpha 的**最小值**。边缘会整圈向内收 r 像素，
//! 外圈那些半透明发白的雾就被吃掉了。RGB 不动 —— 只收形状，不改颜色。

use image::RgbaImage;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("用法: edgefix <in.png> <out.png> [radius]");
        std::process::exit(2);
    }
    let radius: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let radius = radius.clamp(0, 8);

    let img = match image::open(&args[0]) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            eprintln!("读不了 {}: {e}", args[0]);
            std::process::exit(1);
        }
    };

    let out = erode_alpha(&img, radius);
    if let Err(e) = out.save(&args[1]) {
        eprintln!("写不了 {}: {e}", args[1]);
        std::process::exit(1);
    }
}

fn erode_alpha(src: &RgbaImage, radius: i32) -> RgbaImage {
    let (w, h) = src.dimensions();
    let mut dst = src.clone();
    if radius <= 0 {
        return dst;
    }
    let get = |x: i64, y: i64| -> u8 {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            0 // 画布外视为全透明，等价于贴边也收
        } else {
            src.get_pixel(x as u32, y as u32)[3]
        }
    };

    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut mn = 255u8;
            for dy in -radius as i64..=radius as i64 {
                for dx in -radius as i64..=radius as i64 {
                    mn = mn.min(get(x + dx, y + dy));
                }
            }
            if mn != src.get_pixel(x as u32, y as u32)[3] {
                dst.get_pixel_mut(x as u32, y as u32)[3] = mn;
            }
        }
    }
    dst
}
