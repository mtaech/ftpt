; Photo Tool Windows「自动播放」处理器（安装时注册 / 卸载时清理）
; 与 scripts/windows/register-autoplay.ps1 写同一组键；%1 = 设备根路径。
; Tauri 通过 bundle.windows.nsis.installerHooks 引入本文件。

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Handlers\PhotoToolImport" "Action" "用 Photo Tool 导入照片"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Handlers\PhotoToolImport" "Provider" "Photo Tool"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Handlers\PhotoToolImport" "DefaultIcon" "$INSTDIR\ftpt.exe,0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Handlers\PhotoToolImport" "InvokeProgID" "PhotoTool.Import"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Handlers\PhotoToolImport" "InvokeVerb" "open"
  ; 设备根路径以反斜杠结尾，%1 不加引号避免 \" 转义（AutoPlay 传盘符根，不含空格）
  WriteRegStr HKCU "Software\Classes\PhotoTool.Import\shell\open\command" "" '"$INSTDIR\ftpt.exe" --import %1'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\EventHandlers\ShowPicturesOnArrival" "PhotoToolImport" ""
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\EventHandlers\MixedContentOnArrival" "PhotoToolImport" ""
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\EventHandlers\ShowPicturesOnArrival" "PhotoToolImport"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\EventHandlers\MixedContentOnArrival" "PhotoToolImport"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\AutoplayHandlers\Handlers\PhotoToolImport"
  DeleteRegKey HKCU "Software\Classes\PhotoTool.Import"
!macroend
