# 当 git push 走不通时，用 GitHub Git Data API 直接把本地 commit 的内容推上去。
# 用法：python tools/push_via_api.py
import base64
import json
import os
import subprocess
import sys
import urllib.request

REPO = "xianren68/dsh-pet-rs"
BRANCH = os.environ.get("PUSH_BRANCH", "main")
# 不传文件名时，按 BASE..HEAD 的全量改动推。BASE 用上一轮的远程提交即可。
BASE = os.environ.get("PUSH_BASE", "ef03e50")
HEAD = subprocess.run(
    ["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True
).stdout.strip()


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
    with opener.open(req, timeout=60) as r:
        return json.loads(r.read())


def main():
    # 目标分支可能还不存在（比如新建的 win7-compat），先探一下
    try:
        remote = call("GET", f"{API}/git/ref/heads/{BRANCH}")
        remote_sha = remote["object"]["sha"]
        exists = True
    except Exception:
        base_ref = call("GET", f"{API}/git/ref/heads/main")
        remote_sha = base_ref["object"]["sha"]
        exists = False
        print(f"分支 {BRANCH} 不存在，将基于 main ({remote_sha[:7]}) 创建")
    print(f"远程 HEAD: {remote_sha[:7]}")

    # 命令行传了文件名就用它，否则按本地 commit 的全部改动推
    files = [a for a in sys.argv[1:] if a.strip()]
    if not files:
        files = subprocess.run(
            ["git", "diff", "--name-only", f"{BASE}..{HEAD}"],
            capture_output=True, text=True, check=True,
        ).stdout.split("\n")
    files = [f for f in files if f.strip()]
    print(f"待推送文件 {len(files)} 个")

    tree = []
    for path in files:
        blob = subprocess.run(
            ["git", "show", f"{HEAD}:{path}"], capture_output=True, check=True
        ).stdout
        r = call(
            "POST",
            f"{API}/git/blobs",
            {"content": base64.b64encode(blob).decode(), "encoding": "base64"},
        )
        tree.append(
            {"path": path, "mode": "100644", "type": "blob", "sha": r["sha"]}
        )
        print(f"  blob {path}")

    # 以远程当前 tree 为底，只覆盖改动的文件
    base_tree = call("GET", f"{API}/git/commits/{remote_sha}")["tree"]["sha"]
    new_tree = call(
        "POST", f"{API}/git/trees", {"base_tree": base_tree, "tree": tree}
    )
    msg = subprocess.run(
        ["git", "log", "-1", "--pretty=%B", HEAD], capture_output=True, text=True
    ).stdout
    commit = call(
        "POST",
        f"{API}/git/commits",
        {"message": msg, "tree": new_tree["sha"], "parents": [remote_sha]},
    )
    print(f"新 commit: {commit['sha'][:7]}")

    if exists:
        call("PATCH", f"{API}/git/refs/heads/{BRANCH}", {"sha": commit["sha"]})
    else:
        call(
            "POST",
            f"{API}/git/refs",
            {"ref": f"refs/heads/{BRANCH}", "sha": commit["sha"]},
        )
    print(f"已更新 {BRANCH} -> {commit['sha'][:7]}")


if __name__ == "__main__":
    main()
