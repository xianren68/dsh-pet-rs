# dsh-pet-rs

一个极简的 Windows 桌宠：透明置顶、106 段动作、能拖能甩，还会看电量、网络、下载、空闲、锁屏和全屏。单文件约 1 MB，环境反应和问候都由 `assets/events.json` 驱动（全屏隐藏除外），改配置不用重新编译。

> ## 先看原项目：[PC2005-cloud/dsh-pet](https://github.com/PC2005-cloud/dsh-pet)
>
> 这个项目是它的**减法版**，素材、动作分类、行为权重全部来自那边。
>
> 原项目功能完整得多——AI 对话、余额提醒、碎碎念、跟着工作状态换表情，
> 而且仓库里有一整套 **桌宠动作制作教程**（怎么画、怎么导出带透明通道的视频、
> 怎么配权重分类），想自己做动作的话那份教程是绕不开的。
>
> 想要完整体验、或者想做自己的桌宠动作，**直接去用原项目**。
> 我这里只是把"桌宠"最核心的部分用最少的依赖重写了一遍。

## 该用哪个

| | [dsh-pet](https://github.com/PC2005-cloud/dsh-pet)（原项目） | dsh-pet-rs（这个） |
|---|---|---|
| 装好就能用 | 要 Node / Electron 环境 | 解压双击 |
| 内存占用 | 300 MB 左右 | 22 MB |
| AI 对话 | ✅ | ❌ |
| 余额提醒 / 碎碎念 | ✅ | ❌ |
| 工作状态联动 | ✅ | ❌ |
| 动作制作教程 | ✅ | ❌ |
| 动作数量 | 106 + 社区 | 106（同一套） |
| 本地环境感知 | ❌ | ✅ 电量 / 网络 / 下载 / 空闲 / 锁屏 / 全屏 |

**要会聊天、会提醒、会跟着你状态变表情的宠物 → 用原项目。**
**只想桌面上有个小人动一动、不吃内存 → 用这个。**

两个项目共用同一套美术资源，版权都归原作者（见文末）。

## 下载

不想折腾编译和素材转换的，直接下完整包（含 106 段动作，解压双击即用）：

**[dsh-pet-rs-v0.2.0.zip](https://github.com/xianren68/dsh-pet-rs/releases/latest)** · 238 MB

只想看代码的话 clone 仓库就行，素材按下面「素材」一节自己转一次。

## 技术选型

没上 Electron（原项目的选择），也没上 Tauri，理由很简单：桌宠需要的能力只有四样——
透明窗、置顶、像素级命中、播动画，Win32 原生全都有，不必为此带一个浏览器内核。

代价是一个无法回避的取舍：

> **GPU 渲染和系统级逐像素穿透不能同时要。**
> `WS_EX_TRANSPARENT` 只能整窗穿透；`AC_SRC_ALPHA` 的逐像素穿透必须走
> `UpdateLayeredWindow` 上传 BGRA 位图，而那是 GDI，拿不到 GPU 上下文。

这里选了后者。好处是穿透由合成器保证——alpha = 0 的像素天然不吃鼠标，
不用额外维护光标轮询和输入状态机。坏处是没有硬件加速，1080p 以上的高帧率素材会吃力
（640×360 完全没问题，这也是这套素材的原生尺寸）。

原项目走的是另一条路：Chromium 负责渲染，穿透靠"整窗开关 + 定时轮询光标位置"模拟。
两条路各有各的麻烦，不存在谁更对。

## 性能

实测（2560×1440 / 125% 缩放，播放 640×360 待机动画，稳定运行后取值）：

| | 数值 |
|---|---|
| 常驻内存 | 22 MB（640 宽）/ 27.5 MB（820 宽） |
| 单核 CPU | 1.8% |
| 进程数 | 1 |
| exe | ≈1 MB（1007 KB） |
| 素材 | 106 段，238 MB（见下方说明） |

调大尺寸时每帧多一次重采样（820 宽约 5 ms/帧），仍在可控范围。

## 核心机制

| 需求 | 做法 |
|---|---|
| 透明置顶 | `WS_EX_LAYERED \| WS_EX_TOPMOST \| WS_EX_TOOLWINDOW`，`WS_POPUP` |
| 逐像素穿透 | `UpdateLayeredWindow(.., blend{AlphaFormat: AC_SRC_ALPHA}, ULW_ALPHA)` |
| 不抢焦点 | `WM_MOUSEACTIVATE` 返回 `MA_NOACTIVATE`，`ShowWindow(SW_SHOWNOACTIVATE)` |
| 移动 + 换帧 | 同一次 `UpdateLayeredWindow` 的 `pptDst` 完成，不会撕裂 |
| 素材 | 构建期 webm → 动画 WebP，运行期 `image-webp` 解码，无视频依赖 |
| 内存 | 不预解码全帧，只缓存每帧时长表（`.clipcache`），像素现解现用 |
| 缩放 | 双线性重采样（先预乘 alpha，否则透明边缘会渗黑） |
| 朝向 | **素材原生朝左**；朝右时整帧水平镜像（写 DIB 时逐行反转）。搞反会「倒着走」+ 文字反字 |
| 环境事件 | 轮询电量 / 网络 / 下载 / 空闲 / 全屏，接收锁屏 / 恢复消息；阈值、动作、文案全部读 `assets/events.json` |
| 气泡 | 独立的 `WS_EX_LAYERED \| WS_EX_TRANSPARENT` 窗口，GDI 画完再 `UpdateLayeredWindow`，不抢焦点、不吃鼠标 |

`AC_SRC_ALPHA` 是关键一行。加上它，屏幕上一块 640×360 的窗口只有宠物身体接收鼠标，
其余区域直接落到下层窗口——桌面图标、浏览器、任何东西都能正常点。

## 设置面板

托盘**双击**、托盘右键「设置…」、宠物右键「设置…」，或者 `dsh-pet-rs.exe --settings`。

面板本身是普通的 Win32 窗口 + 系统原生控件（`STATIC` / `BUTTON` / `CHECKBOX` /
`msctls_trackbar32`），不做自绘。除了大小、透明度、行为倾向，还多了环境感知和气泡开关：

| 项 | 作用 | 落地 |
|---|---|---|
| 宠物大小 | 200–900 px，16:9 等比，拖动即时生效 | `settings.json: width` |
| 不透明度 | 40–255，改 `UpdateLayeredWindow` 的 `SourceConstantAlpha` | `settings.json: alpha` |
| 行为倾向 | 待机 / 转向 / 走动三档相对权重；剩下的份额给随机小动作 | `settings.json: w_idle/w_turn/w_move` |
| 原地待机 | 勾上后不自己走、不自己转朝向（等价 `physics.fixed`） | `settings.json: fixed` |
| 开机自动启动 | 写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | 注册表，值名 `dsh-pet-rs` |
| 环境感知 | 电量 / 网络 / 下载 / 空闲 / 锁屏 / 全屏，可逐项关掉 | `settings.json: evt_*` |
| 问候与气泡 | 点亮屏幕问好、启动问好、显示气泡、气泡显示文件名 | `settings.json: greet_* / bubble*` |
| 位置记忆 | 拖到哪下次还在哪；换屏或改分辨率会夹回可见区域 | `settings.json: x / y` |
| 回到角落 / 重载素材 / 关闭 | — | — |

关闭按钮只是收起面板（`WM_CLOSE` 也拦成隐藏），下次打开还是同一个窗口，
且会把控件状态与当前值重新同步一次。

`app.manifest` 里声明了 comctl32 v6，按钮和滑块才是 Win 10/11 的现代外观；
不声明就是 Win95 那种灰色凸起。清单用 `build.rs` + `embed-resource` 调 `rc.exe` 嵌进 exe，
`app.rc` 只有一行 `1 24 "app.manifest"`。

> 注意：`WM_CLOSE` 被主窗口吞掉了——桌宠只从托盘退出。副作用是 Alt+F4 关不掉它，
> 但这也避免了各种系统广播把宠物莫名关掉（调试期间实测确实会遇到）。

## 托盘图标

第一次运行从待机动画的第一帧生成 `assets/icon.ico`：先按 alpha 包围盒裁成正方形
（640×360 直接压成 32×32 会把人压扁），再双线性缩到 32×32，PNG 塞进 ICO 容器
（Vista 以后都认这种写法，省掉 BMP + mask 那套）。删掉 `icon.ico` 会重新生成。

## 构建

```bash
cargo build --release --offline
# 产物：target/release/dsh-pet-rs.exe  (约 1 MB)
```

首次编译要拉 `windows-sys` / `image` / `image-webp` / `serde` / `embed-resource`。
国内网络慢就配镜像：

```toml
# ~/.cargo/config.toml
[source.crates-io]
replace-with = "tuna"
[source.tuna]
registry = "sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/"
```

## 打包发布

```powershell
powershell -ExecutionPolicy Bypass -File tools\package.ps1
# 加 -Zip 同时出压缩包
```

产物：

```
dist\dsh-pet-rs\
  dsh-pet-rs.exe        约 1 MB
  assets\
    anims\*.webp        106 段动作，238 MB
    behaviors.json      行为配置
    events.json         环境事件、传感器阈值、问候时间段
  README.md
  使用说明.txt
```

### 素材为什么不内嵌

106 段素材 238 MB，塞进 exe 就变成 238 MB 的单文件：改一个动作要重编，
而且解码器要的是文件/内存流，内嵌后每次启动都得把整块读进内存。
现在这样，往 `assets\anims` 丢一个 `.webp` 就能用，右键「重载素材」立刻生效。

### 为什么配置不写在 assets 里

装到 `Program Files` 这类只读目录时写不进去。所以读写分开：

| 内容 | 位置 | 权限 |
|---|---|---|
| 素材 `anims\`、`behaviors.json`、`events.json` | exe 同级 `assets\` | 只读 |
| `settings.json`（面板设置） | `%APPDATA%\dsh-pet-rs\` | 可写 |
| `icon.ico`（托盘图标） | 同上 | 可写 |
| `dsh-pet-rs-events.log`（`--debug-events` 日志） | 同上；写不进去回退 `%TEMP%\` | 尽力而为 |
| `*.clipcache`（时长表） | 素材旁边 | 尽力而为，写不进去就每次重扫 |

素材搜索顺序：**exe 同级 `assets`** → `%APPDATA%\dsh-pet-rs\assets` →
开发期工程目录。都找不到会弹框提示，不会静默失败。

### 想做成单文件？

可以内嵌一段最简素材（一个待机动作，约 3 MB）当兜底：exe 变 4 MB 左右，
双击就能看到宠物；有 `assets` 时用完整素材。

代价是要把 `Stream` 的 `BufReader<File>` 泛型化（`WebPDecoder<R>` / `Frames<'a>`
都绑着具体 reader 类型），或者加一层 `Cursor<Vec<u8>>` 分支。没做，留作选项——
238 MB 的素材终归得跟着走，单文件只能解决「只有一个动作」的场景。

## 素材

> **仓库里不含素材**（238 MB，而且美术资源版权归原作者，不适合再分发一份）。
>
> 从原项目的 `assets/webm/` 目录取出来，按下文转换一次就行，脚本会自动建目录。

全部动作放 `assets/anims/`，106 段，共 238 MB。格式优先级：`.webp` > `.apng` > `.gif`。
文件名（去扩展名）就是 `behaviors.json` 里引用的动作名。

从原项目的透明 webm 批量转换：

```powershell
python tools\batch-webp.py <webm目录> .\assets\anims 4 12 80
#                             源目录      输出     并行 fps 质量
```

核心命令只有一条：

```
ffmpeg -c:v libvpx-vp9 -i in.webm -vf "fps=12,format=rgba" \
       -c:v libwebp_anim -lossless 0 -quality 80 -loop 0 out.webp
```

两个必须的参数：
- `-c:v libvpx-vp9`：不指定的话 ffmpeg 会把 alpha 当普通 yuv420p 丢掉，出来是黑底
- `format=rgba`：WebP 要显式要 alpha 通道

**为什么用 WebP 不用 APNG**：同样 640×360 / 12fps，APNG 单个 14 MB，WebP 2.1 MB。
106 段下来是 238 MB 对 1.5 GB。`image` crate 只能解静态 WebP，动画要走
`image_webp::WebPDecoder::read_frame`，返回帧时长，正好匹配流式播放。

`tools/prepare.ps1` 是旧的 APNG 版本，保留给需要无损的场合。

## 行为规则

`assets/behaviors.json` 对应原项目 `config.jsonc` 的 `animations` /
`animationWeights` / `physics` 三段：

```jsonc
"weights": { "idle": 10, "turn": 5, "move": 5 }   // 剩下 80 分给各分类
"categories": [                                    // 分类内按 weight 抽类，
  { "id": "小动作", "weight": 20, "actions": [...] },   // 再类内等概率抽动作
  { "id": "玩耍",   "weight": 20, "actions": [...] },
  { "id": "文字",   "weight": 10, "noMirror": true, "actions": [...] }
]
```

链路：**每段动画播完，按权重抽下一档**——待机 10 / 转向 5 / 移动 5 /
随机分类 80。抽到转向，播完翻转朝向；抽到移动，按当前朝向走
`minDist~maxDist`（贴边了自动转身）；抽到分类，先按分类权重抽类、再类内随机。

`physics` 段的 `gravity` / `restitution` / `groundFriction` 直接作用于甩抛：
`groundFriction` 是衰减率，按 `v *= exp(-k·dt)` 算。

## 环境感知（v0.2）

v0.2 加的是**本地环境感知**，不是聊天助手：所有判断都在进程内完成，不发网络请求，
不读剪贴板 / 窗口标题，也不装全局键盘钩子。默认动作仍然很克制——电量、网络和下载
主要用气泡说话；只有插电、充满、下载完成、空闲、锁屏、解锁、睡眠恢复才会切动画。
想加戏不用改 Rust，改 `assets/events.json` 再「重载素材」即可。

### 能感知什么

| 信号 | 数据来源 | 默认反应 |
|---|---|---|
| 电量 | `GetSystemPowerStatus`，默认 60 秒轮询 | 低于 20% / 10% 弹气泡；插电和充满播动作 + 气泡 |
| 网络 | `InternetGetConnectedState`，默认 15 秒轮询，连续 2 次一致才认 | 断网 / 恢复弹气泡 |
| 下载 | 系统「下载」文件夹，默认 2 秒轮询临时文件后缀 | 开始 / 失败弹气泡；完成播拆礼物/收红包 + 气泡 |
| 空闲 | `GetLastInputInfo`，默认 10 分钟无输入睡觉 | 睡觉动画循环；有输入后伸懒腰 |
| 锁屏 / 恢复 | `WTSRegisterSessionNotification` + `WM_POWERBROADCAST` | 锁屏睡觉（循环）；解锁哈欠；睡眠恢复伸懒腰 |
| 全屏 | `SHQueryUserNotificationState` + 前台窗口几何 | 直接隐藏桌宠，退出全屏恢复；不经过事件规则 |

全屏只在三种情况下算数：`QUNS_RUNNING_D3D_FULL_SCREEN`、`QUNS_PRESENTATION_MODE`，
或者 `QUNS_BUSY` 且前台窗口完整盖住所在显示器。普通最大化窗口不算，自己的窗口也会排除。
电量第一次轮询只记录基线：如果启动时已经插着电，不会补播「充上电啦」；但如果启动时
就在电池上且电量很低，低电量事件仍会正常触发。

### `assets/events.json`

事件规则、传感器阈值、问候时间段都在这一个文件里。支持 `//` 行注释，UTF-8 有 BOM
也能读；改完保存，托盘右键「重载素材」即时生效。结构大致是：

```jsonc
{
  "globalCooldownSec": 8,          // 会切动画的事件之间至少隔 8 秒
  "bubbleStyle": {
    "fontFamily": "Microsoft YaHei UI",
    "fontSize": 16,                 // 96 DPI 基准，高 DPI 自动缩放
    "fontWeight": 400,
    "textColor": "#FFFFFF",
    "backgroundColor": "#2B2B2B",
    "alpha": 240,
    "cornerRadius": 7,
    "padding": 12,
    "maxWidth": 320,
    "durationMs": 4000
  },
  "sensors": {
    "battery":  { "lowPct": 20, "criticalPct": 10, "pollSec": 60 },
    "network":  { "pollSec": 15, "confirmTicks": 2 },
    "idle":     { "sleepSec": 600, "wakeSec": 5 },
    "download": {
      "dir": "",                   // 空 = 自动找系统“下载”文件夹
      "pollSec": 2,
      "tempExts": [".crdownload", ".part", ".opdownload"]
    },
    "fullscreen": { "pollSec": 2 }
  },
  "events": {
    "batteryLow": {
      "actions": [],               // 空数组 = 只弹气泡
      "bubble": "电量只剩 {pct}% 了…",
      "priority": 80,
      "cooldownSec": 600,
      "interrupt": true
    }
  },
  // 问候时间段：见下方「问候」一节
  "greetings": {}
}
```

单条事件支持的字段：

| 字段 | 默认 | 说明 |
|---|---|---|
| `enabled` | `true` | 写 `false` 可临时关掉，不用删整条 |
| `actions` | `[]` | 候选动作名，随机抽一个；空数组表示只弹气泡 |
| `bubble` | 无 | 气泡模板；省略就不弹气泡 |
| `priority` | `50` | 数字越大越先处理 |
| `cooldownSec` | `60` | 同一事件两次触发的最小间隔 |
| `interrupt` | `false` | 是否允许打断当前动画 |
| `hold` | `false` | 播完从头循环，保持状态，直到收到退出事件 |
| `noMirror` | `false` | 带文字的动作在朝右时先转回朝左 |

模板变量：`{pct}` 电量百分比、`{name}` 文件名、`{size}` 文件大小、`{time}` 当前时间。
事件键缺席等于关闭该事件；文件整个缺失时桌宠仍能正常待机，只是不再响应环境事件。
`actions` 里写了不存在的动作不会崩：跳过并在 `--debug-events` 日志里提示；如果整条
动作都找不到，气泡仍然会显示。

### 气泡样式

`bubbleStyle` 控制所有气泡的外观，改的是下一次弹出的气泡；已经在屏幕上的气泡不会
中途换样式。它没有放进设置面板，避免面板继续变高，直接改 `events.json` 再「重载素材」
即可。几个常用值：

| 字段 | 默认 | 说明 |
|---|---|---|
| `fontFamily` | `Microsoft YaHei UI` | 字体名；系统没有时 GDI 自动回退 |
| `fontSize` | `16` | 96 DPI 基准字号，实际按系统 DPI 缩放 |
| `fontWeight` | `400` | GDI 字重，`700` 是粗体 |
| `textColor` | `#FFFFFF` | 文字颜色，`#RRGGBB` |
| `backgroundColor` | `#2B2B2B` | 背景颜色，`#RRGGBB` |
| `alpha` | `240` | 整体不透明度，`0`–`255` |
| `cornerRadius` | `7` | 圆角半径，`0` 是直角 |
| `padding` | `12` | 文字与边缘的内边距 |
| `maxWidth` | `320` | 最大宽度，超出自动换行 |
| `durationMs` | `4000` | 显示时长，范围 `500`–`60000` 毫秒 |

字号、圆角、内边距、最大宽度都按系统 DPI 缩放；颜色写错会回退到默认色，数值过小或
过大会被自动夹到合法范围。`fontFamily` 留空也会回退到微软雅黑 UI。想快速试效果，
可以临时配一个醒目的颜色再运行：

```powershell
dsh-pet-rs.exe --simulate downloadDone:test.zip:1048576
```

### 默认事件表

| 事件键 | 默认行为 |
|---|---|
| `batteryLow` | 气泡：电量低 |
| `batteryCritical` | 气泡：快充电 |
| `batteryCharging` | 播「余额-钱袋满溢」+ 气泡 |
| `batteryFull` | 播「余额-金袋叮当」+ 气泡 |
| `networkDown` / `networkUp` | 断网 / 恢复气泡 |
| `downloadStart` | 气泡：开始下载 |
| `downloadDone` | 播「拆礼物」/「收红包」+ 文件名和大小 |
| `downloadFailed` | 气泡：好像没下成 |
| `idleSleep` | 循环播放睡觉动画（`hold: true`） |
| `idleWake` | 播「超大伸懒腰」 |
| `sessionLock` | 循环播放睡觉动画（`hold: true`） |
| `sessionUnlock` | 播「哈欠连天」 |
| `resume` | 播「超大伸懒腰」；问好气泡可以同时出现 |
| 全屏 | 不经过事件，直接隐藏 / 恢复桌宠 |

`hold` 的含义是**从头循环播放**，不是停在最后一帧。睡觉动作会一直循环，直到
`idleWake`、`sessionUnlock`、`resume` 或屏幕点亮；锁屏时则直接切到睡觉并开始循环。
屏幕熄灭 / 锁屏 / 睡眠期间攒下的事件会在恢复时清掉，避免一解锁就补播旧事件。

### 下载识别

默认监听系统「下载」文件夹里的临时后缀：`.crdownload`、`.part`、`.opdownload`、
`.download`、`.!qb`、`.aria2`、`.td`、`.xltd`。临时文件出现 = 开始，消失且出现
完成文件 = 完成，消失但没有完成文件 = 失败。它只读取文件名、大小和修改时间，不打开
文件内容，也不区分是哪个浏览器下的。

`dir` 留空时先读注册表里的 Downloads 路径，再退回 `%USERPROFILE%\Downloads`；
也可以写绝对路径。下载目录在程序启动时不存在的话，重载素材时会再试一次。

### 问候

问候在四种时机触发：启动（默认延迟 1.5 秒，避开首帧加载）、屏幕点亮、解锁、睡眠恢复。
只有屏幕亮着、未锁屏、桌宠可见且不在全屏时才问好；点亮 / 解锁 / 恢复还要求离开至少
60 秒，两次问好至少隔 10 分钟。启动问好和点亮问好可以在设置面板里分别关掉。

时间段写在 `greetings.slots`，`from` / `to` 都是 `HH:MM`，**两端都包含**，并且支持
跨午夜（例如 `23:00`–`04:59`）。当前时间不属于任何时间段时使用 `fallbackTexts`；
离开超过 `longAwaySec`（默认一天）优先使用 `longAwayTexts`。问候默认只弹气泡，
`slots[].actions` / `longAwayActions` 留空即可；如果想让问候也播动作，填动作名即可。

解锁和恢复的动画不会被问候吞掉：问候气泡和 `sessionUnlock` / `resume` 动作可以同时
发生。关掉「显示气泡」后，问候仍可通过时间段里的 `actions` 播动画；两者都没有时就不问好。

### 调试

```powershell
dsh-pet-rs.exe --debug-events
dsh-pet-rs.exe --simulate downloadDone:test.zip:1048576
dsh-pet-rs.exe --simulate greet:20:30
dsh-pet-rs.exe --greet-at 20:30
```

`--debug-events` 会把初始化、事件入队、动作启动、问候原因写到 stderr，并尝试写
`%APPDATA%\dsh-pet-rs\dsh-pet-rs-events.log`；配置目录写不进去时回退到
`%TEMP%\dsh-pet-rs-events.log`，只警告一次，不影响桌宠运行。`--simulate` 支持逗号分隔
的多个事件，`downloadDone:<文件名>:<字节数>` 可以模拟下载完成，`greet[:HH:MM]` 可以
指定问候时间；带 `--simulate` 时不会自动播启动问候。`--greet-at` 只改问候用的测试时间，
不影响系统时间。

## 交互

| 操作 | 行为 |
|---|---|
| 左键单击 | 从 `clicks` 池随机播一段回应 |
| 左键拖拽 | 播 `drag` 姿势并跟手 |
| 甩出去 | 抛物线 + 边缘反弹 + 落地摩擦停稳 |
| 右键 | 分类子菜单（待机/转向/移动/点击/拖拽/小动作/玩耍/吃什么/时节/文字）+ 设置 |
| 托盘双击 | 打开设置面板 |
| `--play "<动作名>"` | 启动即播某段动作，调试用 |
| `--settings` | 启动即打开设置面板 |
| `--debug-events` | 输出环境事件日志，排查传感器和规则 |
| `--simulate <事件>` | 模拟 `batteryLow`、`downloadDone:...`、`greet:20:30` 等事件 |
| `--greet-at HH:MM` | 只把问候的测试时间固定到某个时刻 |
| 待机 | 完全按权重自己演，会走会转身 |

## 目录

```
src/
  main.rs      窗口类、消息循环、WndProc、单实例、命令行
  app.rs       状态机：动画链、朝向、位移、拖拽、物理、缩放、事件队列
  behavior.rs  behaviors.json 的结构与加载
  event.rs     events.json 的结构、事件规则、问候时间段
  sense.rs     电量 / 网络 / 下载 / 空闲 / 全屏 / 屏幕与锁屏传感器
  bubble.rs    独立的分层点击穿透气泡窗口
  settings.rs  设置面板（原生控件，含环境感知开关）
  autorun.rs   开机自启（写 HKCU Run 项）
  surface.rs   DIB 表面 + UpdateLayeredWindow + 水平镜像
  scale.rs     双线性缩放（预乘 alpha）
  clip.rs      WebP/APNG/GIF 探测与流式解码
  icon.rs      从素材生成 .ico
  tray.rs      托盘 + 两级右键菜单
  util.rs      光标、工作区、xorshift
build.rs       rc 编译 manifest
app.rc         `1 24 "app.manifest"`
app.manifest   comctl32 v6 声明 + per-monitor DPI
tools/
  batch-webp.py webm → 动画 WebP（推荐）
  prepare.ps1   webm → APNG（无损，体积大）
assets/
  behaviors.json  行为配置
  events.json     环境事件、传感器阈值、问候时间段
  anims/*.webp    106 段动作
  *.clipcache     每段动画的时长表缓存，自动生成，可删
%APPDATA%\dsh-pet-rs\
  settings.json   面板写下的尺寸/透明度/权重/开关/位置（自动生成）
  icon.ico        托盘图标（自动生成，可删）
  dsh-pet-rs-events.log  --debug-events 日志（尽力而为）
```

启动时只探测待机那一条，其余在后台线程慢慢建缓存；首次播放某段可能等
一两百毫秒，之后秒开。

## 已知限制（以及它不做的事）

这个项目的定位是"桌宠 + 本地环境反应"，所以下面这些**故意没有**，不是没做完：

- **不会聊天**。没有 AI 对话，也没有碎碎念。
- **不会提醒你余额**，不跟着工作状态换表情——那些都需要接宿主环境，
  原项目做得很好，请用它。
- **不联网、不读隐私数据**。环境感知只看本机 API 和下载目录的文件名/大小/时间，
  没有 HTTP 服务，没有天气，不监控剪贴板、窗口标题或全局键盘。
- **网络事件是本地适配器状态**，不是「能不能打开某个网站」；断网/恢复有防抖，
  但 VPN、代理和强制门户可能让它看起来和实际访问不一致。
- **下载识别靠临时后缀**（`.crdownload` / `.part` 等），直接写最终文件、改名或
  浏览器自定义临时名的下载可能漏报；开始/完成/失败是启发式判断，不是下载器 API。
- **全屏识别靠系统通知状态 + 窗口几何**，特殊游戏、多显示器和远程桌面下可能误判；
  出现误隐藏可以在设置里关掉「全屏时隐藏」。
- **锁屏 / 恢复依赖 Windows 会话消息**，精简系统或某些远程环境可能收不到。
- **没有动作制作教程**。想做自己的动作去看
  [原项目](https://github.com/PC2005-cloud/dsh-pet) 的文档。
- **仅 Windows**。`UpdateLayeredWindow` 是 Win32 独占，macOS 得另写一套。
- 不支持高帧率大尺寸素材（GDI 软件合成）。
- 放得越大每帧越费（820 宽约 5 ms/帧），900 是上限。
- 没有安装包，就是解压即用的绿色版。
- 主分支按现代 Windows 的 per-monitor DPI 写；要 Win7 兼容请用 `win7-compat`
  分支，别把 Win8+ 才有的 API 带过去。

## 致谢与版权

**素材、动作分类、行为权重、以及整个项目的灵感都来自
[PC2005-cloud/dsh-pet](https://github.com/PC2005-cloud/dsh-pet)。**
那个项目还提供了一整套桌宠动作制作流程，想做自己的角色强烈建议去看。

- 本仓库代码：MIT
- 美术素材：CC BY-NC-SA 4.0 —— **禁止商用**，转载 / 二创须署名原作者
- 素材不随仓库分发，需自行从原项目转换（见上方「素材」一节）

