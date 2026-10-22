param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') { throw 'Invalid version.' }
$Root = Split-Path -Parent $PSScriptRoot
$Dist = Join-Path $Root 'dist'
$Stage = Join-Path $Root ('target/source-archive/' + [guid]::NewGuid().ToString('N'))
$Name = "TraceReel-$Version-source"
New-Item -ItemType Directory -Path $Dist, $Stage -Force | Out-Null
$MainRevision = & git -C $Root rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot read main revision.' }
$GyroRevision = & git -C "$Root/gyroflow-src" rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot read Gyroflow revision.' }
$RecordedGyro = & git -C $Root rev-parse HEAD:gyroflow-src
if ($LASTEXITCODE -ne 0 -or $RecordedGyro -ne $GyroRevision) { throw 'Gyroflow checkout does not match the main repository commit.' }
& git -C $Root archive --format=zip "--prefix=$Name/" "--output=$Stage/main.zip" HEAD
if ($LASTEXITCODE -ne 0) { throw 'Main source archive failed.' }
& git -C "$Root/gyroflow-src" archive --format=zip "--prefix=$Name/gyroflow-src/" "--output=$Stage/gyroflow.zip" HEAD
if ($LASTEXITCODE -ne 0) { throw 'Gyroflow source archive failed.' }
Expand-Archive -LiteralPath "$Stage/main.zip" -DestinationPath $Stage
Expand-Archive -LiteralPath "$Stage/gyroflow.zip" -DestinationPath $Stage -Force
@"
TraceReel version: $Version
Main repository commit: $MainRevision
Gyroflow submodule commit: $GyroRevision
This archive includes the checked-in source for both repositories. See THIRD-PARTY-SOURCE.md for the source of bundled third-party components.
"@ | Set-Content -LiteralPath "$Stage/$Name/SOURCE-REVISION.txt" -Encoding utf8
$thirdPartySource = @"
# 第三方组件的对应源码

本文件随对应源码包一起发布，用于满足 GPL 对随包二进制组件的源码可获得性要求。

- 主程序：本包内主仓库源码（版本 $Version，提交 $MainRevision）
- gyroflow-core：本包内 gyroflow-src（提交 $GyroRevision）
- Qt 6.8.2（动态链接）：https://download.qt.io/archive/qt/6.8/6.8.2/single/
- FFmpeg n8.1（GPL shared 构建）：源码 https://github.com/FFmpeg/FFmpeg （n8.1 分支），构建脚本 https://github.com/BtbN/FFmpeg-Builds
- ONNX Runtime 1.22.0：https://github.com/microsoft/onnxruntime/releases/tag/v1.22.0
- OCR 模型（PP-OCRv4，RapidOCR 1.4.4 分发）：https://pypi.org/project/rapidocr_onnxruntime/1.4.4/
- 相机镜头库 profiles.cbor.gz：https://github.com/gyroflow/lens_profiles
- Visual C++ 运行库：Microsoft 再分发许可，https://visualstudio.microsoft.com/license-terms/

各组件许可见仓库 THIRD-PARTY.md 与安装目录 licenses/。
"@
$thirdPartySource | Set-Content -LiteralPath "$Stage/$Name/THIRD-PARTY-SOURCE.md" -Encoding utf8
$Zip = Join-Path $Dist "$Name.zip"
Compress-Archive -LiteralPath "$Stage/$Name" -DestinationPath $Zip -CompressionLevel Optimal -Force
. (Join-Path $PSScriptRoot 'file-hash.ps1')
$Hash = Get-PackageFileSha256 -LiteralPath $Zip
"$Hash  $([IO.Path]::GetFileName($Zip))" | Set-Content -LiteralPath "$Zip.sha256" -Encoding ascii
Write-Host "Corresponding source archive: $Zip"
