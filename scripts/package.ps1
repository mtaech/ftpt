# 发布打包（GPUI 版）：cargo release → 收集 exe + DLL + 模型 + 名录库 → zip
#
# 用法：pwsh scripts/package.ps1 [-Configuration release] [-Version 0.1.0] [-VerifyOnly]
#   -Version 缺省时取根 Cargo.toml 的 [workspace.package].version（单一事实源）
#   -VerifyOnly：只做「资产/版本校验 + 打印将要打进去的清单」，不构建、不拷贝、不打包。
#     存在的理由：打包只能在 Windows 上完成（exe 名、DirectML.dll、bsdtar），
#     但**校验逻辑**是跨平台的——在 Linux/macOS 上跑 -VerifyOnly 就能验证资产齐不齐、
#     VERSION 字段对不对（历史上正是这里漏过 data/taxon/，打出来的包缺名录子集包）。
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
# - 运行时资产：models/*.onnx + data/bird_catalog.db + data/taxon/（名录子集包，
#   缺它识别器会报 ModelLoad；data_root() 定位：PHOTO_DATA_DIR → exe 同级 models/ → 仓库根回退，
#   见 crates/photo-ui/src/state/app_state.rs）
# - 署名与许可：NOTICE + LICENSE + LICENSE-AGPL-3.0.txt 随包发（包内含 AGPL-3.0 的
#   org_det.onnx，所以整个发布包按 AGPL-3.0 分发，AGPL 全文必须随包）
# - DirectML.dll 由 ort(directml) 构建时自动拷入 target 目录，随包收集
#   （onnxruntime 已静态链接进 exe）

param(
    [string]$Configuration = "release",
    [string]$Version = "",
    [switch]$VerifyOnly
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot          # 仓库根
$manifest = Join-Path $root "Cargo.toml"
# 二进制名跟着平台走：Windows 是 ftpt.exe，其它平台是 ftpt。
# （-VerifyOnly 在 Linux 上跑时也能正确检查产物；打包本身仍只在 Windows 上有意义）
# 用 RuntimeInformation 而不是 $IsWindows：后者只在 pwsh 7+ 存在
$isWindowsHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Windows)
$exeName = if ($isWindowsHost) { "ftpt.exe" } else { "ftpt" }
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

# 1. Rust release 构建（-VerifyOnly 跳过：校验不需要产物，除非产物已存在）
if ($VerifyOnly) {
    Write-Host "==> -VerifyOnly：跳过 cargo build / 拷贝 / 打包，只做资产与版本校验"
} else {
    Write-Host "==> cargo build --$Configuration -p photo-ui"
    cargo build --$Configuration -p photo-ui --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }
}

# 2. 资产检查（缺失即失败，不打残缺包）
$exe = Join-Path $targetDir $exeName
$modelsDir = Join-Path $root "models"
$catalogDb = Join-Path $root "data/bird_catalog.db"
$taxonDir = Join-Path $root "data/taxon"
$taxonFiles = @("txt_emb_bioclip-2.npy", "txt_emb_bioclip-2.json", "zh_names.json", "VERSION")
$notice = Join-Path $root "NOTICE"
$agpl = Join-Path $root "LICENSE-AGPL-3.0.txt"
foreach ($required in @($modelsDir, $catalogDb, $taxonDir, $notice, $agpl)) {
    if (-not (Test-Path $required)) {
        throw "缺少发布资产：$required（模型放 models/，名录库 data/bird_catalog.db，名录子集包 data/taxon/，署名 NOTICE）"
    }
}
if (-not (Test-Path $exe)) {
    if ($VerifyOnly) {
        Write-Host "==> 提示：产物尚不存在（$exe）；-VerifyOnly 下允许（未构建）"
    } else {
        throw "缺少构建产物：$exe"
    }
}
$taxonMissing = $taxonFiles | Where-Object { -not (Test-Path (Join-Path $taxonDir $_)) }
if ($taxonMissing) {
    throw "data/taxon/ 缺少文件：$($taxonMissing -join ', ')（用 bioclip_demo/data/build_taxon_pack.py 重建）"
}
# VERSION 是名录子集包的版本卡（schema/dim/labels）；dim 与识别代码常量不符时识别必错，先拦下
$taxonVersion = Get-Content (Join-Path $taxonDir "VERSION") -Raw | ConvertFrom-Json
foreach ($key in @("schema", "dim", "labels")) {
    if ($null -eq $taxonVersion.$key) { throw "data/taxon/VERSION 缺少字段：$key" }
}
Write-Host "==> 名录子集包：schema $($taxonVersion.schema) / dim $($taxonVersion.dim) / labels $($taxonVersion.labels)"
# bird_regions.json 是**可选**的地区过滤资产（省级鸟种分布，build_bird_regions.py 生成）：
# 缺失时识别退化为「不过滤」，识别不受影响；存在则随 data/taxon/ 整体打进包。
if (Test-Path (Join-Path $taxonDir "bird_regions.json")) {
    Write-Host "==> 地区过滤资产：data/taxon/bird_regions.json 已就绪（随包分发）"
} else {
    Write-Host "==> 地区过滤资产：无 bird_regions.json（可选，缺失 = 识别不做地区过滤）"
}

