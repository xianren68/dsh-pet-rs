# 把 dsh-pet 的透明 webm 批量转成 APNG，供 dsh-pet-rs 使用。
# 用法：
#   powershell -ExecutionPolicy Bypass -File tools\prepare.ps1 -Src .\webm -Out .\assets -Fps 15
# 需要 ffmpeg 在 PATH 里，或用 -Ffmpeg 指定路径。

param(
  [string]$Src = ".\webm",
  [string]$Out = ".\assets",
  [int]$Fps = 15,
  [string]$Ffmpeg = "ffmpeg"
)

if (-not (Test-Path $Out)) { New-Item -ItemType Directory -Path $Out | Out-Null }

$files = Get-ChildItem -Path $Src -Filter *.webm
if ($files.Count -eq 0) { Write-Host "no .webm found in $Src"; exit 1 }

foreach ($f in $files) {
  $name = $f.BaseName
  # 文件名转成 ASCII 安全的形式：保留字母数字，其余变下划线
  $safe = ($name.ToCharArray() | ForEach-Object {
    if ($_ -match '[0-9A-Za-z\-_]') { $_ } else { '_' }
  }) -join ''
  if ([string]::IsNullOrWhiteSpace($safe)) { $safe = "clip$($f.Name.GetHashCode())" }
  $target = Join-Path $Out ($safe + ".apng")

  Write-Host "-> $name  =>  $safe.apng"
  # -c:v libvpx-vp9 必须显式指定，否则 alpha 会被当普通 yuv420p 丢掉
  & $Ffmpeg -y -hide_banner -loglevel error -c:v libvpx-vp9 -i $f.FullName `
    -vf "fps=$Fps,format=rgba" -pix_fmt rgba -plays 0 -f apng $target

  if ($LASTEXITCODE -ne 0) {
    Write-Host "   FAILED"
  } else {
    $mb = [math]::Round((Get-Item $target).Length / 1MB, 1)
    Write-Host "   ok  ${mb} MB"
  }
}

Write-Host "`ndone. 记得把待机动画命名为 idle*.apng"
