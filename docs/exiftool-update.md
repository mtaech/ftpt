# ExifTool 本地运行时与更新指引

photo-engine 的 EXIF 提取主后端（`ExifToolProvider`）调用本地捆绑的
ExifTool。运行时文件放在 `local-lib/exiftool/`（**不纳入版本控制**，
见仓库根 .gitignore），本文件是权威更新指引（进 git）。

## 目录结构（版本无关，覆盖式更新）

```
local-lib/exiftool/          # 本地运行时（gitignore，不进 git）
├── VERSION.txt              # 当前版本号（如 13.59）——唯一版本标记
├── README.md                # 一行指向本文件的提示（冗余）
├── windows/                 # Windows 版（官方 exiftool-<ver>_64.zip 解压内容）
│   ├── exiftool.exe                # -k 变体（内嵌 -k，程序化调用不用它）
│   └── exiftool_files/             # perl.exe + exiftool.pl + lib/（实际使用）
└── linux/                   # Linux 版（官方 Image-ExifTool-<ver>.tar.gz 解压内容）
    ├── exiftool                     # 纯 Perl 脚本（shebang #!/usr/bin/env perl）
    └── lib/                         # 模块（Image/ExifTool.pm 等）
```

## 更新步骤（升级 ExifTool 版本）

1. 下载新版（`<ver>` 替换为目标版本，如 13.59）：
   - Windows：`https://exiftool.org/exiftool-<ver>_64.zip`
     （官方页面 https://exiftool.org/ 的 Windows 下载；也可用
     sourceforge 镜像）
   - Linux：`https://exiftool.org/exiftool-<ver>.tar.gz`（纯 Perl 源码包）
2. 解压到临时目录，**清空并覆盖**对应平台目录：
   ```bash
   # Windows（zip 解压出 exiftool_files/ + exiftool(-k).exe）
   rm -rf local-lib/exiftool/windows
   mkdir -p local-lib/exiftool/windows
   # 把 exiftool_files/ 与 exiftool(-k).exe 移入 windows/

   # Linux（tar.gz 解压出 Image-ExifTool-<ver>/）
   rm -rf local-lib/exiftool/linux
   mkdir -p local-lib/exiftool/linux
   tar -xzf Image-ExifTool-<ver>.tar.gz \
       -C local-lib/exiftool/linux --strip-components=1
   ```
3. 更新 `local-lib/exiftool/VERSION.txt` 为 `<ver>`。
4. 验证：
   ```bash
   # 版本
   local-lib/exiftool/windows/exiftool_files/perl.exe exiftool.pl -ver   # Windows
   perl local-lib/exiftool/linux/exiftool -ver                           # Linux

   # 程序化提取（含对焦点）
   cargo run -p photo-engine --example focus_check -- <RAW 或 JPG>
   ```
5. 打包产物复制由 scripts/package.ps1 处理（exe 同级 `exiftool/`）。

## 运行方式（photo-engine 自动选择）

- **Windows**：`perl.exe exiftool.pl`——**不要用 exiftool(-k).exe**：
  它内嵌 `-k`（每个命令后等待 ENTER），不适合程序化调用。
- **Linux**：系统 perl 执行 `linux/exiftool` 脚本。目标机需预装 perl
  （多数发行版自带）；若无 perl，需另装或用源码包自带运行时。
- **macOS**：与 Linux 相同（同一源码包）。

定位优先级（`find_root`）：`PHOTO_EXIFTOOL` env → exe 同级
`exiftool/`（打包产物）→ 仓库 `local-lib/exiftool/`（开发/测试）→ PATH。

## 对焦点等厂商私有字段（为什么走 exiftool）

统一后端解决了 kamadak-exif 无法覆盖的厂商私有数据：

| 数据源 | 输出 tag | 映射 |
|---|---|---|
| JPEG EXIF SubjectArea (0x9214) | `SubjectArea` | point/circle/rectangle → FocusPoint |
| Panasonic MakerNote AFPointPosition (0x004d) | `AFPointPosition` + `AFAreaSize`（已归一化） | 中心 + 区域尺寸 → 矩形 |
| Nikon AFInfo2（V0300/V0400） | `AFImageWidth` + `AFAreaX/YPosition` 等 | AF 图像坐标系 → 归一化矩形 |

本地 rawlib 路径（Fuji FocusPixel / Nikon AFInfo blob）保留为回退后端，
exiftool 不可用时 RAW 仍可提取（常规图无回退）。

## 已知坑

- `-stay_open` 长驻进程模式**不能加 `-q`**：`-q` 会同时抑制 `{ready}`
  标记，导致读取端等不到结果边界而挂起。
- 版本目录名（`Image-ExifTool-<ver>/`）因升级变化，代码按固定
  `linux/exiftool` 定位，覆盖式更新即可，无需改代码。
