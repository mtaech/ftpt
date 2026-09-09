# KDE Solid 设备动作：用 Photo Tool 导入照片

把 `photo-tool-import.desktop` 安装到用户级 Solid 动作目录，即可在 KDE「磁盘和设备」
（Device Notifier）里为挂载的 U 盘 / SD 卡增加「用 Photo Tool 导入照片」菜单项，
点击后以该挂载点作为导入源启动应用并自动打开导入对话框。

## 安装

```bash
mkdir -p ~/.local/share/solid/actions
cp scripts/linux/photo-tool-import.desktop ~/.local/share/solid/actions/
```

- **发布包**：`Exec=ftpt --import "%f"` 假定 `ftpt` 在 PATH 中（或改成绝对路径）。
- **开发运行**：把 `Exec` 改成实际二进制绝对路径，并用 `PHOTO_DATA_DIR` 指向仓库根
  （否则 `data_root()` 在设备动作的启动 cwd 下找不到 `models/` 与 `data/bird_catalog.db`）：

  ```
  Exec=env PHOTO_DATA_DIR=/path/to/photo_tool /path/to/photo_tool/target/debug/ftpt --import "%f"
  ```

改完若菜单项没立即出现，重新插拔设备或重启 plasmashell：

```bash
kquitapp6 plasmashell && kstart plasmashell
```

## 说明

- 谓词沿用 Gwenview 导入器的写法（`StorageVolume.usage == 'FileSystem'`），因此对所有已挂载
  文件系统生效；Device Notifier 默认只展示可移动/热插拔设备。
- 已接入 `tauri-plugin-single-instance`：应用已在运行时，再次点击菜单项会把
  `--import <挂载点>` 转发给已运行实例（发 `import:open` 事件让前端打开导入对话框），
  不会开出第二个窗口。
- 开发运行（`pnpm run tauri dev`）时 debug 二进制从 vite（1420）加载前端，所以设备动作
  需要 dev 进程在跑；要独立可启动就用 `pnpm run tauri build` 的 release 产物（前端已内嵌）。
