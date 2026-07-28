# 发布打包：构建 release → 收集 exe + DLL + 模型 + 名录库 → zip
#
# 用法：pwsh scripts/package.ps1 [-Configuration release] [-Version 0.1.0]
#
# 产物：dist/photo-tool-<Version>-windows-x64.zip，解压即用（便携模式：
# 配置 PT.db/PT.toml 与各照片文件夹的 .pt/ 首运时自建，不进包）。
#
# 注意：本脚本是仓库「无构建脚本」约定的唯一有意例外（仅发布打包，
# 构建/测试仍纯 cargo），见 docs/adr/0003-recognition-subsystem.md。

param(
    [string]$Configuration = "release",
    [string]$Version = "0.1.0"
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot          # 仓库根（worktree 根）
$targetDir = Join-Path $root "target/$Configuration"
$stageDir = Join-Path $root "dist/photo-tool"
$zipPath = Join-Path $root "dist/photo-tool-$Version-windows-x64.zip"

# 1. 构建（ort 静态链接进 exe；copy-dylibs 自动把 DirectML.dll 拷到 targetDir）
Write-Host "==> cargo build --$Configuration -p photo-tool-app"
cargo build --$Configuration -p photo-tool-app --manifest-path (Join-Path $root "Cargo.toml")
if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }

# 2. 资产检查（缺失即失败，不打残缺包）
$exe = Join-Path $targetDir "photo-tool-app.exe"
$modelsDir = Join-Path $root "models"
$catalogDb = Join-Path $root "data/pica_ref.db"
foreach ($required in @($exe, $modelsDir, $catalogDb)) {
    if (-not (Test-Path $required)) {
        throw "缺少发布资产：$required（模型放 models/，名录库放 data/pica_ref.db）"
    }
}

# 3. 收集到暂存目录
if (Test-Path $stageDir) { Remove-Item -Recurse -Force $stageDir }
New-Item -ItemType Directory -Force -Path "$stageDir/models", "$stageDir/data" | Out-Null

Copy-Item $exe "$stageDir/photo-tool.exe"
# target 根目录的非 proc-macro DLL（当前为 DirectML.dll；onnxruntime 已静态链接进 exe）
Get-ChildItem $targetDir -Filter "*.dll" | Copy-Item -Destination $stageDir
Copy-Item "$modelsDir/*.onnx" "$stageDir/models/"
Copy-Item $catalogDb "$stageDir/data/pica_ref.db"

# 4. 打 zip
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
Compress-Archive -Path $stageDir -DestinationPath $zipPath

# 5. 汇总
$zipSize = "{0:N1} MB" -f ((Get-Item $zipPath).Length / 1MB)
Write-Host ""
Write-Host "==> 完成：$zipPath（$zipSize）"
Get-ChildItem -Recurse $stageDir | ForEach-Object { Write-Host "   $($_.FullName.Substring($stageDir.Length + 1))" }
