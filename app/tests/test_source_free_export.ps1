# Export a static map on the GPX clock, without a source video, with HUD on/off.
param([switch]$SkipBuild, [string]$QtRoot = 'C:\Qt\6.8.2\msvc2022_64')
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $SkipBuild) {
    & (Join-Path $projectRoot 'build.ps1') build -p tracereel --bin qmlprobe
    if ($LASTEXITCODE -ne 0) { throw 'qmlprobe build failed' }
}
$artifacts = Join-Path $projectRoot 'target/source-free-export'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
$names = @('PATH', 'QT_QPA_PLATFORM', 'QMLPROBE_FILE', 'QMLPROBE_ORV',
    'QMLPROBE_RUN_LOOP', 'QMLPROBE_TIMEOUT_MS', 'QMLPROBE_SCREENSHOT', 'QMLPROBE_LOAD_ONLY')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $env:PATH = "$QtRoot/bin;$projectRoot/tools/ffmpeg/bin;$env:PATH"
    $env:QT_QPA_PLATFORM = 'offscreen'
    $env:QMLPROBE_FILE = Join-Path $PSScriptRoot 'source_free_export_probe.qml'
    $env:QMLPROBE_RUN_LOOP = '1'
    $env:QMLPROBE_TIMEOUT_MS = '60000'
    Remove-Item Env:QMLPROBE_SCREENSHOT, Env:QMLPROBE_LOAD_ONLY -ErrorAction SilentlyContinue
    $map = (Join-Path $artifacts 'map.png') -replace '\\', '/'
    $gpx = (Join-Path $artifacts 'track.gpx') -replace '\\', '/'
    & "$projectRoot/tools/ffmpeg/bin/ffmpeg.exe" -y -loglevel error -f lavfi -i 'color=c=0xdc3c28:s=320x240' -frames:v 1 $map
    if ($LASTEXITCODE -ne 0) { throw 'Map generation failed' }
    [IO.File]::WriteAllText($gpx, '<gpx version="1.1"><trk><trkseg><trkpt lat="30" lon="119"><time>2026-01-01T00:00:00Z</time></trkpt><trkpt lat="30.0001" lon="119.0001"><time>2026-01-01T00:00:02Z</time></trkpt></trkseg></trk></gpx>')
    foreach ($hud in @('true', 'false')) {
        $project = Join-Path $artifacts "hud-$hud.trr"
        # Keep includeVideo=true: absent sources must select the map/GPX timeline automatically.
        [IO.File]::WriteAllText($project, "[map]`nimageFilePath=$map`n[hud]`ngpxFilePath=$gpx`n[encode]`nincludeVideo=true`nincludeMap=true`nincludeHud=$hud`nresolution=320x240`nframeRate=24`nencoder=libx264`n[gyro]`nenabled=false`n")
        $env:QMLPROBE_ORV = $project
        $logs = & "$projectRoot/target/debug/qmlprobe.exe" 2>&1
        $code = $LASTEXITCODE
        $logs | Set-Content (Join-Path $artifacts "hud-$hud.log")
        $output = $logs -join "`n"
        if ($code -ne 0 -or $output -notmatch 'SOURCE_FREE_EXPORT_PASS:' -or $output -match 'SOURCE_FREE_EXPORT_FAIL:') {
            throw "Source-free export failed (HUD=$hud); see $artifacts"
        }
        $video = $project -replace '\.trr$', '-export.mp4'
        $duration = & "$projectRoot/tools/ffmpeg/bin/ffprobe.exe" -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 $video
        if ($LASTEXITCODE -ne 0) { throw 'Cannot read exported duration' }
        $seconds = [double]::Parse(($duration | Out-String).Trim(), [Globalization.CultureInfo]::InvariantCulture)
        if ([math]::Abs($seconds - 2.0) -gt 0.15) { throw "Expected 2s export, got $seconds (HUD=$hud)" }
        Write-Output "PASS: source-free export (HUD=$hud), duration=$seconds"
    }
} finally {
    foreach ($name in $names) {
        if ($null -eq $previous[$name]) { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
    }
}
