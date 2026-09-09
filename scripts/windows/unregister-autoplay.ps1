#Requires -Version 5.1
<#
.SYNOPSIS
  卸载 Photo Tool 的 Windows「自动播放」处理器（register-autoplay.ps1 的逆操作）。
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\windows\unregister-autoplay.ps1
#>
param(
  [ValidateSet('CurrentUser', 'LocalMachine')]
  [string]$Scope = 'CurrentUser'
)

$ErrorActionPreference = 'Stop'

$handlerName = 'PhotoToolImport'
$progId = 'PhotoTool.Import'
$events = @('ShowPicturesOnArrival', 'MixedContentOnArrival')

if ($Scope -eq 'LocalMachine') {
  $autoRoot = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers'
  $classesRoot = 'HKLM:\SOFTWARE\Classes'
} else {
  $autoRoot = 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers'
  $classesRoot = 'HKCU:\SOFTWARE\Classes'
}

foreach ($ev in $events) {
  $evKey = Join-Path $autoRoot "EventHandlers\$ev"
  if (Test-Path $evKey) {
    Remove-ItemProperty -Path $evKey -Name $handlerName -ErrorAction SilentlyContinue
  }
}
Remove-Item -Path (Join-Path $autoRoot "Handlers\$handlerName") -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -Path (Join-Path $classesRoot $progId) -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "已注销 AutoPlay 处理器：$handlerName ($Scope)"
