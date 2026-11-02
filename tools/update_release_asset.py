# 替换已发布 Release 的附件（GitHub 不允许覆盖同名文件，得先删再传）。
# 用法：python tools/update_release_asset.py <tag> <附件路径> [附件名]
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
        body = r.read()
        return json.loads(body) if body else {}


tag = sys.argv[1]
path = sys.argv[2]
name = sys.argv[3] if len(sys.argv) > 3 else os.path.basename(path)

rel = call("GET", f"{API}/releases/tags/{tag}")
print(f"Release {tag} (id={rel['id']})")

for a in rel.get("assets", []):
    if a["name"] == name:
        call("DELETE", f"{API}/releases/assets/{a['id']}")
        print(f"  已删除旧附件 {a['name']}")

size = os.path.getsize(path)
print(f"上传 {name} ({size / 1048576:.1f} MB)……")
url = f"https://uploads.github.com/repos/{REPO}/releases/{rel['id']}/assets?name={name}"
r = subprocess.run(
    ["curl", "-sS", "--max-time", "1200", "-X", "POST",
     "-H", f"Authorization: token {TOKEN}",
     "-H", "Content-Type: application/zip",
     "--data-binary", f"@{path}", url],
    capture_output=True, text=True,
)
if r.returncode != 0:
    print("上传失败:", r.stderr[:300])
    sys.exit(1)
try:
    info = json.loads(r.stdout)
    print(f"  已上传: {info.get('browser_download_url', '')}")
except json.JSONDecodeError:
    print("返回:", r.stdout[:300])
