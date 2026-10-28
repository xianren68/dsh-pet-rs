# 把 dsh-pet 的透明 webm 批量转成动画 WebP（体积约为 APNG 的 1/6）。
# 用法：
#   python tools\batch-webp.py <webm目录> <输出目录> [并行数] [fps] [质量]
# 需要 ffmpeg：http://www.gyan.dev/ffmpeg/builds/ 或 pip install imageio-ffmpeg

import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed

FF_CANDIDATES = [
    "ffmpeg",
    r"C:\01\Python\Python312\Lib\site-packages\imageio_ffmpeg\binaries\ffmpeg-win-x86_64-v7.1.exe",
]


def find_ffmpeg() -> str:
    for c in FF_CANDIDATES:
        try:
            subprocess.run([c, "-version"], capture_output=True, check=True)
            return c
        except Exception:
            continue
    sys.exit("ffmpeg not found")


def convert(ff, src, out, fps, quality):
    cmd = [
        ff, "-y", "-hide_banner", "-loglevel", "error",
        # 必须显式指定 vp9 解码器，否则 alpha 会被当普通 yuv420p 丢掉
        "-c:v", "libvpx-vp9", "-i", src,
        "-vf", f"fps={fps},format=rgba",
        "-c:v", "libwebp_anim", "-lossless", "0",
        "-quality", str(quality), "-loop", "0",
        out,
    ]
    r = subprocess.run(cmd, capture_output=True)
    if r.returncode != 0:
        return False, r.stderr.decode("utf-8", "ignore")[:160]
    return True, os.path.getsize(out)


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    src_dir, out_dir = sys.argv[1], sys.argv[2]
    workers = int(sys.argv[3]) if len(sys.argv) > 3 else 4
    fps = int(sys.argv[4]) if len(sys.argv) > 4 else 12
    quality = int(sys.argv[5]) if len(sys.argv) > 5 else 80

    os.makedirs(out_dir, exist_ok=True)
    ff = find_ffmpeg()
    files = [f for f in sorted(os.listdir(src_dir)) if f.lower().endswith(".webm")]
    todo = []
    for f in files:
        stem = os.path.splitext(f)[0]
        out = os.path.join(out_dir, stem + ".webp")
        if os.path.exists(out) and os.path.getsize(out) > 0:
            continue
        todo.append((f, out))

    print(f"total={len(files)} todo={len(todo)} workers={workers} fps={fps} q={quality}", flush=True)
    total_bytes = 0
    failed = []
    with ThreadPoolExecutor(max_workers=workers) as ex:
        futs = {
            ex.submit(convert, ff, os.path.join(src_dir, f), out, fps, quality): f
            for f, out in todo
        }
        done = 0
        for fut in as_completed(futs):
            name = futs[fut]
            ok, info = fut.result()
            done += 1
            if ok:
                total_bytes += info
                print(f"[{done}/{len(todo)}] {name} -> {info // 1024} KB", flush=True)
            else:
                failed.append((name, info))
                print(f"[{done}/{len(todo)}] FAIL {name}: {info}", flush=True)

    print(f"\nok={len(todo) - len(failed)} fail={len(failed)} "
          f"total={total_bytes / 1024 / 1024:.0f} MB", flush=True)
    for n, e in failed:
        print("FAILED", n, e)


if __name__ == "__main__":
    main()
