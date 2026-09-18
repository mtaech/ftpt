# 发布打包（GPUI 版）：cargo release → 收集 exe + DLL + 模型 + 名录库 → zip
#
# 用法：pwsh scripts/package.ps1 [-Configuration release] [-Version 0.1.0]
#   -Version 缺省时取根 Cargo.toml 的 [workspace.package].version（单一事实源）
#
# 产物：dist/ftpt-<Version>-windows-x64.zip，解压即用（便携模式：
# 配置 PT.db/PT.toml 与各照片文件夹的 .pt/ 首运时自建，不进包）。
#
# 注意：本脚本是仓库「无构建脚本」约定的唯一有意例外（仅发布打包，
# 构建/测试仍纯 cargo），见 docs/adr/0003-recognition-subsystem.md。
#
# GPUI 版说明（2026-09-18 删除 Tauri 版后重写）：
# - 前端 crate 为 photo-ui（workspace 成员），二进制名 ftpt；没有前端构建步骤，
#   也没有 tauri-build / custom-protocol —— cargo build 出的 exe 就是完整应用
# - 运行时资产：models/*.onnx + data/bird_catalog.db（data_root() 定位：
#   PHOTO_DATA_DIR → exe 同级 models/ → 仓库根回退，见
#   crates/photo-ui/src/state/app_state.rs）
# - DirectML.dll 由 ort(directml) 构建时自动拷入 target 目录，随包收集
#   （onnxruntime 已静态链接进 exe）

param(
    [string]$Configuration = "release",
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot          # 仓库根
$manifest = Join-Path $root "Cargo.toml"
$exeName = "ftpt.exe"
$stageDir = Join-Path $root "dist/ftpt"

# target 目录以 cargo 为准（本机可在 ~/.cargo/config.toml 里改到仓库外）
$targetDir = (cargo metadata --format-version 1 --no-deps --manifest-path $manifest | ConvertFrom-Json).target_directory
$targetDir = Join-Path $targetDir $Configuration

# 版本号：默认取根 Cargo.toml 的 [workspace.package].version，可用 -Version 覆盖
if ([string]::IsNullOrEmpty($Version)) {
    $toml = Get-Content $manifest -Raw
    if ($toml -match '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
        $Version = $Matches[1]
    } else {
        throw "无法从 $manifest 的 [workspace.package] 解析 version，请用 -Version 指定"
    }
}
$zipPath = Join-Path $root "dist/ftpt-$Version-windows-x64.zip"

# 0. 前置检查
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "缺少命令：cargo（请安装 Rust 工具链）"
}

# 1. Rust release 构建
Write-Host "==> cargo build --$Configuration -p photo-ui"
cargo build --$Configuration -p photo-ui --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }

# 2. 资产检查（缺失即失败，不打残缺包）
$exe = Join-Path $targetDir $exeName
$modelsDir = Join-Path $root "models"
$catalogDb = Join-Path $root "data/bird_catalog.db"
foreach ($required in @($exe, $modelsDir, $catalogDb)) {
    if (-not (Test-Path $required)) {
        throw "缺少发布资产：$required（模型放 models/，名录库放 data/bird_catalog.db）"
    }
}

# 3. 收集到暂存目录
if (Test-Path $stageDir) { Remove-Item -Recurse -Force $stageDir }
New-Item -ItemType Directory -Force -Path "$stageDir/models", "$stageDir/data" | Out-Null

Copy-Item $exe "$stageDir/$exeName"
# target 目录根下的运行时 DLL（当前只有 DirectML.dll；onnxruntime 已静态链接）
Get-ChildItem $targetDir -Filter "*.dll" | Copy-Item -Destination $stageDir
Copy-Item "$modelsDir/*.onnx" "$stageDir/models/"
Copy-Item $catalogDb "$stageDir/data/bird_catalog.db"

# EXIF 后端：ExifTool 本地运行时（local-lib/exiftool，跨平台各自打包对应平台目录）
# 运行时定位优先级：exe 同级 exiftool/ → local-lib/exiftool/ → PATH（见 docs/exiftool-update.md）
$exifToolDir = Join-Path $root "local-lib/exiftool"
if (Test-Path $exifToolDir) {
    Copy-Item $exifToolDir "$stageDir/exiftool" -Recurse -Force
}

# 4. 打 zip
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
Compress-Archive -Path $stageDir -DestinationPath $zipPath

# 5. 汇总
$zipSize = "{0:N1} MB" -f ((Get-Item $zipPath).Length / 1MB)
Write-Host ""
Write-Host "==> 完成：$zipPath（$zipSize）"
Get-ChildItem -Recurse $stageDir | ForEach-Object { Write-Host "   $($_.FullName.Substring($stageDir.Length + 1))" }
