param(
    [switch]$SkipBuild,
    [string]$QtRoot = $(if ($env:TRACEREEL_QT_ROOT) { $env:TRACEREEL_QT_ROOT } else { 'C:\Qt\6.8.2\msvc2022_64' }),
    # 必须包含**软件 AV1 编码器**（libsvtav1 / libaom-av1）：
    # avbuild 的 gpl-lite 构建没有它们，AV1 硬件不可用时无法回退到 CPU。
    [string]$FFmpegDir = $(if ($env:TRACEREEL_FFMPEG_DIR) { $env:TRACEREEL_FFMPEG_DIR } else { Join-Path $PSScriptRoot 'tools\ffmpeg' }),
    [string]$CrtDir = ''
)

$ErrorActionPreference = 'Stop'
$Root = $PSScriptRoot
$BuildDir = Join-Path $Root 'target/package-build'
$Binary = Join-Path $BuildDir 'release/tracereel-gui.exe'
. (Join-Path $Root 'packaging/file-hash.ps1')
foreach ($tool in @('cargo', 'rustc')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Missing build tool: $tool" }
}
if (-not $CrtDir) {
    $crtCandidates = @(
        Get-ChildItem -Path "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT", "$env:ProgramFiles\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT" -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Parent.Parent.Name -match '^\d+\.\d+\.\d+$' } |
            Sort-Object { [version]$_.Parent.Parent.Name } -Descending
    )
    if (-not $crtCandidates) { throw 'MSVC x64 runtime not found. Pass -CrtDir to specify its folder.' }
    $CrtDir = $crtCandidates[0].FullName
}
foreach ($required in @("$QtRoot/bin/windeployqt.exe", "$FFmpegDir/bin/ffmpeg.exe", "$CrtDir/msvcp140.dll")) {
    if (-not (Test-Path -LiteralPath $required)) { throw "Missing dependency: $required" }
}
if (-not $SkipBuild) {
    Write-Host '[1/3] Building Release...'
    # Older packaging builds were seeded with relocated cache files. Regenerate
    # local crate metadata once, retaining the expensive third-party cache.
    $cacheTag = Join-Path $BuildDir 'CACHEDIR.TAG'
    if ((Test-Path -LiteralPath $BuildDir) -and -not (Test-Path -LiteralPath $cacheTag)) {
        'Signature: 8a477f597d28d172789f06886806bc55' | Set-Content -LiteralPath $cacheTag -Encoding ascii
        & cargo clean --release --manifest-path "$Root/Cargo.toml" --target-dir $BuildDir -p tracereel-core -p tracereel -p gyroflow-core
        if ($LASTEXITCODE -ne 0) { throw 'Could not repair legacy build cache.' }
    }
    $env:TRACEREEL_QT_ROOT = $QtRoot
    $env:TRACEREEL_FFMPEG_DIR = $FFmpegDir
    & (Join-Path $Root 'build.ps1') build --release --locked --target-dir $BuildDir -p tracereel --bin tracereel-gui
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
}
foreach ($required in @($Binary, "$QtRoot/bin/windeployqt.exe", "$FFmpegDir/bin/ffmpeg.exe", "$CrtDir/msvcp140.dll", "$Root/gyroflow-src/resources/camera_presets/profiles.cbor.gz")) {
    if (-not (Test-Path -LiteralPath $required)) { throw "Missing dependency: $required" }
}

