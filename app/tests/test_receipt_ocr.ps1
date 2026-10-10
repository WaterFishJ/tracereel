param(
    [Parameter(Mandatory=$true)][string]$SampleImage,
    [switch]$SkipBuild,
    [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64'
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin qmlprobe
    if ($LASTEXITCODE -ne 0) { throw 'qmlprobe build failed' }
}
$env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
$env:QT_QPA_PLATFORM = 'offscreen'
$env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
$env:QT_FORCE_STDERR_LOGGING = '1'
$env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'receipt_ocr_harness.qml'
$env:QMLPROBE_RUN_LOOP = '1'
$env:QMLPROBE_TIMEOUT_MS = '35000'
$env:OCR_TEST_IMAGE = ([uri][IO.Path]::GetFullPath($SampleImage)).AbsoluteUri
$logs = & (Join-Path $projectRoot 'target/debug/qmlprobe.exe') 2>&1
$code = $LASTEXITCODE
$logs | Write-Output
$output = $logs -join "`n"
if ($code -ne 0 -or $output -notmatch 'RECEIPT_TEST_PASS' -or $output -match 'RECEIPT_TEST_FAIL') {
    throw 'Receipt OCR end-to-end test failed'
}