# -VerifyOnly 到这里就够了：不构建、不拷贝、不打包，只把「将要打进去的内容」打出来。
# 这样在 Linux/macOS 上也能验证资产齐不齐、VERSION 字段对不对（打 Windows 包本身仍需 Windows）。
if ($VerifyOnly) {
    Write-Host ""
    Write-Host "==> 校验通过，将要打进去的内容："
    Write-Host "    $exeName（cargo build -p photo-ui 产物）"
    Get-ChildItem $modelsDir -Filter "*.onnx" | ForEach-Object { Write-Host "    models/$($_.Name)" }
    Write-Host "    data/bird_catalog.db"
    Get-ChildItem $taxonDir | ForEach-Object { Write-Host "    data/taxon/$($_.Name)" }
    Write-Host "    NOTICE / LICENSE / LICENSE-AGPL-3.0.txt"
    if (Test-Path (Join-Path $root "local-lib/exiftool")) {
        Write-Host "    exiftool/（ExifTool 运行时，随平台各自目录）"
    }
    Write-Host "    target/$Configuration/*.dll（Windows 上是 DirectML.dll；onnxruntime 已静态链接）"
    Write-Host ""
    Write-Host "==> -VerifyOnly 结束（未构建、未拷贝、未打包）"
    exit 0
}

# 3. 收集到暂存目录
if (Test-Path $stageDir) { Remove-Item -Recurse -Force $stageDir }
New-Item -ItemType Directory -Force -Path "$stageDir/models", "$stageDir/data" | Out-Null

Copy-Item $exe "$stageDir/$exeName"
# target 目录根下的运行时 DLL（当前只有 DirectML.dll；onnxruntime 已静态链接）
Get-ChildItem $targetDir -Filter "*.dll" | Copy-Item -Destination $stageDir
Copy-Item "$modelsDir/*.onnx" "$stageDir/models/"
Copy-Item $catalogDb "$stageDir/data/bird_catalog.db"
# 名录子集包：识别器的文本向量与中文名（走 data_root() 的 data/taxon/）
Copy-Item $taxonDir "$stageDir/data/taxon" -Recurse -Force
# 署名与许可文本（BioCLIP 2 是 MIT、CoL China 是 CC BY，两者都要求保留声明；
# 包内含 AGPL-3.0 的 org_det.onnx，所以 AGPL 全文也必须随包分发）
Copy-Item $notice "$stageDir/NOTICE"
Copy-Item (Join-Path $root "LICENSE") "$stageDir/LICENSE"
Copy-Item (Join-Path $root "LICENSE-AGPL-3.0.txt") "$stageDir/LICENSE-AGPL-3.0.txt"

# EXIF 后端：ExifTool 本地运行时（local-lib/exiftool，跨平台各自打包对应平台目录）
# 运行时定位优先级：exe 同级 exiftool/ → local-lib/exiftool/ → PATH（见 docs/exiftool-update.md）
$exifToolDir = Join-Path $root "local-lib/exiftool"
if (Test-Path $exifToolDir) {
    Copy-Item $exifToolDir "$stageDir/exiftool" -Recurse -Force
}

# 4. 打 zip
# 用系统自带的 bsdtar（Windows 10 1803+ 的 C:\Windows\System32\tar.exe），不用 Compress-Archive：
# 后者对 2GB+ 有已知上限，而且包里的 .onnx/.npy 本就是接近随机的字节，deflate 基本压不动、只在浪费时间。
# 必须确认是 bsdtar：若 PATH 前面是 Git/MSYS 的 GNU tar，`-a` 不认 zip，会静默产出 tar 文件。
$tarVersion = (& tar --version 2>&1 | Select-Object -First 1)
if ($tarVersion -notmatch "bsdtar") { throw "需要 bsdtar（Windows 自带 tar.exe），当前 tar 是：$tarVersion" }
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
# 在 dist 目录里用相对路径调用（避开 tar 对 C:\ 盘符参数的处理差异）
Push-Location (Split-Path -Parent $stageDir)
try {
    & tar -a -c -f (Split-Path -Leaf $zipPath) (Split-Path -Leaf $stageDir)
    if ($LASTEXITCODE -ne 0) { throw "tar 打包失败（需要 Windows 10 1803+ 的 tar.exe）" }
}
finally { Pop-Location }

# 5. 汇总
$zipSize = "{0:N1} MB" -f ((Get-Item $zipPath).Length / 1MB)
Write-Host ""
Write-Host "==> 完成：$zipPath（$zipSize）"
Get-ChildItem -Recurse $stageDir | ForEach-Object { Write-Host "   $($_.FullName.Substring($stageDir.Length + 1))" }