# Use a fresh folder for each run; never reuse obsolete deployed libraries.
$metadataJson = & cargo metadata --no-deps --format-version 1 --manifest-path "$Root/Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw 'Could not read the application version.' }
$metadata = $metadataJson | ConvertFrom-Json
$Version = ($metadata.packages | Where-Object { $_.name -eq 'tracereel' } | Select-Object -First 1).version
if (-not $Version) { throw 'TraceReel package version not found.' }
$Stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$Name = "TraceReel-$Version-windows-x64-$Stamp"
$Dist = Join-Path $Root 'dist'
$Package = Join-Path $Dist $Name
Write-Host '[2/3] Collecting runtime dependencies...'
New-Item -ItemType Directory -Path "$Package/qml/app", "$Package/camera_presets", "$Package/presets", "$Package/licenses" -Force | Out-Null
Copy-Item -LiteralPath $Binary -Destination "$Package/TraceReel.exe"
& "$Root/ocr/setup.ps1"
Copy-Item -LiteralPath "$Root/tools/ocr" -Destination "$Package/ocr" -Recurse
Copy-Item -LiteralPath "$Root/ocr/README.md" -Destination "$Package/ocr"
Copy-Item -LiteralPath "$Root/app/src/bin/ReceiptImport.js" -Destination "$Package/qml/app"
Get-ChildItem "$Root/app/src/bin" -Filter '*.qml' -File | Copy-Item -Destination "$Package/qml/app"

$env:PATH = "$QtRoot/bin;$env:PATH"
& "$QtRoot/bin/windeployqt.exe" --release --verbose 0 --qmldir "$Package/qml/app" --no-compiler-runtime --no-translations --no-ffmpeg --dir $Package "$Package/TraceReel.exe"
if ($LASTEXITCODE -ne 0) { throw 'Qt deployment failed.' }
Get-ChildItem "$CrtDir/*.dll" -File | Copy-Item -Destination $Package
Get-ChildItem "$FFmpegDir/bin/*.dll" -File | Copy-Item -Destination $Package
Copy-Item "$FFmpegDir/bin/ffmpeg.exe" -Destination $Package
Copy-Item "$BuildDir/release/stabilize_spirv.dll" -Destination $Package
$RustRoot = & rustc --print sysroot
if ($LASTEXITCODE -ne 0) { throw 'Could not locate the Rust runtime.' }
Get-ChildItem "$RustRoot/bin/std-*.dll" -File | Copy-Item -Destination $Package
Copy-Item "$Root/gyroflow-src/resources/camera_presets/profiles.cbor.gz" -Destination "$Package/camera_presets"
Get-ChildItem "$Root/presets" -Filter '*.gyroflow' -File | ForEach-Object {
    # A camera preset should not point to a folder on the developer's machine.
    $preset = Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
    if ($preset.output) { $preset.output.output_folder = '' }
    $preset | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath (Join-Path "$Package/presets" $_.Name) -Encoding utf8
}
Copy-Item "$Root/LICENSE" -Destination "$Package/licenses/GPL-3.0.txt"
# FFmpeg 构建不同，许可证文件名也不同（avbuild: COPYING.GPLv2.1；BtbN: LICENSE.txt）。
$ffLicense = Get-ChildItem -Path $FFmpegDir -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^(COPYING|LICENSE)' } |
    Select-Object -First 1
if (-not $ffLicense) { throw "FFmpeg license file not found under $FFmpegDir" }
Copy-Item $ffLicense.FullName -Destination "$Package/licenses/FFmpeg-COPYING.txt"
Copy-Item "$Root/packaging/使用说明.txt" -Destination $Package
Copy-Item "$Root/THIRD-PARTY.md" -Destination "$Package/licenses/第三方组件.txt"
@"
[Paths]
Prefix=.
Plugins=.
QmlImports=qml
"@ | Set-Content "$Package/qt.conf" -Encoding ascii

$Zip = Join-Path $Dist "$Name-portable.zip"
Write-Host '[3/3] Creating ZIP...'
Compress-Archive -LiteralPath $Package -DestinationPath $Zip -CompressionLevel Optimal
$Hash = Get-PackageFileSha256 -LiteralPath $Zip
"$Hash  $([IO.Path]::GetFileName($Zip))" | Set-Content "$Zip.sha256" -Encoding ascii
Write-Output "Package: $Package"
Write-Output "ZIP: $Zip"
Write-Output "SHA256: $Hash"
