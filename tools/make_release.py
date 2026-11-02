# 创建 GitHub Release 并上传附件。
# git push 走不通（代理 502）时用，走 api.github.com / uploads.github.com。
# 用法：python tools/make_release.py <tag> <标题> <正文文件> <附件路径> [附件名]
import json
import os
import subprocess
import sys
import urllib.request

REPO = "xianren68/dsh-pet-rs"


def token():
    out = subprocess.run(
        ["git", "credential", "fill"],
        input="protocol=https\nhost=github.com\n\n",
        capture_output=True, text=True,
    ).stdout
    for line in out.splitlines():
        if line.startswith("password="):
            return line.split("=", 1)[1]
    sys.exit("拿不到 token")


TOKEN = token()
proxy = os.environ.get("http_proxy") or os.environ.get("https_proxy")
opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({"http": proxy, "https": proxy})
)
API = f"https://api.github.com/repos/{REPO}"


def call(method, url, payload=None):
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"token {TOKEN}")
    req.add_header("Accept", "application/vnd.github+json")
    if data:
        req.add_header("Content-Type", "application/json")
    with opener.open(req, timeout=90) as r:
        return json.loads(r.read())


tag = sys.argv[1]
title = sys.argv[2]
body_path = sys.argv[3]
asset = sys.argv[4] if len(sys.argv) > 4 else None
asset_name = sys.argv[5] if len(sys.argv) > 5 else (os.path.basename(asset) if asset else None)

with open(body_path, encoding="utf-8") as f:
    body = f.read()

rel = call(
    "POST",
    f"{API}/releases",
    {
        "tag_name": tag,
        "name": title,
        "body": body,
        "draft": False,
        "prerelease": False,
        # tag 不存在时按这个分支的 HEAD 建 tag
        "target_commitish": os.environ.get("RELEASE_TARGET", "main"),
    },
)
print(f"Release 已创建: {rel['html_url']}  (id={rel['id']})")

if asset:
    size = os.path.getsize(asset)
    print(f"上传附件 {asset_name} ({size / 1048576:.1f} MB)……")
    # 大文件交给 curl，urllib 会把整个文件读进内存
    url = f"https://uploads.github.com/repos/{REPO}/releases/{rel['id']}/assets?name={asset_name}"
    r = subprocess.run(
        [
            "curl", "-sS", "--max-time", "900",
            "-X", "POST",
            "-H", f"Authorization: token {TOKEN}",
            "-H", "Content-Type: application/zip",
            "--data-binary", f"@{asset}",
            url,
        ],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        print("上传失败:", r.stderr[:300])
        sys.exit(1)
    try:
        info = json.loads(r.stdout)
        print(f"附件已上传: {info.get('browser_download_url', r.stdout[:200])}")
    except json.JSONDecodeError:
        print("返回:", r.stdout[:300])
