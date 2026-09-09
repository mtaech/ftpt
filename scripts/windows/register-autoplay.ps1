#Requires -Version 5.1
<#
.SYNOPSIS
  注册 Photo Tool 的 Windows「自动播放」(AutoPlay) 处理器：
  插入 SD 卡 / U 盘时，在自动播放弹窗里出现「用 Photo Tool 导入照片」。

.DESCRIPTION
  写入注册表（默认 HKCU，无需管理员；-Scope LocalMachine 写 HKLM，需管理员，对所有用户生效）：
    ...\AutoplayHandlers\Handlers\PhotoToolImport      处理器元数据
    ...\AutoplayHandlers\EventHandlers\<事件>           把处理器挂到照片/混合内容事件
    ...\Classes\PhotoTool.Import\shell\open\command   "ftpt.exe" --import "%1"
  %1 由系统替换为设备根路径，应用据此打开导入对话框（单实例模式下转发给已运行实例并置顶）。

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\windows\register-autoplay.ps1
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\windows\register-autoplay.ps1 `
      -ExePath C:\path\to\ftpt.exe -Scope LocalMachine
#>
param(
  [string]$ExePath = "",
  [ValidateSet('CurrentUser', 'LocalMachine')]
  [string]$Scope = 'CurrentUser'
)

$ErrorActionPreference = 'Stop'

if (-not $ExePath) {
  $cmd = Get-Command ftpt.exe -ErrorAction SilentlyContinue
  if ($cmd) {
    $ExePath = $cmd.Source
  } else {
    throw "找不到 ftpt.exe，请用 -ExePath 指定完整路径（如 .\target\debug\ftpt.exe）"
  }
}
if (-not (Test-Path -LiteralPath $ExePath)) { throw "exe 不存在: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

$handlerName = 'PhotoToolImport'
$progId = 'PhotoTool.Import'
$action = '用 Photo Tool 导入照片'
$provider = 'Photo Tool'
# ShowPicturesOnArrival = 纯照片设备；MixedContentOnArrival = 混合内容设备
$events = @('ShowPicturesOnArrival', 'MixedContentOnArrival')

if ($Scope -eq 'LocalMachine') {
  $autoRoot = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers'
  $classesRoot = 'HKLM:\SOFTWARE\Classes'
} else {
  $autoRoot = 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers'
  $classesRoot = 'HKCU:\SOFTWARE\Classes'
}

# 1) ProgID：shell\open\command（%1 = 设备根路径）
$cmdKey = Join-Path $classesRoot "$progId\shell\open\command"
New-Item -Path $cmdKey -Force | Out-Null
# 注意：设备根路径（如 E:）以反斜杠结尾，套引号会被 Windows 命令行当转义符吞掉，
# 因此 %1 不加引号（AutoPlay 传的是盘符根，不含空格）。
Set-ItemProperty -Path $cmdKey -Name '(default)' -Value ('"{0}" --import %1' -f $ExePath)

# 2) 处理器元数据
$handlerKey = Join-Path $autoRoot "Handlers\$handlerName"
New-Item -Path $handlerKey -Force | Out-Null
Set-ItemProperty -Path $handlerKey -Name 'Action' -Value $action
Set-ItemProperty -Path $handlerKey -Name 'Provider' -Value $provider
Set-ItemProperty -Path $handlerKey -Name 'DefaultIcon' -Value ('"{0}",0' -f $ExePath)
Set-ItemProperty -Path $handlerKey -Name 'InvokeProgID' -Value $progId
Set-ItemProperty -Path $handlerKey -Name 'InvokeVerb' -Value 'open'

# 3) 挂到事件
foreach ($ev in $events) {
  $evKey = Join-Path $autoRoot "EventHandlers\$ev"
  New-Item -Path $evKey -Force | Out-Null
  Set-ItemProperty -Path $evKey -Name $handlerName -Value ''
}

Write-Host "已注册 AutoPlay 处理器：$handlerName ($Scope)"
Write-Host "  exe : $ExePath"
Write-Host "  事件: $($events -join ', ')"
Write-Host "插入 SD 卡/U 盘后，在「自动播放」弹窗里选「$action」；"
Write-Host "若弹窗没出现：设置 → 蓝牙和其他设备 → 自动播放（确保已开启）。"
