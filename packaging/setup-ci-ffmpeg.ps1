param([string]$Destination = '')
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
if (-not $Destination) { $Destination = Join-Path $Root 'tools/ffmpeg' }

# Immutable upstream build with matching FFmpeg 8.x ABI and software AV1 support.
$Url = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-10-13-04/ffmpeg-n8.1.3-17-gc389840894-win64-gpl-shared-8.1.zip'
$ExpectedHash = 'FDDEE8B730220AB6973C1CD1DE5A90C4F349BCBA50F906798E25C2AFC0A5D07E'
$Marker = Join-Path $Destination 'tracereel-ci-build.sha256'
if (-not (Test-Path $Marker) -or (Get-Content $Marker -Raw).Trim() -ne $ExpectedHash) {
    $DownloadDir = Join-Path $Root ('target/ffmpeg-downloads/' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $DownloadDir -Force | Out-Null
    $Archive = Join-Path $DownloadDir 'ffmpeg.zip'
    Invoke-WebRequest -Uri $Url -OutFile $Archive
    . (Join-Path $PSScriptRoot 'file-hash.ps1')
    if ((Get-PackageFileSha256 -LiteralPath $Archive) -ne $ExpectedHash) {
        throw 'Downloaded FFmpeg archive failed SHA256 verification.'
    }
    Expand-Archive -LiteralPath $Archive -DestinationPath "$DownloadDir/extracted"
    $Source = (Get-ChildItem "$DownloadDir/extracted" -Directory | Select-Object -First 1).FullName
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    foreach ($Name in @('bin', 'include', 'lib', 'LICENSE.txt')) {
        Copy-Item -LiteralPath (Join-Path $Source $Name) -Destination $Destination -Recurse -Force
    }
    $ExpectedHash | Set-Content -LiteralPath $Marker -Encoding ascii
}
foreach ($File in @('bin/ffmpeg.exe', 'bin/avcodec-62.dll', 'include/libavcodec/avcodec.h', 'lib/avcodec.lib', 'LICENSE.txt')) {
    if (-not (Test-Path (Join-Path $Destination $File))) { throw "Missing FFmpeg component: $File" }
}
$Encoders = & "$Destination/bin/ffmpeg.exe" -hide_banner -encoders 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $Encoders -notmatch 'libsvtav1|libaom-av1|librav1e') {
    throw 'FFmpeg has no working software AV1 encoder.'
}
if ($env:GITHUB_ENV) { "TRACEREEL_FFMPEG_DIR=$Destination" >> $env:GITHUB_ENV }
Write-Host "FFmpeg development and runtime files ready: $Destination"
