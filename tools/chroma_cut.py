# -*- coding: utf-8 -*-
"""绿幕抠像：浅草绿背景 -> 透明 alpha。用法:
  python chroma_cut.py <输入图> <输出图>            # 单帧调试
  python chroma_cut.py <输入目录帧序列> ...         # 批量由 build_webp.py 调用
判定逻辑: G 明显高于 R/B => 绿幕; 按 (G-max(R,B)) 的差值做 alpha 软过渡。
"""
import sys
import numpy as np
from PIL import Image


def cut_frame(rgb: np.ndarray) -> np.ndarray:
    """输入 HxWx3 uint8 RGB, 输出 HxWx4 uint8 RGBA。"""
    r = rgb[:, :, 0].astype(np.float32)
    g = rgb[:, :, 1].astype(np.float32)
    b = rgb[:, :, 2].astype(np.float32)
    mx = np.maximum(r, b)
    # 绿色主导强度: G 比 R/B 高出多少
    greenness = g - mx
    # 纯绿幕区 greenness > ~40 => alpha=0; 边缘渐变区 10~40 => 半透明
    hard = 45.0    # 高于此值完全透明
    soft = 12.0     # 低于此值完全不透明
    alpha = np.clip((hard - greenness) / (hard - soft) * 255.0, 0, 255)
    alpha = alpha.astype(np.uint8)
    out = np.dstack([rgb, alpha])
    return out


def main():
    src, dst = sys.argv[1], sys.argv[2]
    im = Image.open(src).convert("RGB")
    arr = np.array(im)
    out = cut_frame(arr)
    Image.fromarray(out, "RGBA").save(dst)
    print("saved", dst)


if __name__ == "__main__":
    main()
