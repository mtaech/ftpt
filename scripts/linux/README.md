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
- 每次点击会启动一个新实例；单实例复用（已有窗口时聚焦并传入路径）需要
  `tauri-plugin-single-instance`，当前未接入。
