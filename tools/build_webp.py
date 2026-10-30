# -*- coding: utf-8 -*-
"""完整管线: mp4 -> 逐帧 HSV 抠像 -> 640x360 -> 12fps 动画 WebP。"""
import os
import subprocess
import sys
import shutil

import numpy as np
from PIL import Image

FF = r"C:\project\dsh-pet-rs\tools\ffmpeg-9.0.2-essentials_build\bin\ffmpeg.exe"
WORK = r"C:\project\dsh-pet-rs\tools"


def cut_frame(rgb: np.ndarray) -> np.ndarray:
    """HSV 色相判定抠像（与原项目 chroma_step02.py 同算法）。
    只把"绿色色相 + 足够饱和/明度"判为背景透明；白色/深蓝/粉色永不误抠，
    从根上避免 greenness 阈值法产生的灰白锯齿边。"""
    r = rgb[..., 0].astype(np.float32) / 255.0
    g = rgb[..., 1].astype(np.float32) / 255.0
    b = rgb[..., 2].astype(np.float32) / 255.0
    mx = np.maximum(np.maximum(r, g), b)
    mn = np.minimum(np.minimum(r, g), b)
    delta = mx - mn
    nz = delta > 1e-6
    with np.errstate(divide="ignore", invalid="ignore"):
        hr = 60.0 * (((g - b) / np.where(nz, delta, 1)) % 6.0)
        hg = 60.0 * (((b - r) / np.where(nz, delta, 1)) + 2.0)
        hb = 60.0 * (((r - g) / np.where(nz, delta, 1)) + 4.0)
    hue = np.where((mx == r) & nz, hr, np.where((mx == g) & nz, hg, np.where((mx == b) & nz, hb, 0.0)))
    sat = np.where(mx > 0, delta / np.maximum(mx, 1e-6), 0.0)

    # 原项目参数微调: 色相[66,174], 饱和度>=0.18, 明度>=0.15, 边界 feather 4°
    lo, hi = 70.0 - 4.0, 170.0 + 4.0
    in_hue = np.clip((hue - lo) / 4.0, 0, 1) * np.clip((hi - hue) / 4.0, 0, 1)
    bg_mask = (sat >= 0.18) & (mx >= 0.15) & (hue >= lo) & (hue <= hi)
    # 色相中心全透明, 边界渐变, 非背景 255
    alpha = np.where(bg_mask, (1.0 - in_hue) * 255.0, 255.0)
    # despill: 对半透明边缘像素, 把残留绿色分量压回 (R+B)/2, 根除淡绿描边
    edge = (alpha > 1.0) & (alpha < 254.0)
    avg_rb = (r + b) / 2.0
    g = np.where(edge & (g > avg_rb), avg_rb, g)
    rgba = np.dstack([r * 255.0, g * 255.0, b * 255.0, alpha]).astype(np.uint8)
    return rgba


def main():
    src_mp4, out_webp = sys.argv[1], sys.argv[2]
    frames_dir = os.path.join(WORK, "_frames_tmp")
    if os.path.exists(frames_dir):
        shutil.rmtree(frames_dir)
    os.makedirs(frames_dir)

    # 1) 解帧 (24fps 全帧)
    subprocess.run([FF, "-y", "-i", src_mp4, os.path.join(frames_dir, "f%04d.png")],
                   check=True, capture_output=True)
    files = sorted(f for f in os.listdir(frames_dir) if f.endswith(".png"))
    print(f"decoded {len(files)} frames")

    # 2) 逐帧抠像 + 缩放, 每2帧取1帧 -> 12fps
    frames = []
    for i, name in enumerate(files):
        if i % 2 != 0:
            continue
        im = Image.open(os.path.join(frames_dir, name)).convert("RGB")
        rgba = cut_frame(np.array(im))
        pil = Image.fromarray(rgba, "RGBA").resize((640, 360), Image.LANCZOS)
        frames.append(pil)
    print(f"kept {len(frames)} frames @12fps")

    # 3) 编码动画 webp
    frames[0].save(
        out_webp,
        save_all=True,
        append_images=frames[1:],
        duration=83,           # 1000/12
        loop=0,
        quality=80,
        lossless=False,
        method=6,
    )
    shutil.rmtree(frames_dir)
    print("saved", out_webp, os.path.getsize(out_webp), "bytes")


if __name__ == "__main__":
    main()
