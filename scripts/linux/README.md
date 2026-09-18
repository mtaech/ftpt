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
  ⚠️ **GPUI 版尚未接线 `--import`**（见文末「现状」）：目前点这个菜单项只会以普通方式启动应用。
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
## 现状：`--import` 与单实例转发尚未接线（2026-09-18）

原 Tauri 版由 `crates/photo-tauri/src-tauri/src/lib.rs` 解析 `--import <挂载点>`，并靠
`tauri-plugin-single-instance` 把重复启动转发给已运行实例（发 `import:open` 事件打开导入对话框）。
该 crate 已删除，而 GPUI 版 `crates/photo-ui/src/main.rs` **还没有解析命令行参数、也没有单实例通道**，所以：

- 这个 Solid 动作**不会**自动打开导入对话框，只是把应用启动起来；应用已在运行时还会开出第二个实例。
- 先把它当占位留着：等 `photo-ui` 补上 `--import`（解析后把挂载点塞进
  `crates/photo-ui/src/state/import.rs` 的导入源）再启用。

开发运行直接 `cargo run -p photo-ui`（二进制 `target/debug/ftpt`；本机 target 目录可能被
`~/.cargo/config.toml` 改到仓库外，用 `cargo metadata` 查）。
