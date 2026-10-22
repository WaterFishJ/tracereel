param(
    [string]$Version = '',
    [string]$PackageDir = '',
    [string]$OutputDir = '',
    [switch]$SkipAssets,
    [switch]$KeepStaging,
    [string]$SignThumbprint = ''
)

# Build the Microsoft Store (MSIX) package from an existing portable package folder.
# The package is left unsigned by default: the Store re-signs it after certification.
# Local install testing can pass -SignThumbprint (a self-signed code signing cert).
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $OutputDir) { $OutputDir = Join-Path $Root 'dist' }
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

if (-not $Version) {
    $metadataJson = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $Root 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read Cargo metadata.' }
    $Version = ($metadataJson | ConvertFrom-Json).packages |
        Where-Object name -eq 'tracereel' |
        Select-Object -First 1 -ExpandProperty version
}
if (-not $Version) { throw 'Missing version.' }
$baseVersion = ($Version -split '[-+]')[0]
if ($baseVersion -notmatch '^\d+\.\d+\.\d+$') { throw "Unsupported version: $Version" }
$msixVersion = "$baseVersion.0"

if (-not $PackageDir) {
    $candidates = @(Get-ChildItem (Join-Path $Root 'dist') -Directory -Filter 'TraceReel-*-windows-x64-*' -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -notlike '*-portable' } |
        Sort-Object LastWriteTime -Descending)
    if ($candidates.Count -eq 0) {
        throw 'No portable package folder found under dist; run package.ps1 first.'
    }
    $PackageDir = $candidates[0].FullName
}
if (-not (Test-Path -LiteralPath $PackageDir)) { throw "Package folder not found: $PackageDir" }

function Find-SdkTool([string]$Name) {
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    if (Test-Path -LiteralPath $kits) {
        $found = Get-ChildItem $kits -Directory |
            Sort-Object Name -Descending |
            ForEach-Object {
                $candidate = Join-Path $_.FullName "x64\$Name"
                if (Test-Path -LiteralPath $candidate) { $candidate }
            } |
            Select-Object -First 1
        if ($found) { return $found }
    }
    throw "Windows SDK tool not found: $Name"
}

$Staging = Join-Path $PSScriptRoot 'staging'
if (Test-Path -LiteralPath $Staging) { Remove-Item -LiteralPath $Staging -Recurse -Force }
New-Item -ItemType Directory -Force -Path $Staging | Out-Null
Write-Host "[1/4] Staging payload..."
Copy-Item -Path (Join-Path $PackageDir '*') -Destination $Staging -Recurse -Force

# MSIX part names must be ANSI; rename the two Chinese-named files shipped by package.ps1.
$renames = @{
    '使用说明.txt' = 'README.zh-CN.txt'
    'licenses\第三方组件.txt' = 'licenses\THIRD-PARTY-COMPONENTS.txt'
}
foreach ($source in $renames.Keys) {
    $from = Join-Path $Staging $source
    if (Test-Path -LiteralPath $from) {
        $to = Join-Path $Staging $renames[$source]
        Move-Item -LiteralPath $from -Destination $to -Force
        Write-Host "  renamed: $source -> $($renames[$source])"
    }
}
$qmlTooling = Join-Path $Staging 'qmltooling'
if (Test-Path -LiteralPath $qmlTooling) {
    Remove-Item -LiteralPath $qmlTooling -Recurse -Force
    Write-Host '  removed: qmltooling (QML debug plugins)'
}

Write-Host "[2/4] Manifest and assets..."
if (-not $SkipAssets) { & (Join-Path $PSScriptRoot 'make-assets.ps1') }
$assets = Join-Path $PSScriptRoot 'Assets'
if (-not (Test-Path -LiteralPath (Join-Path $assets 'StoreLogo.png'))) {
    throw 'Assets are missing; run make-assets.ps1.'
}
Copy-Item -LiteralPath $assets -Destination $Staging -Recurse -Force
$manifest = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'AppxManifest.xml'), [Text.UTF8Encoding]::new($false))
$manifest = $manifest.Replace('{{VERSION}}', $msixVersion)
[IO.File]::WriteAllText((Join-Path $Staging 'AppxManifest.xml'), $manifest, (New-Object Text.UTF8Encoding($false)))

Write-Host "[3/4] resources.pri..."
$makepri = Find-SdkTool 'makepri.exe'
$priConfig = Join-Path $Staging 'priconfig.xml'
& $makepri createconfig /cf $priConfig /dq zh-CN | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makepri createconfig failed.' }
& $makepri new /pr (Join-Path $Staging 'Assets') /cf $priConfig /mn (Join-Path $Staging 'AppxManifest.xml') /of (Join-Path $Staging 'resources.pri') /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'makepri new failed.' }
Remove-Item -LiteralPath $priConfig -Force -ErrorAction SilentlyContinue

Write-Host "[4/4] Packing .msix..."
$makeappx = Find-SdkTool 'makeappx.exe'
$Msix = Join-Path $OutputDir "TraceReel-$baseVersion-windows-x64.msix"
& $makeappx pack /d $Staging /p $Msix /o
if ($LASTEXITCODE -ne 0) { throw 'makeappx pack failed.' }

if ($SignThumbprint) {
    $signtool = Find-SdkTool 'signtool.exe'
    & $signtool sign /sha1 $SignThumbprint /fd SHA256 $Msix
    if ($LASTEXITCODE -ne 0) { throw 'signtool sign failed.' }
    Write-Host '  signed for local testing only (the Store re-signs submissions).'
}

if (-not $KeepStaging) { Remove-Item -LiteralPath $Staging -Recurse -Force }
. (Join-Path $Root 'packaging/file-hash.ps1')
$Hash = Get-PackageFileSha256 -LiteralPath $Msix
"$Hash  $([IO.Path]::GetFileName($Msix))" | Set-Content -LiteralPath "$Msix.sha256" -Encoding ascii
Write-Host "MSIX: $Msix"
Write-Host "SHA256: $Hash"
