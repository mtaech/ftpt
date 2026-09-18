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

`%1` 由系统替换为设备根路径。⚠️ **GPUI 版尚未实现 `--import` 解析与单实例转发**（原 Tauri 版在
`crates/photo-tauri/src-tauri/src/lib.rs`，该 crate 已于 2026-09-18 删除）：注册后「自动播放」菜单项
会在，但点击只会普通启动应用、不会自动打开导入对话框；已在运行时还会开出第二个实例。
机制与 Linux 侧同源，见 `scripts/linux/README.md` 文末「现状」。

## 注册 / 测试（开发环境）

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\register-autoplay.ps1 `
    -ExePath .\target\debug\ftpt.exe
```

注册后插入卡即可测试。原 Tauri 版靠 `AllowSetForegroundWindow(ASFW_ANY)` 把重复启动转发到已有窗口并置顶；
GPUI 版尚无单实例通道（见上）——重复启动会开新窗口。

注销：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\unregister-autoplay.ps1
```

## 打包安装

GPUI 版没有安装包构建流程：发布产物是 `scripts/package.ps1` 打出的便携 zip（解压即用），
自动播放注册需要手动跑 `register-autoplay.ps1`（注销用 `unregister-autoplay.ps1`）。
原 Tauri 版的 NSIS installer-hooks 已随 `crates/photo-tauri` 一起删除。

## 注意

- 默认写 HKCU，不需要管理员；`-Scope LocalMachine` 写 HKLM，需管理员。
- 「自动播放」弹窗被关闭时不会出现：设置 → 蓝牙和其他设备 → 自动播放 → 开启
  「为所有媒体和设备使用自动播放」。
- 想设成默认动作（不弹窗直接导入），可在自动播放设置里把「照片」的默认动作选为
  「用 Photo Tool 导入照片」。
