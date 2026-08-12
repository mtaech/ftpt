# 发布打包（Tauri v2 版）：前端构建 → cargo release → 收集 exe + DLL + 模型 + 名录库 → zip
#
# 用法：pwsh scripts/package.ps1 [-Configuration release] [-Version 0.1.0]
#   -Version 缺省时取 tauri.conf.json 的 version（单一事实源）
#
# 产物：dist/ftpt-<Version>-windows-x64.zip，解压即用（便携模式：
# 配置 PT.db/PT.toml 与各照片文件夹的 .pt/ 首运时自建，不进包）。
#
# 注意：本脚本是仓库「无构建脚本」约定的唯一有意例外（仅发布打包，
# 构建/测试仍纯 cargo），见 docs/adr/0003-recognition-subsystem.md。
#
# Tauri v2 说明（2026-08-11 迁移完成，统一定名后更新）：
# - 后端 crate 为 photo-tauri（workspace 成员），二进制名 ftpt
# - 前端为 Vue 3 + Vite：frontendDist 由 tauri-build 的 build.rs 在编译期
#   嵌入 exe，因此必须先 npm run build 再 cargo build（顺序不可颠倒）
# - 走 cargo build 而非 tauri build：NSIS/MSI 安装包（icon + resources 配置）
#   属 Phase 4 打包项，便携 zip 用不到；DirectML.dll 由 ort(directml) 构建时
#   自动拷入 target/release，随包收集

param(
    [string]$Configuration = "release",
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot          # 仓库根
$frontendDir = Join-Path $root "crates/photo-tauri"
$targetDir = Join-Path $root "target/$Configuration"
$exeName = "ftpt.exe"
$stageDir = Join-Path $root "dist/ftpt"
$configFile = Join-Path $frontendDir "src-tauri/tauri.conf.json"

# 版本号：默认取 tauri.conf.json 的 version，可用 -Version 覆盖
if ([string]::IsNullOrEmpty($Version)) {
    $Version = (Get-Content $configFile -Raw | ConvertFrom-Json).version
}
$zipPath = Join-Path $root "dist/ftpt-$Version-windows-x64.zip"

# 0. 前置检查
foreach ($cmd in @("cargo", "npm")) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        throw "缺少命令：$cmd（请安装 Rust 工具链 / Node.js）"
    }
}

# 1. 前端构建（tauri-build 编译期嵌入 frontendDist，必须先于 cargo build）
Write-Host "==> npm run build（$frontendDir）"
Push-Location $frontendDir
try {
    npm run build
    if ($LASTEXITCODE -ne 0) { throw "前端构建失败" }
} finally {
    Pop-Location
}

# 2. Rust release 构建（workspace 统一 target）
#    必须带 --features custom-protocol：tauri 据此嵌入 frontendDist（生产语义），
#    否则编译出 dev 模式 exe，运行时加载 devUrl（localhost:1420）→ 打包后白屏/拒绝连接
Write-Host "==> cargo build --$Configuration -p photo-tauri --features custom-protocol"
cargo build --$Configuration -p photo-tauri --features custom-protocol --manifest-path (Join-Path $root "Cargo.toml")
if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }

# 3. 资产检查（缺失即失败，不打残缺包）
$exe = Join-Path $targetDir $exeName
$modelsDir = Join-Path $root "models"
$catalogDb = Join-Path $root "data/pica_ref.db"
foreach ($required in @($exe, $modelsDir, $catalogDb)) {
    if (-not (Test-Path $required)) {
        throw "缺少发布资产：$required（模型放 models/，名录库放 data/pica_ref.db）"
    }
}

# 4. 收集到暂存目录
if (Test-Path $stageDir) { Remove-Item -Recurse -Force $stageDir }
New-Item -ItemType Directory -Force -Path "$stageDir/models", "$stageDir/data" | Out-Null

Copy-Item $exe "$stageDir/$exeName"
# target/release 根目录的运行时 DLL（当前为 DirectML.dll，ort(directml) 构建时自动拷入；
# onnxruntime 已静态链接进 exe）。排除 photo_tauri_lib.dll：crate-type 含 cdylib 的
# 编译副产物，bin 经 rlib 静态链接，非运行时依赖。
# 注意：勿用 -Filter + -Exclude 组合（provider 层反向匹配会把结果清空），用 Where-Object
Get-ChildItem $targetDir -Filter "*.dll" | Where-Object { $_.Name -ne "photo_tauri_lib.dll" } | Copy-Item -Destination $stageDir
Copy-Item "$modelsDir/*.onnx" "$stageDir/models/"
Copy-Item $catalogDb "$stageDir/data/pica_ref.db"

# EXIF 后端：ExifTool 本地运行时（local-lib/exiftool，跨平台各自打包对应平台目录）
# 运行时定位优先级：exe 同级 exiftool/ → local-lib/exiftool/ → PATH（见 docs/exiftool-update.md）
$exifToolDir = Join-Path $root "local-lib/exiftool"
if (Test-Path $exifToolDir) {
    Copy-Item $exifToolDir "$stageDir/exiftool" -Recurse -Force
}

# 5. 打 zip
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
Compress-Archive -Path $stageDir -DestinationPath $zipPath

# 6. 汇总
$zipSize = "{0:N1} MB" -f ((Get-Item $zipPath).Length / 1MB)
Write-Host ""
Write-Host "==> 完成：$zipPath（$zipSize）"
Get-ChildItem -Recurse $stageDir | ForEach-Object { Write-Host "   $($_.FullName.Substring($stageDir.Length + 1))" }
