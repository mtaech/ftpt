# 0003 — 鸟类识别子系统：ort 运行时、独立 crate、资产分发与打包

## 状态

已接受（设计阶段，随识别功能落地实施）

## 背景

鸟类识别能力（YOLO 检测 + 鸟种分类 + 名录映射）移植自 Flutter 应用 pica，其管线代码完整但从未接入 UI、结果不落盘。photo_tool 集成时需选定：Rust 推理运行时、代码在 workspace 中的位置、121MB 模型文件与名录库的分发方式。

## 决策

**运行时**：`ort` crate（ONNX Runtime 官方 C API 的 Rust 绑定，当前 2.0.0-rc 系，绑 ONNX Runtime 1.24）。Windows 走 DirectML 执行提供程序（AMD 集显可用），session 创建失败回退 CPU——与 pica 的 `DirectML → CPU` 策略对等。`ort::Session::run` 为同步阻塞调用，与 engine 全同步架构天然契合，UI 侧 rayon 池包装，不引入 tokio。

**代码位置**：新 crate `photo-recognize`，依赖 `photo-engine`（复用 RAW 内嵌预览提取、图像解码机械）与 `photo-domain`（新领域类型：BBox、BirdMatch、RecognitionStatus 等放此）。DAG 变为 `app → photo-recognize → engine → domain`。

**资产分发**：全部按 exe 相对路径解析（便携模式一致），不入版本控制：

```
photo-tool.exe
├── models/{detect.onnx, bird_model.onnx}   121MB，缺失时明确报错
└── data/pica_ref.db                          名录库，裁剪自 pica.db，只读打开
```

名录库裁剪：仅 `animal_info` + `sp_cls_map` 两表（约 15MB），**抛弃 `distributions`（337 万行）与 `places`——pica 中无任何代码查询它们**。以 `READ_ONLY` 打开，无种子复制机制（pica 的种子路径不一致 bug 即源于复制机制，只读打开消除整个 bug 类别）。将来若做"按 GPS 过滤候选鸟种"，从 pica repo 源数据重新生成。

**打包**：`scripts/package.ps1`——`cargo build --release` 后收集 exe + ort `copy-dylibs` 拷出的 dll + models/ + data/ → `Compress-Archive` 产出 zip，解压即用。此为仓库"无构建脚本"约定的**唯一有意例外**（仅发布打包，构建/测试仍纯 cargo），AGENTS.md 相应条款需改写。

## 被否决的选项

| 选项 | 否决原因 |
|---|---|
| `tract`（纯 Rust ONNX） | 仅 CPU；对 yolo26 这类 2025 年新模型的算子覆盖无保障，踩坑即死路 |
| 外挂 Python/子进程推理 | 违背全同步、无外部进程依赖的架构；分发噩梦 |
| 识别代码放 `photo-engine` 内 | ort 是 workspace 最重依赖（构建期下载运行时 + 原生链接），不应拖累 engine 的所有消费者与 `cargo test -p photo-engine` |
| rust-embed 嵌入模型 | 121MB 进二进制，编译链接与单文件体积均不可接受 |
| config 增加模型路径配置项 | 无真实第二路径需求，AppConfig 不加假想字段 |
| 全量 120MB pica.db 照搬 | 为从未被查询的死表付出分发体积 |
| cargo-dist 等打包框架 | 无 CI，过度工程 |

## 已验证项

- ~~ort `download-binaries` 是否内置 DirectML EP~~：**已验证内置**。pyke 预编译运行时为**静态链接**（`rustc-link-lib=static=onnxruntime` + dxguid/DXGI/D3D12/DirectML 链接库），`onnxruntime.dll` 不存在、不需要；唯二动态依赖是 `DirectML.dll`（`copy-dylibs` 自动拷到 target 输出目录）。分发物 = exe + DirectML.dll + models/ + data/

## 影响

- 正：与 pica 同一 ONNX Runtime 底座的 C API，预处理/后处理可对照 Dart 源码逐行移植，行为可验证
- 正：`cargo test -p photo-engine` 等既有工作流不受 ort 拖累
- 正：zip 包解压到任意位置（含 U 盘）可运行，配置/名录/模型全部相对 exe 解析
- 负：ort 处于 2.0.0-rc 阶段，API 仍可能变动；升级需跟进 CHANGELOG
- 负：发布体积 ~250MB（exe + dll + 模型 + 名录库），对个人应用可接受但值得知晓
