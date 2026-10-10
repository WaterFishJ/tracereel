# Prepare pinned CPU ONNX runtime and Paddle models. No Python installation.
param([string]$Destination = '')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $Destination) { $Destination = Join-Path $projectRoot 'tools/ocr' }
$cache = Join-Path $projectRoot 'target/ocr-downloads'
New-Item -ItemType Directory -Force $cache, $Destination, "$Destination/models" | Out-Null
function Get-VerifiedArchive($Url, $Name, $Hash) {
    $path = Join-Path $cache $Name
    if (-not (Test-Path -LiteralPath $path) -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $Hash) {
        Invoke-WebRequest -Uri $Url -OutFile $path
    }
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $Hash) { throw "Checksum mismatch: $Name" }
    return $path
}
$runtime = Get-VerifiedArchive 'https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-win-x64-1.22.0.zip' 'onnxruntime-1.22.0.zip' '174C616EFC0271194488642A72F1A514E01487DA4DFE84C49296D66E40EBE0DA'
$wheel = Get-VerifiedArchive 'https://files.pythonhosted.org/packages/ba/12/1e5497183bdbe782dbb91bad1d0d2297dba4d2831b2652657f7517bfc6df/rapidocr_onnxruntime-1.4.4-py3-none-any.whl' 'rapidocr-models-1.4.4.zip' '971D7D5F223A7A808662229DF1EF69893809D8457D834E6373D3854BC1782CBF'
Add-Type -AssemblyName System.IO.Compression.FileSystem
function Copy-ArchiveFile($Archive, $EntryName, $Target) {
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $entry = $zip.GetEntry($EntryName)
        if (-not $entry) { throw "Archive entry missing: $EntryName" }
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $Target, $true)
    } finally { $zip.Dispose() }
}
foreach ($file in @('onnxruntime.dll', 'onnxruntime_providers_shared.dll')) {
    Copy-ArchiveFile $runtime "onnxruntime-win-x64-1.22.0/lib/$file" "$Destination/$file"
}
Copy-ArchiveFile $runtime 'onnxruntime-win-x64-1.22.0/LICENSE' "$Destination/ONNX-Runtime-LICENSE.txt"
Copy-ArchiveFile $runtime 'onnxruntime-win-x64-1.22.0/ThirdPartyNotices.txt' "$Destination/ONNX-Runtime-ThirdPartyNotices.txt"
foreach ($file in @('ch_PP-OCRv4_det_infer.onnx', 'ch_PP-OCRv4_rec_infer.onnx', 'ch_ppocr_mobile_v2.0_cls_infer.onnx')) {
    Copy-ArchiveFile $wheel "rapidocr_onnxruntime/models/$file" "$Destination/models/$file"
}
Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'licenses') -Filter '*.txt' -File | Copy-Item -Destination $Destination
Write-Host "Rust OCR assets ready: $Destination"