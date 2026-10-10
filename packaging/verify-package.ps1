param([Parameter(Mandatory)][string]$Zip)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$TestDir = Join-Path $Root ('target/package-smoke/' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $TestDir -Force | Out-Null
Expand-Archive -LiteralPath $Zip -DestinationPath $TestDir
$Package = (Get-ChildItem -LiteralPath $TestDir -Directory | Select-Object -First 1).FullName

# No development Qt, FFmpeg, or compiler directories in the child's environment.
$env:PATH = "$env:SystemRoot/System32;$env:SystemRoot"
foreach ($key in @('QML_IMPORT_PATH','QML2_IMPORT_PATH','QT_PLUGIN_PATH','QT_QPA_PLATFORM_PLUGIN_PATH','QT_QUICK_CONTROLS_STYLE','FFMPEG_DIR','QT_INCLUDE_PATH','QT_LIBRARY_PATH')) {
    [Environment]::SetEnvironmentVariable($key, $null, 'Process')
}
$env:QT_FORCE_STDERR_LOGGING = '1'
$env:QT_LOGGING_TO_CONSOLE = '1'
$env:QML_IMPORT_TRACE = '1'
$env:RUST_LOG = 'info'
$Stdout = Join-Path $TestDir 'stdout.log'
$Stderr = Join-Path $TestDir 'stderr.log'
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class PackageWindows {
    public delegate bool Callback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(Callback cb, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr window, StringBuilder text, int max);
    public static bool HasMainWindow(uint targetPid) {
        bool found = false;
        EnumWindows((window, data) => {
            uint pid; GetWindowThreadProcessId(window, out pid);
            if (pid == targetPid) {
                var text = new StringBuilder(256);
                GetWindowTextW(window, text, text.Capacity);
                if (text.ToString() == "TraceReel · 迹映") found = true;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
$Process = Start-Process -FilePath "$Package/TraceReel.exe" -WorkingDirectory $TestDir -WindowStyle Hidden -RedirectStandardOutput $Stdout -RedirectStandardError $Stderr -PassThru
try {
    Start-Sleep -Seconds 12
    $Process.Refresh()
    if ($Process.HasExited) { throw "GUI exited: $($Process.ExitCode). See $Stderr" }
    if (-not [PackageWindows]::HasMainWindow([uint32]$Process.Id)) {
        throw "Main window failed to open: $($Process.MainWindowTitle). See $Stderr"
    }
    $Dependencies = $Process.Modules | Where-Object { $_.ModuleName -match '^(Qt6|avcodec|avformat|avutil|swscale|msvcp140|vcruntime140)' }
    $External = $Dependencies | Where-Object { -not $_.FileName.StartsWith($Package, [StringComparison]::OrdinalIgnoreCase) }
    if ($External) { throw "Dependencies loaded outside package: $($External.FileName -join ', ')" }
    $Dependencies | Select-Object ModuleName,FileName | ConvertTo-Json | Set-Content "$TestDir/loaded-dependencies.json"
    $Log = Get-Content -LiteralPath $Stderr -Raw
    if ($Log -match 'QQmlApplicationEngine failed|is not installed|is not a type|gui.qml not found|QQmlComponent: Component is not ready|does not support customization') {
        throw "QML startup failed. See $Stderr"
    }
    if (-not $Log.Contains("$($Package.Replace('\','/'))/qml/app/gui.qml")) {
        throw "Bundled QML not seen in import trace. See $Stderr"
    }
    Write-Output "GUI PASS: $($Process.MainWindowTitle), $($Dependencies.Count) dependencies loaded from extracted ZIP."
} finally {
    if (-not $Process.HasExited) {
        $null = $Process.CloseMainWindow()
        if (-not $Process.WaitForExit(3000)) { Stop-Process -Id $Process.Id -Force }
    }
}

& "$Package/ffmpeg.exe" -hide_banner -loglevel error -f lavfi -i 'color=c=blue:s=160x90:r=10' -t 0.5 -c:v libx264 -pix_fmt yuv420p "$TestDir/encode-test.mp4"
if ($LASTEXITCODE -ne 0 -or -not (Test-Path "$TestDir/encode-test.mp4")) { throw 'FFmpeg encoding failed.' }
& "$Package/ffmpeg.exe" -hide_banner -loglevel error -i "$TestDir/encode-test.mp4" -f null -
if ($LASTEXITCODE -ne 0) { throw 'FFmpeg decoding failed.' }
Write-Output 'FFmpeg PASS: encoded and decoded a synthetic H.264 video.'
Write-Output "Evidence: $TestDir"
