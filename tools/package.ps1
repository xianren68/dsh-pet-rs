<#
  dsh-pet-rs 打包脚本

  用法：
    powershell -ExecutionPolicy Bypass -File tools\package.ps1
    powershell -ExecutionPolicy Bypass -File tools\package.ps1 -Zip

  产物：
    dist\dsh-pet-rs\
      dsh-pet-rs.exe        ~1 MB
      assets\anims\*.webp   全部动作
      assets\behaviors.json 行为配置
      assets\events.json    环境事件 / 传感器 / 问好
      README.md
      使用说明.txt
#>
param(
    [string]$Out = "dist",
    [switch]$Zip
)

$ErrorActionPreference = "Stop"
# Compress-Archive 的进度条会把输出刷满屏
$ProgressPreference = "SilentlyContinue"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "==> 编译 release" -ForegroundColor Cyan
& cargo build --release --offline
if ($LASTEXITCODE -ne 0) { throw "编译失败" }

$name = "dsh-pet-rs"
$pkg = Join-Path $root (Join-Path $Out $name)
if ([System.IO.Directory]::Exists($pkg)) { [System.IO.Directory]::Delete($pkg, $true) }
New-Item -ItemType Directory -Path $pkg -Force | Out-Null

Write-Host "==> 复制 exe 与文档" -ForegroundColor Cyan
Copy-Item (Join-Path $root "target\release\$name.exe") $pkg
Copy-Item (Join-Path $root "README.md") $pkg

Write-Host "==> 复制素材" -ForegroundColor Cyan
$assetsOut = Join-Path $pkg "assets"
$animsOut = Join-Path $assetsOut "anims"
New-Item -ItemType Directory -Path $animsOut -Force | Out-Null
Copy-Item (Join-Path $root "assets\behaviors.json") $assetsOut
Copy-Item (Join-Path $root "assets\events.json") $assetsOut

# 只收素材本体，跳过 .clipcache（运行时按需重建）
$n = 0
Get-ChildItem (Join-Path $root "assets\anims") -File |
    Where-Object { $_.Extension -in ".webp", ".apng", ".gif" } |
    ForEach-Object {
        Copy-Item $_.FullName $animsOut
        $n++
    }
Write-Host "    $n 段动作"

$guide = @'
dsh-pet-rs —— 极简原生 Windows 桌宠
=====================================

【怎么用】
  双击 dsh-pet-rs.exe 即可。宠物出现在屏幕右下角。

【操作】
  左键单击   播一段回应动作
  左键拖拽   抓起来走
  甩出去     抛物线飞行，撞墙反弹，落地停稳
  右键       分类菜单，可以点播任意动作
  托盘图标   双击打开设置；右键有显示/隐藏、回到角落、重载素材、退出
  退出       只能从托盘菜单退出（Alt+F4 被故意屏蔽）

【设置面板】
  宠物大小      200-900 像素，16:9 等比
  不透明度      40-255
  行为倾向      待机 / 转向 / 走动 三档相对权重
  原地待机      勾上后不自己走、不自己转朝向
  开机自动启动  写 HKCU 的 Run 项，取消勾选即移除
  环境感知      电量 / 网络 / 下载 / 空闲 / 锁屏 / 全屏 /
                点亮屏幕问好 / 启动问好 / 气泡 / 文件名
  数工资        勾选后右键菜单出现「数工资」；填上下班、午休、
                月薪，按今天已工作的比例报出赚了多少
  位置记忆      拖到哪，下次启动还在哪；换屏会自动夹回

【环境感知】
  全部是本地规则，不联网、没有 AI、没有全局键鼠钩子。
  规则在 assets\events.json：
    电量低 / 充满、网络断开 / 恢复、下载开始 / 完成 / 失败、
    空闲睡觉 / 醒来、锁屏 / 解锁 / 睡眠恢复、按时间段问好。
  气泡样式也在 events.json 的 bubbleStyle：字体、字号、字重、
    文字色、背景色、透明度、圆角、内边距、最大宽度、显示时长。
  actions 留空数组 = 只弹气泡；改完托盘右键“重载素材”生效。
  问好默认 10 分钟冷却；全屏应用时自动隐藏，设置里可关。

【数工资】
  按「月薪 ÷ 21.75 个计薪日」折算日薪，再按当天已计薪时长占
  总时长的比例算到此刻。午休那段时间完全不计薪。
  上班前显示 ¥0，下班后封顶到全天日薪。
  触发两种：右键菜单「数工资」；或者随机抽到该动作时自动报数。
  午休两栏留空 = 没有午休，只按上下班区间算。
  数字不落盘，每次现算，跨天自动归零。

【文件在哪】
  素材目录   exe 同级的 assets\
             anims\*.webp      全部动作
             behaviors.json    动画池 / 权重 / 物理
             events.json       环境事件 / 传感器 / 问好
  配置目录   %APPDATA%\dsh-pet-rs\
             settings.json     面板设置、事件开关、位置
             dsh-pet-rs-events.log  --debug-events 事件日志（不可写则退到 %TEMP%）
             icon.ico          托盘图标（从待机动画第一帧生成）

  配置放在 APPDATA，是为了让程序装到 Program Files 这类
  只读目录时也能保存设置。

【换素材】
  把 .webp / .apng / .gif 丢进 assets\anims\，右键托盘
  “重载素材”即可，不用重启。
  文件名（去扩展名）就是 behaviors.json / events.json 里
  引用的动作名。

  想改行为权重、动作分类，编辑 assets\behaviors.json。
  想改环境反应、电量阈值、问好时段，编辑 assets\events.json。

【命令行】
  dsh-pet-rs.exe --settings
  dsh-pet-rs.exe --play "动作名"
  dsh-pet-rs.exe --simulate "downloadDone:test.zip:1048576,greet:20:30"
  dsh-pet-rs.exe --greet-at 20:30 --simulate greet
  dsh-pet-rs.exe --debug-events
  同一时间只会运行一只；重复双击会提示已在运行。

【兼容性】
  Windows 10 / 11，64 位。
  环境感知没有使用 Win8+ 独占 API；Win7 构建见 win7-compat 分支。
  首次运行会在配置目录生成 icon.ico，属正常现象。

【版权】
  代码 MIT；素材形象 CC BY-NC-SA 4.0，禁止商用。
  素材来自 https://github.com/PC2005-cloud/dsh-pet
'@
$guide | Out-File -FilePath (Join-Path $pkg "使用说明.txt") -Encoding utf8

Write-Host "==> 统计" -ForegroundColor Cyan
$exeSize = (Get-Item (Join-Path $pkg "$name.exe")).Length
$allSize = (Get-ChildItem $pkg -Recurse -File | Measure-Object -Property Length -Sum).Sum
Write-Host ("    exe      {0:N0} KB" -f ($exeSize / 1KB))
Write-Host ("    总计     {0:N1} MB" -f ($allSize / 1MB))

if ($Zip) {
    Write-Host "==> 打包 zip" -ForegroundColor Cyan
    $zipPath = Join-Path $root (Join-Path $Out "$name.zip")
    if ([System.IO.File]::Exists($zipPath)) { [System.IO.File]::Delete($zipPath) }
    # 传目录本身（不是 $pkg\*），解压出来才有顶层 dsh-pet-rs\ 目录
    Compress-Archive -Path $pkg -DestinationPath $zipPath
    Write-Host ("    {0}  {1:N1} MB" -f $zipPath, ((Get-Item $zipPath).Length / 1MB))
}

Write-Host "完成：$pkg" -ForegroundColor Green
