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
This archive includes the checked-in source for both repositories.
"@ | Set-Content -LiteralPath "$Stage/$Name/SOURCE-REVISION.txt" -Encoding utf8
$Zip = Join-Path $Dist "$Name.zip"
Compress-Archive -LiteralPath "$Stage/$Name" -DestinationPath $Zip -CompressionLevel Optimal -Force
. (Join-Path $PSScriptRoot 'file-hash.ps1')
$Hash = Get-PackageFileSha256 -LiteralPath $Zip
"$Hash  $([IO.Path]::GetFileName($Zip))" | Set-Content -LiteralPath "$Zip.sha256" -Encoding ascii
Write-Host "Corresponding source archive: $Zip"
