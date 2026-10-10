param([string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64')
$ErrorActionPreference = 'Stop'
$testDir = Join-Path ([IO.Path]::GetTempPath()) ('ov-receipt-zoom-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testDir | Out-Null
$component = Join-Path $testDir 'ReceiptImageView.qml'
$test = Join-Path $testDir 'tst_receipt_zoom.qml'
$picture = Join-Path $testDir 'image.png'
$results = Join-Path $testDir 'results.txt'
try {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../src/bin/ReceiptImageView.qml') -Destination $component
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'tst_receipt_zoom.qml') -Destination $test
    Add-Type -AssemblyName System.Drawing
    $bitmap = [Drawing.Bitmap]::new(400,400)
    try { $bitmap.Save($picture,[Drawing.Imaging.ImageFormat]::Png) } finally { $bitmap.Dispose() }
    $env:PATH = "$QtRoot/bin;$env:PATH"
    $env:QT_QPA_FONTDIR = 'C:/Windows/Fonts'
    $env:QT_QUICK_CONTROLS_STYLE = 'Fusion'
    $runner = Start-Process -FilePath "$QtRoot/bin/qmltestrunner.exe" -ArgumentList @('-platform','offscreen','-input',('"'+$test+'"'),'-o',('"'+$results+',txt"')) -WindowStyle Hidden -PassThru -Wait
    $testExit = $runner.ExitCode
    Get-Content -LiteralPath $results
} finally {
    Remove-Item -LiteralPath $component,$test,$picture,$results -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $testDir -ErrorAction SilentlyContinue
}
exit $testExit
