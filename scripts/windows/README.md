# Windows「自动播放」集成：用 Photo Tool 导入照片

插入 SD 卡 / U 盘时，Windows 的「自动播放」弹窗会出现「用 Photo Tool 导入照片」，
点击后以该设备根目录作为导入源启动应用（`ftpt.exe --import "<盘符或目录>"`），
与 Linux 的 KDE Solid 设备动作等价。

## 机制

注册表（AutoPlay Handlers，微软文档明确支持 HKLM 或 HKCU）：

```
...\Explorer\AutoplayHandlers\Handlers\PhotoToolImport
    Action        = 用 Photo Tool 导入照片
    Provider      = Photo Tool
    DefaultIcon   = "<exe>",0
    InvokeProgID  = PhotoTool.Import
    InvokeVerb    = open
...\Explorer\AutoplayHandlers\EventHandlers\ShowPicturesOnArrival
    PhotoToolImport = ""
...\Explorer\AutoplayHandlers\EventHandlers\MixedContentOnArrival
    PhotoToolImport = ""
...\Classes\PhotoTool.Import\shell\open\command
    (默认) = "<exe>" --import "%1"
```

`%1` 由系统替换为设备根路径。应用侧的 `--import` 解析与单实例转发见
`crates/photo-tauri/src-tauri/src/lib.rs`（与 Linux 共用同一套逻辑）。

## 开发环境（`pnpm run tauri dev`）

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\register-autoplay.ps1 `
    -ExePath .\target\debug\ftpt.exe
```

注册后插入卡即可测试；应用已在运行时会转发到已有窗口并置顶（Windows 侧靠
`AllowSetForegroundWindow(ASFW_ANY)` 解除前台锁）。

注销：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\unregister-autoplay.ps1
```

## 打包安装

NSIS 安装包已通过 `src-tauri/windows/installer-hooks.nsh` 自动注册/注销
（Tauri `bundle.windows.nsis.installerHooks`），安装后无需手动执行脚本。

## 注意

- 默认写 HKCU，不需要管理员；`-Scope LocalMachine` 写 HKLM，需管理员。
- 「自动播放」弹窗被关闭时不会出现：设置 → 蓝牙和其他设备 → 自动播放 → 开启
  「为所有媒体和设备使用自动播放」。
- 想设成默认动作（不弹窗直接导入），可在自动播放设置里把「照片」的默认动作选为
  「用 Photo Tool 导入照片」。
