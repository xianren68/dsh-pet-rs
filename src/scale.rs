//! 位图缩放（双线性，alpha 预乘处理）。
//!
//! 素材是 640×360，用户改大小时要按目标尺寸重采样。只在缩放比 ≠ 1 时调用，
//! 默认尺寸下零开销。
//!
//! 关键：**先预乘 alpha 再插值**。非预乘的 RGBA 直接插值，透明边缘会渗出
//! 黑色（透明像素的 RGB 是 0），人物轮廓会黑一圈。

/// 缩放一帧 BGRA（`0xAARRGGBB`）到目标尺寸。
pub fn scale(src: &[u32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u32> {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return vec![0; dw * dh];
    }
    if sw == dw && sh == dh {
        return src.to_vec();
    }

    let mut out = vec![0u32; dw * dh];
    let xr = sw as f32 / dw as f32;
    let yr = sh as f32 / dh as f32;

    for y in 0..dh {
        // +0.5 取像素中心，避免整体偏移半像素
        let sy = (y as f32 + 0.5) * yr - 0.5;
        let y0 = sy.floor();
        let fy = (sy - y0).clamp(0.0, 1.0);
        let y0i = y0.clamp(0.0, (sh - 1) as f32) as usize;
        let y1i = (y0i + 1).min(sh - 1);

        for x in 0..dw {
            let sx = (x as f32 + 0.5) * xr - 0.5;
            let x0 = sx.floor();
            let fx = (sx - x0).clamp(0.0, 1.0);
            let x0i = x0.clamp(0.0, (sw - 1) as f32) as usize;
            let x1i = (x0i + 1).min(sw - 1);

            let p00 = src[y0i * sw + x0i];
            let p10 = src[y0i * sw + x1i];
            let p01 = src[y1i * sw + x0i];
            let p11 = src[y1i * sw + x1i];

            out[y * dw + x] = mix4(p00, p10, p01, p11, fx, fy);
        }
    }
    out
}

#[inline]
fn mix4(p00: u32, p10: u32, p01: u32, p11: u32, fx: f32, fy: f32) -> u32 {
    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;

    let a00 = ((p00 >> 24) & 0xff) as f32;
    let a10 = ((p10 >> 24) & 0xff) as f32;
    let a01 = ((p01 >> 24) & 0xff) as f32;
    let a11 = ((p11 >> 24) & 0xff) as f32;
    let a = a00 * w00 + a10 * w10 + a01 * w01 + a11 * w11;

    if a < 0.5 {
        return 0;
    }

    // 预乘后插值，再反预乘
    let ch = |shift: u32| -> f32 {
        let c00 = ((p00 >> shift) & 0xff) as f32 * a00;
        let c10 = ((p10 >> shift) & 0xff) as f32 * a10;
        let c01 = ((p01 >> shift) & 0xff) as f32 * a01;
        let c11 = ((p11 >> shift) & 0xff) as f32 * a11;
        (c00 * w00 + c10 * w10 + c01 * w01 + c11 * w11) / a
    };

    let r = ch(16).clamp(0.0, 255.0) as u32;
    let g = ch(8).clamp(0.0, 255.0) as u32;
    let b = ch(0).clamp(0.0, 255.0) as u32;
    let a = a.clamp(0.0, 255.0) as u32;

    (a << 24) | (r << 16) | (g << 8) | b
}
